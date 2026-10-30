//! Platform memory probes. Each returns `None` when the platform cannot answer,
//! never a made-up number.

use crate::budget::SystemMemory;
use crate::pressure::PressureLevel;
use browser_core::ByteSize;

/// Memory attributed to this process, with the distinctions that matter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessMemory {
    /// Resident set size: includes shared pages, so over-counts. Informational.
    pub rss: ByteSize,
    /// Proportional set size (Linux): shared pages divided among sharers.
    pub pss: Option<ByteSize>,
    /// Unique/private bytes (Linux: Private_Clean + Private_Dirty).
    pub private: Option<ByteSize>,
    /// macOS `ri_phys_footprint`: what Activity Monitor calls "Memory"; includes
    /// compressed pages and excludes clean shared file pages.
    pub footprint: Option<ByteSize>,
}

impl ProcessMemory {
    /// The best "how much unique RAM are we costing the system" figure.
    pub fn best_unique(&self) -> ByteSize {
        self.footprint.or(self.pss).or(self.private).unwrap_or(self.rss)
    }
}

pub trait MemoryProbe: Send + Sync {
    fn system(&self) -> Option<SystemMemory>;
    fn process(&self) -> Option<ProcessMemory>;
    /// OS-reported pressure, if the platform provides one.
    fn os_pressure(&self) -> Option<PressureLevel>;
}

pub fn platform_probe() -> Box<dyn MemoryProbe> {
    #[cfg(target_os = "linux")]
    return Box::new(linux::LinuxProbe);
    #[cfg(target_os = "macos")]
    return Box::new(macos::MacProbe);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Box::new(NullProbe);
}

#[allow(dead_code)]
pub struct NullProbe;
impl MemoryProbe for NullProbe {
    fn system(&self) -> Option<SystemMemory> { None }
    fn process(&self) -> Option<ProcessMemory> { None }
    fn os_pressure(&self) -> Option<PressureLevel> { None }
}

/// Parse `Key:   123 kB` lines from /proc-style text.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn kib_field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?.strip_prefix(':')?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_meminfo(text: &str) -> Option<SystemMemory> {
    let total = kib_field(text, "MemTotal")? * 1024;
    // MemAvailable already estimates reclaimable cache; fall back for old kernels.
    let available = match kib_field(text, "MemAvailable") {
        Some(a) => a * 1024,
        None => (kib_field(text, "MemFree")? + kib_field(text, "Cached").unwrap_or(0)) * 1024,
    };
    Some(SystemMemory { total: ByteSize(total), available: ByteSize(available) })
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_smaps_rollup(text: &str) -> Option<ProcessMemory> {
    let k = |key| kib_field(text, key).map(|v| ByteSize(v * 1024));
    Some(ProcessMemory {
        rss: k("Rss")?,
        pss: k("Pss"),
        private: Some(ByteSize((kib_field(text, "Private_Clean").unwrap_or(0) + kib_field(text, "Private_Dirty").unwrap_or(0)) * 1024)),
        footprint: None,
    })
}

/// PSI line: `some avg10=0.00 avg60=0.00 avg300=0.00 total=0`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_psi(text: &str) -> Option<PressureLevel> {
    let line = text.lines().find(|l| l.starts_with("some "))?;
    let avg10: f64 = line.split_whitespace().find_map(|f| f.strip_prefix("avg10="))?.parse().ok()?;
    // Thresholds follow common practice (Android lmkd / systemd-oomd use ~10-60 %).
    Some(if avg10 >= 40.0 {
        PressureLevel::Critical
    } else if avg10 >= 10.0 {
        PressureLevel::Warn
    } else {
        PressureLevel::Normal
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    pub struct LinuxProbe;
    impl MemoryProbe for LinuxProbe {
        fn system(&self) -> Option<SystemMemory> {
            parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
        }
        fn process(&self) -> Option<ProcessMemory> {
            parse_smaps_rollup(&std::fs::read_to_string("/proc/self/smaps_rollup").ok()?)
        }
        fn os_pressure(&self) -> Option<PressureLevel> {
            parse_psi(&std::fs::read_to_string("/proc/pressure/memory").ok()?)
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! macOS probes via `sysctlbyname` and `proc_pid_rusage`.
    //!
    //! * `hw.memsize` — physical RAM.
    //! * `vm.page_free_count`, `vm.page_speculative_count`,
    //!   `vm.page_purgeable_count`, `vm.page_pageable_external_count` — pages the
    //!   VM can hand out without swapping.
    //! * `kern.memorystatus_vm_pressure_level` — 1 normal / 2 warn / 4 critical;
    //!   the same signal that backs `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`.
    //! * `proc_pid_rusage(RUSAGE_INFO_V4).ri_phys_footprint`.
    use super::*;
    use std::ffi::CString;

    fn sysctl_u64(name: &str) -> Option<u64> {
        let cname = CString::new(name).ok()?;
        let mut buf = [0u8; 8];
        let mut len: libc::size_t = buf.len();
        // SAFETY: `buf` is valid for `len` bytes; the name is NUL-terminated.
        let rc = unsafe { libc::sysctlbyname(cname.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
        if rc != 0 {
            return None;
        }
        Some(match len {
            4 => u32::from_ne_bytes(buf[..4].try_into().ok()?) as u64,
            8 => u64::from_ne_bytes(buf),
            _ => return None,
        })
    }

    pub struct MacProbe;
    impl MemoryProbe for MacProbe {
        fn system(&self) -> Option<SystemMemory> {
            let total = sysctl_u64("hw.memsize")?;
            let page = sysctl_u64("vm.pagesize").or_else(|| sysctl_u64("hw.pagesize"))?;
            let pages = ["vm.page_free_count", "vm.page_speculative_count", "vm.page_purgeable_count", "vm.page_pageable_external_count"]
                .iter()
                .map(|n| sysctl_u64(n).unwrap_or(0))
                .sum::<u64>();
            Some(SystemMemory { total: ByteSize(total), available: ByteSize((pages * page).min(total)) })
        }

        fn process(&self) -> Option<ProcessMemory> {
            let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
            // SAFETY: `info` is a valid, writable rusage_info_v4.
            let rc = unsafe {
                libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&mut info as *mut libc::rusage_info_v4).cast())
            };
            if rc != 0 {
                return None;
            }
            Some(ProcessMemory {
                rss: ByteSize(info.ri_resident_size),
                pss: None,
                private: None,
                footprint: Some(ByteSize(info.ri_phys_footprint)),
            })
        }

        fn os_pressure(&self) -> Option<PressureLevel> {
            Some(match sysctl_u64("kern.memorystatus_vm_pressure_level")? {
                4 => PressureLevel::Critical,
                2 => PressureLevel::Warn,
                _ => PressureLevel::Normal,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meminfo_with_and_without_memavailable() {
        let t = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    8000000 kB\nCached:  2000000 kB\n";
        let m = parse_meminfo(t).unwrap();
        assert_eq!(m.total.0, 16_000_000 * 1024);
        assert_eq!(m.available.0, 8_000_000 * 1024);
        let old = "MemTotal: 1000 kB\nMemFree: 100 kB\nCached: 200 kB\n";
        assert_eq!(parse_meminfo(old).unwrap().available.0, 300 * 1024);
        assert!(parse_meminfo("garbage").is_none());
    }

    #[test]
    fn parses_smaps_rollup_and_prefers_unique_figures() {
        let t = "Rss:  1000 kB\nPss:  400 kB\nPrivate_Clean:  100 kB\nPrivate_Dirty:  200 kB\n";
        let p = parse_smaps_rollup(t).unwrap();
        assert_eq!(p.rss.0, 1000 * 1024);
        assert_eq!(p.pss.unwrap().0, 400 * 1024);
        assert_eq!(p.private.unwrap().0, 300 * 1024);
        assert_eq!(p.best_unique().0, 400 * 1024, "PSS beats RSS");
    }

    #[test]
    fn parses_psi() {
        assert_eq!(parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00"), Some(PressureLevel::Normal));
        assert_eq!(parse_psi("some avg10=12.50 avg60=1 avg300=1 total=9"), Some(PressureLevel::Warn));
        assert_eq!(parse_psi("some avg10=55.0 avg60=1 avg300=1 total=9"), Some(PressureLevel::Critical));
        assert_eq!(parse_psi("nonsense"), None);
    }

    #[test]
    fn live_probe_returns_plausible_numbers_on_this_platform() {
        let p = platform_probe();
        if let Some(s) = p.system() {
            assert!(s.total.0 > 0 && s.available.0 <= s.total.0);
        }
        if let Some(pm) = p.process() {
            assert!(pm.rss.0 > 0);
            assert!(pm.best_unique().0 > 0);
        }
    }
}
