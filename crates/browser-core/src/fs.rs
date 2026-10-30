//! Crash-safe file persistence: write to a temp file in the same directory,
//! fsync, then rename over the destination so readers never see a torn file.

use std::io::{self, Write};
use std::path::Path;

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_contents() {
        let dir = std::env::temp_dir().join(format!("browserust-fs-{}", std::process::id()));
        let p = dir.join("a.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert!(!p.with_extension("tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
