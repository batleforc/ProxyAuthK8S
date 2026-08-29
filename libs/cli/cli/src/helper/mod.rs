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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal scratch directory with RAII cleanup, to avoid a dev-dependency.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "proxyauthk8s-cli-helper-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("the scratch directory is creatable");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .expect("the file exists")
            .permissions()
            .mode()
            // Keep the permission bits only, dropping the file-type bits.
            & 0o777
    }

    #[test]
    fn writes_the_contents_it_was_given() {
        let dir = TempDir::new();
        let path = dir.join("config.yaml");

        secure_write(&path, "hello: world\n").expect("the write succeeds");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is readable"),
            "hello: world\n"
        );
    }

    #[test]
    fn truncates_an_existing_file_instead_of_appending() {
        let dir = TempDir::new();
        let path = dir.join("config.yaml");

        secure_write(&path, "first write, the longer one").expect("the first write succeeds");
        secure_write(&path, "second").expect("the second write succeeds");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is readable"),
            "second"
        );
    }

    #[cfg(unix)]
    #[test]
    fn creates_the_file_owner_readable_only() {
        let dir = TempDir::new();
        let path = dir.join("config.yaml");

        secure_write(&path, "token: secret\n").expect("the write succeeds");

        // 0600: no group or other bits, whatever the process umask is.
        assert_eq!(mode_of(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn tightens_an_already_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let path = dir.join("config.yaml");
        std::fs::write(&path, "old").expect("the file is creatable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("the permissions are settable");
        assert_eq!(mode_of(&path), 0o644);

        secure_write(&path, "new").expect("the write succeeds");

        // `mode()` only applies on creation, so this is the explicit
        // `set_permissions` fixing a config that was left world-readable.
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn reports_an_error_when_the_path_is_not_writable() {
        let dir = TempDir::new();
        // The parent directory does not exist, so the open fails.
        let path = dir.join("missing").join("config.yaml");

        assert!(secure_write(&path, "anything").is_err());
    }
}
