use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// An advisory lock file that prevents concurrent dagger operations.
/// The lock is released (file deleted) when the guard is dropped.
pub struct StoreLock {
    lock_path: PathBuf,
}

impl StoreLock {
    /// Acquire an advisory lock by creating a lock file.
    /// Returns an error if another process holds the lock.
    pub fn acquire(dagger_root: &Path) -> io::Result<Self> {
        fs::create_dir_all(dagger_root)?;
        let lock_path = dagger_root.join("lock");

        // Try to create the lock file exclusively.
        // O_CREAT | O_EXCL ensures atomic creation — fails if file already exists.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let _ = writeln!(file, "{}", std::process::id());
                Ok(Self { lock_path })
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::other(format!(
                "another dgr process appears to be running; \
                 if this is incorrect, delete '{}'",
                lock_path.display()
            ))),
            Err(e) => Err(e),
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_creates_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let lock = StoreLock::acquire(dir.path()).unwrap();
        assert!(dir.path().join("lock").exists());
        drop(lock);
    }

    #[test]
    fn drop_removes_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let lock = StoreLock::acquire(dir.path()).unwrap();
        drop(lock);
        assert!(!dir.path().join("lock").exists());
    }

    #[test]
    fn second_acquire_fails_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let _lock = StoreLock::acquire(dir.path()).unwrap();
        let result = StoreLock::acquire(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("another dgr process"));
    }

    #[test]
    fn acquire_succeeds_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let lock = StoreLock::acquire(dir.path()).unwrap();
        drop(lock);
        let _lock2 = StoreLock::acquire(dir.path()).unwrap();
    }
}
