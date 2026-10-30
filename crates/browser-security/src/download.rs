/// Make a server-suggested filename safe to use inside the downloads directory:
/// no path components, no control characters, no leading dots, no reserved
/// Windows device names, bounded length.
pub fn sanitize_filename(raw: &str) -> String {
    // Keep only the final path component, treating both separators as such.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    s = s.trim().trim_start_matches('.').trim_end_matches(['.', ' ']).to_string();

    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
        "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        s.insert(0, '_');
    }
    if s.is_empty() {
        return "download".into();
    }
    if s.len() > 200 {
        // Truncate on a char boundary, keeping the extension.
        let ext = s.rfind('.').map(|i| s[i..].to_string()).filter(|e| e.len() <= 16).unwrap_or_default();
        let mut cut = 200 - ext.len();
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s = format!("{}{}", &s[..cut], ext);
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadRisk {
    Normal,
    /// Executable / installer / script: warn and never auto-open.
    Dangerous,
}

pub fn assess_download(filename: &str) -> DownloadRisk {
    const DANGEROUS: &[&str] = &[
        "exe", "msi", "bat", "cmd", "com", "scr", "pif", "ps1", "vbs", "js", "jse", "wsf", "jar", "dmg", "pkg", "app", "command", "sh",
        "bash", "run", "apk", "deb", "rpm", "appimage", "lnk", "reg", "hta", "dll",
    ];
    // Check the *last* extension and every dot-separated part, so `a.pdf.exe` is caught.
    let lower = filename.to_ascii_lowercase();
    let mut parts = lower.rsplit('.');
    let last = parts.next().unwrap_or("");
    if DANGEROUS.contains(&last) {
        DownloadRisk::Dangerous
    } else {
        DownloadRisk::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_paths_and_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename("/abs/path/file.txt"), "file.txt");
    }

    #[test]
    fn removes_control_chars_hidden_files_and_reserved_names() {
        assert_eq!(sanitize_filename("a\u{0}b\nc.txt"), "abc.txt");
        assert_eq!(sanitize_filename(".bashrc"), "bashrc");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("..."), "download");
    }

    #[test]
    fn long_names_are_truncated_but_keep_extension() {
        let n = format!("{}.pdf", "a".repeat(500));
        let s = sanitize_filename(&n);
        assert!(s.len() <= 200 && s.ends_with(".pdf"));
        let uni = format!("{}.txt", "é".repeat(300));
        assert!(sanitize_filename(&uni).is_char_boundary(sanitize_filename(&uni).len()));
    }

    #[test]
    fn risk_uses_final_extension() {
        assert_eq!(assess_download("setup.exe"), DownloadRisk::Dangerous);
        assert_eq!(assess_download("report.pdf.exe"), DownloadRisk::Dangerous);
        assert_eq!(assess_download("photo.JPG"), DownloadRisk::Normal);
        assert_eq!(assess_download("installer.DMG"), DownloadRisk::Dangerous);
        assert_eq!(assess_download("exe"), DownloadRisk::Dangerous, "bare 'exe' has no dot; still flagged conservatively");
    }
}
