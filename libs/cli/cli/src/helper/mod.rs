use std::io;
use std::path::Path;

/// Write `contents` to `path`, restricting the file to owner read/write (`0600`)
/// on Unix.
///
/// The CLI's config and kubeconfig can hold, or point at, credential material,
/// so they must not be created world-readable (the process umask would
/// otherwise typically yield `0644`). On non-Unix platforms this falls back to a
/// plain write.
pub fn secure_write<P: AsRef<Path>>(path: P, contents: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        // `mode()` only applies when the file is created; tighten an existing
        // file's permissions too so a previously world-readable config is fixed.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}
