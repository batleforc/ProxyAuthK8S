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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "proxyauth-cli-helper-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("file")
    }

    #[test]
    fn secure_write_writes_and_truncates_the_contents() {
        let path = scratch_path("contents");
        secure_write(&path, "a longer first content").unwrap();
        secure_write(&path, "short").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "short");
    }

    #[cfg(unix)]
    #[test]
    fn secure_write_creates_the_file_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch_path("create");
        secure_write(&path, "secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn secure_write_tightens_an_existing_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch_path("tighten");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        secure_write(&path, "new").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }
}
