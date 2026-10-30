//! Process allocator configuration.
//!
//! `servo-allocator` (with `use-jemalloc`) installs jemalloc as the global
//! allocator, which Servo's `malloc_size_of` memory reporting requires. Here we
//! tune it for a memory-constrained desktop: freed pages are returned to the OS
//! by a background thread within seconds instead of being retained, which is
//! what lowers resident memory after a tab is closed or discarded.
//!
//! jemalloc reads this symbol before any allocation. A typo is silently
//! ignored, so [`opt_dirty_decay_ms`] lets tests and the memory panel confirm
//! the setting is live.

#[cfg(not(any(windows, target_env = "ohos")))]
mod imp {
    use std::ffi::c_void;

    /// Prefixed symbol name used by tikv-jemalloc-sys.
    #[unsafe(export_name = "_rjem_malloc_conf")]
    pub static MALLOC_CONF: &[u8] = b"background_thread:true,dirty_decay_ms:3000,muzzy_decay_ms:3000\0";

    unsafe extern "C" {
        #[link_name = "_rjem_mallctl"]
        fn mallctl(name: *const u8, oldp: *mut c_void, oldlenp: *mut usize, newp: *mut c_void, newlen: usize) -> i32;
    }

    fn read<T: Default>(name: &[u8]) -> Option<T> {
        let mut v = T::default();
        let mut len = std::mem::size_of::<T>();
        // SAFETY: `name` is NUL-terminated; `v`/`len` describe a valid buffer.
        let rc = unsafe { mallctl(name.as_ptr(), (&mut v as *mut T).cast(), &mut len, std::ptr::null_mut(), 0) };
        (rc == 0).then_some(v)
    }

    pub fn opt_dirty_decay_ms() -> Option<i64> {
        read::<i64>(b"opt.dirty_decay_ms\0")
    }
    pub fn opt_background_thread() -> Option<bool> {
        read::<bool>(b"opt.background_thread\0")
    }
    /// Bytes currently allocated by the application / mapped by jemalloc.
    pub fn stats() -> Option<(usize, usize)> {
        // Refresh the stats epoch first.
        let mut epoch: u64 = 1;
        let mut len = std::mem::size_of::<u64>();
        unsafe { mallctl(b"epoch\0".as_ptr(), (&mut epoch as *mut u64).cast(), &mut len, (&mut epoch as *mut u64).cast(), len) };
        Some((read::<usize>(b"stats.allocated\0")?, read::<usize>(b"stats.resident\0")?))
    }
}

#[cfg(any(windows, target_env = "ohos"))]
mod imp {
    pub fn opt_dirty_decay_ms() -> Option<i64> { None }
    pub fn opt_background_thread() -> Option<bool> { None }
    pub fn stats() -> Option<(usize, usize)> { None }
}

pub use imp::{opt_background_thread, opt_dirty_decay_ms, stats};
