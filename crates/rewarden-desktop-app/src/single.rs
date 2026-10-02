//! One Reins per user. The first one holds a lock on `app.lock` in the state directory for as long as it runs; a
//! second one leaves `app.show` next to it (the first one shows its window when it sees it) and ends.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

pub struct Instance {
    _lock: Option<File>,
    show_request: PathBuf,
}

impl Instance {
    /// The lock, or `None` when another Reins holds it (it was asked to show its window).
    pub fn acquire(state_dir: &Path) -> Option<Self> {
        if let Err(e) = std::fs::create_dir_all(state_dir) {
            log::warn!("{}: {e}", state_dir.display());
        }
        let show_request = state_dir.join("app.show");
        let lock = match OpenOptions::new().create(true).truncate(false).write(true).open(state_dir.join("app.lock")) {
            Ok(f) => f,
            Err(e) => {
                // Better two of them than none.
                log::warn!("cannot open the instance lock: {e}");
                return Some(Self {
                    _lock: None,
                    show_request,
                });
            }
        };
        match lock.try_lock() {
            Ok(()) => {
                let _stale = std::fs::remove_file(&show_request);
                Some(Self {
                    _lock: Some(lock),
                    show_request,
                })
            }
            Err(TryLockError::WouldBlock) => {
                if let Err(e) = std::fs::write(&show_request, b"") {
                    log::warn!("{}: {e}", show_request.display());
                }
                None
            }
            Err(TryLockError::Error(e)) => {
                log::warn!("cannot lock the instance lock: {e}");
                Some(Self {
                    _lock: Some(lock),
                    show_request,
                })
            }
        }
    }

    /// Whether another start asked for the window since the last call.
    pub fn take_show_request(&self) -> bool {
        std::fs::remove_file(&self.show_request).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_instance_asks_the_first_to_show_itself() {
        let dir = tempfile::tempdir().unwrap();
        let first = Instance::acquire(dir.path()).unwrap();
        assert!(!first.take_show_request());
        assert!(Instance::acquire(dir.path()).is_none());
        assert!(first.take_show_request());
        assert!(!first.take_show_request());
        drop(first);
        assert!(Instance::acquire(dir.path()).is_some());
    }
}
