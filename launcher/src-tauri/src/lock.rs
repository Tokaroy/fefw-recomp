// SPDX-License-Identifier: GPL-3.0-or-later
//! One setup or update at a time in an install folder. The window and a launcher run without one (headless.rs) are separate
//! processes, so they hold launcher.lock there while they work. The system drops the lock when the process ends, however it ends.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

pub const BUSY: &str = "Another setup or update is running for this install. Wait for it to finish.";

/// Held until dropped.
pub struct Lock(#[allow(dead_code)] Option<File>);

/// Err(BUSY) while someone else holds it. A folder or file system that cannot lock is not a reason to refuse: the job goes on
/// without one (and says itself when it cannot write there).
pub fn acquire(install: &Path) -> Result<Lock, String> {
    let Ok(f) = OpenOptions::new().create(true).write(true).truncate(false).open(install.join("launcher.lock")) else {
        return Ok(Lock(None));
    };
    match f.try_lock() {
        Ok(()) => Ok(Lock(Some(f))),
        Err(TryLockError::WouldBlock) => Err(BUSY.into()),
        Err(TryLockError::Error(_)) => Ok(Lock(None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_at_a_time() {
        let d = std::env::temp_dir().join(format!("fefw-lock-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let first = acquire(&d).unwrap();
        assert_eq!(acquire(&d).err().as_deref(), Some(BUSY));
        drop(first);
        assert!(acquire(&d).is_ok());
        assert!(acquire(&d.join("missing")).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
