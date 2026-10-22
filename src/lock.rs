//! Single-instance guard.
//!
//! Two loops probing the same peer is harmless on the wire but confusing in the
//! logs and impossible to stop cleanly — a second `run` must refuse to start.
//! An advisory file lock gives us this without the stale-pidfile problem: the
//! OS drops the lock when the process dies, however it dies.

use anyhow::{Context, Result, bail};
use fs4::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

pub struct InstanceLock {
    file: File,
    path: PathBuf,
}

impl InstanceLock {
    /// Try to take the lock. Fails if another process holds it.
    pub fn acquire(path: &Path) -> Result<InstanceLock> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("failed to open lock file {}", path.display()))?;

        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(fs4::TryLockError::WouldBlock) => {
                bail!(
                    "another instance is already running (lock held on {})",
                    path.display()
                )
            }
            Err(fs4::TryLockError::Error(err)) => {
                return Err(err).with_context(|| format!("failed to lock {}", path.display()));
            }
        }

        // Best effort: record the pid for humans reading the file.
        let _ = std::fs::write(path, format!("{}\n", std::process::id()));

        Ok(InstanceLock {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_on_same_path_fails() {
        let dir = std::env::temp_dir().join(format!("tska-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("instance.lock");

        let first = InstanceLock::acquire(&path).unwrap();
        assert!(InstanceLock::acquire(&path).is_err());

        drop(first);
        assert!(InstanceLock::acquire(&path).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lock_file_is_created_with_parent_dirs() {
        let dir = std::env::temp_dir().join(format!("tska-lock-nested-{}", std::process::id()));
        let path = dir.join("a").join("b").join("instance.lock");
        let lock = InstanceLock::acquire(&path).unwrap();
        assert!(path.exists());
        assert_eq!(lock.path(), path.as_path());
        std::fs::remove_dir_all(&dir).ok();
    }
}
