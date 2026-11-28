//! Replacing a file so that a failure, or a crash, at any point leaves its last version whole:
//! World saves (`crate::saves`) and the player's settings (`crate::settings`) both commit this way.
//!
//! A write stages the whole new file beside the old one under a name of its own ([`stage`]),
//! flushes it to disk, then renames it over the old one, which Linux and Windows both do in one
//! step. Then it flushes the folder ([`sync_dir`]), so the rename survives a power cut.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Writes `bytes` to a new file at `path` and flushes it to disk. Fails if `path` exists.
pub(crate) fn stage(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Flushes the folder's list of files to disk, so a rename into it survives a power cut. Only
/// Unix can open a folder to flush it; Windows' NTFS journals renames itself. Some file systems
/// can't flush a folder at all, so failing is fine: the rename has happened either way.
pub(crate) fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}
