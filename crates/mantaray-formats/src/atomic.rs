//! Saving a file so that a failed save cannot cost the copy already there.
//!
//! `std::fs::write` truncates the target and then writes into it. If the disk
//! fills, the USB stick is pulled or the program dies half-way, what is left
//! is neither the old file nor the new one, and the old one was usually the
//! operator's only copy of a count. [`write_atomic`] writes a neighbouring
//! temporary file instead, makes sure it is on the disk, and only then renames
//! it over the target. A rename within one directory is atomic, so a reader
//! (or the operator, after a crash) sees the whole old file or the whole new
//! one, never a mixture.
//!
//! On Windows the same `std::fs::rename` replaces an existing target: the
//! standard library calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, and
//! if that is refused with access denied it retries with
//! `SetFileInformationByHandle(FileRenameInfoEx)` and
//! `FILE_RENAME_FLAG_REPLACE_IF_EXISTS` (library/std/src/sys/fs/windows.rs,
//! `rename`, Rust 1.97). A target held open without delete sharing makes the
//! rename fail. Where the other program also denies write sharing (a CSV open
//! in Excel) an in-place write fails there too; where it shares write (C's
//! `fopen(..., "r")`, Python's `open`) an in-place write works, so a refused
//! rename falls back to one (see [`write_atomic`]). Untested on Windows: the
//! fallback is tested on Linux with the refusal injected.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `path`, replacing any file there only once the new
/// contents are safely on the disk.
///
/// On success the target holds exactly `bytes` and no temporary file is left
/// behind. On failure the target is untouched (or still absent) and the
/// temporary file has been removed, except where the save fell back to an
/// in-place write (below), which can fail half-way as 0.2.4's did.
///
/// An existing target must be writable, as an in-place save would need it to
/// be: a file the operator marked read-only is refused, not replaced. Its
/// permission bits are carried over to the new file where the filesystem
/// allows; ownership and ACLs other than those bits are not. A symbolic link
/// is followed, so the file it points at is replaced (or made, for a link to
/// a file not there yet) and the link survives.
///
/// The temporary file goes in the target's own directory. Where that
/// directory refuses it (permission denied), or takes it but refuses to let
/// it replace the target (permission denied, or a Windows sharing
/// violation), the file is written in place instead, the way
/// `std::fs::write` does, and the temporary file removed: that save still
/// works where 0.2.4's did, but is not atomic.
///
/// # Errors
///
/// Any I/O error from checking the target, creating, writing or syncing the
/// temporary file, or renaming it into place.
pub fn write_atomic(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let bytes = bytes.as_ref();
    write_atomic_with(path.as_ref(), |file| file.write_all(bytes))
}

/// [`write_atomic`] with the writing done by `fill`, so a test can fail it
/// half-way through.
fn write_atomic_with(path: &Path, fill: impl Fn(&mut File) -> io::Result<()>) -> io::Result<()> {
    write_atomic_via(path, fill, |from, to| fs::rename(from, to))
}

/// Whether a refused rename is the kind an in-place write may still get
/// past: permission denied (Unix EACCES/EPERM, Windows ERROR_ACCESS_DENIED),
/// and on Windows a sharing violation, which std maps to no kind of its own.
fn rename_refused(error: &io::Error) -> bool {
    /// Windows' ERROR_SHARING_VIOLATION.
    #[cfg(windows)]
    const SHARING_VIOLATION: i32 = 32;
    #[cfg(windows)]
    if error.raw_os_error() == Some(SHARING_VIOLATION) {
        return true;
    }
    error.kind() == io::ErrorKind::PermissionDenied
}

/// [`write_atomic_with`] with the rename done by `rename`, so a test can
/// refuse it as a sticky folder or a Windows reader without delete sharing
/// does. `fill` is called a second time when the rename is refused and the
/// file is written in place.
fn write_atomic_via(
    path: &Path,
    fill: impl Fn(&mut File) -> io::Result<()>,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let target = follow_link(path);
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} does not name a file", path.display()),
        )
    })?;
    let directory = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let existing = match fs::metadata(&target) {
        Ok(metadata) => {
            // The same check an in-place write makes when it opens the file,
            // without changing it. Without this a rename would quietly
            // replace a file the operator had made read-only.
            OpenOptions::new().write(true).open(&target)?;
            Some(metadata.permissions())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let (scratch, mut part) = match create_scratch(&directory, name) {
        Ok(created) => created,
        // A folder that may not be written into, holding a file that may:
        // a lab share set up that way saved on 0.2.4, and being unable to
        // save at all is worse than a save that is not atomic. Windows'
        // ERROR_ACCESS_DENIED reads as this kind too
        // (library/std/src/sys/io/error/windows.rs).
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return write_in_place(&target, fill);
        }
        Err(error) => return Err(error),
    };
    let filled = (|| {
        if let Some(permissions) = existing {
            // Best effort: a FAT stick or a network share may refuse to set
            // modes at all, and that must not make the save fail.
            part.set_permissions(permissions).ok();
        }
        fill(&mut part)?;
        part.sync_all()
    })();
    drop(part);
    let written = match filled {
        Ok(()) => rename(&scratch, &target).map_err(Failed::Renaming),
        Err(error) => Err(Failed::Writing(error)),
    };
    match written {
        Ok(()) => {
            sync_directory(&directory);
            Ok(())
        }
        // The `.part` was written but may not replace the target: a Windows
        // reader sharing write but not delete (C's fopen "r", Python's
        // open), a sticky folder and a target owned by someone else, a share
        // granting Write but not Delete. 0.2.4's in-place write got past
        // all three, so the save falls back to it rather than failing a save
        // 0.2.4 made (Lucius, review of 3dfa038). Not atomic there.
        Err(Failed::Renaming(error)) if rename_refused(&error) => {
            discard(&scratch);
            write_in_place(&target, fill)
        }
        Err(Failed::Renaming(error) | Failed::Writing(error)) => {
            discard(&scratch);
            Err(error)
        }
    }
}

/// Which step of an atomic save failed.
enum Failed {
    /// Filling or syncing the `.part`.
    Writing(io::Error),
    /// Renaming it over the target.
    Renaming(io::Error),
}

/// Removes a `.part` that will not be renamed. Best effort: the error worth
/// reporting is the save's. Where the folder refuses deletion too (a share
/// granting Write but not Delete) it is at least emptied, so no second copy
/// of the count is left hidden beside the file.
fn discard(scratch: &Path) {
    if fs::remove_file(scratch).is_err()
        && let Ok(file) = OpenOptions::new().write(true).open(scratch)
    {
        file.set_len(0).ok();
    }
}

/// The old way: truncate the target and write into it. Only for a folder
/// that refuses the temporary file; see [`write_atomic`].
fn write_in_place(target: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let mut out = File::create(target)?;
    fill(&mut out)?;
    out.sync_all()
}

/// The file a symbolic link points at, or `path` itself.
///
/// A link to a file not there yet cannot be canonicalized, and replacing the
/// link's own path put a plain file where the link was (Lucius a4); 0.2.4's
/// in-place write made the file at the link's destination. Such a link is
/// read hop by hop instead (a relative one from its own folder), at most as
/// many hops as Linux follows.
fn follow_link(path: &Path) -> PathBuf {
    /// Linux's MAXSYMLINKS.
    const HOPS: usize = 40;
    let mut at = path.to_path_buf();
    for _ in 0..HOPS {
        match fs::symlink_metadata(&at) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if let Ok(real) = fs::canonicalize(&at) {
                    return real;
                }
                let Ok(next) = fs::read_link(&at) else {
                    return at;
                };
                at = match at.parent() {
                    Some(folder) if next.is_relative() => folder.join(next),
                    _ => next,
                };
            }
            _ => return at,
        }
    }
    at
}

/// A new, empty temporary file beside the target, named so it cannot be
/// mistaken for a finished one and cannot collide with another save.
///
/// The name is the target's with a dot before and `.<pid>-<n>.part` after,
/// unless that is longer than the filesystem allows for a name the target
/// itself fits in (a 250-byte name saved on 0.2.4; Lucius a3): then it is
/// `.mantaray-<pid>-<n>.part`.
fn create_scratch(directory: &Path, name: &std::ffi::OsStr) -> io::Result<(PathBuf, File)> {
    match create_scratch_named(directory, Some(name)) {
        Err(error) if error.kind() == io::ErrorKind::InvalidFilename => {
            create_scratch_named(directory, None)
        }
        other => other,
    }
}

/// [`create_scratch`], named after `name` or, without one, after this program.
fn create_scratch_named(
    directory: &Path,
    name: Option<&std::ffi::OsStr>,
) -> io::Result<(PathBuf, File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut scratch_name = std::ffi::OsString::from(".");
        scratch_name.push(name.unwrap_or_else(|| std::ffi::OsStr::new("mantaray")));
        scratch_name.push(format!(
            ".{}-{}.part",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let scratch = directory.join(scratch_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&scratch)
        {
            Ok(file) => return Ok((scratch, file)),
            // Left by a run that died; the counter moves past it.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

/// Makes the rename itself durable on Unix, where a directory entry is only
/// on the disk once the directory has been synced.
///
/// Best effort: by now the new file is complete and in place, and some
/// filesystems (network ones, FAT) refuse to sync a directory at all. A
/// failure here means only that a power cut in the next moments could undo
/// the rename - leaving the old file, never a broken one. Windows has no
/// equivalent call for a directory; NTFS journals the rename itself.
fn sync_directory(directory: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = File::open(directory) {
        handle.sync_all().ok();
    }
    #[cfg(not(unix))]
    let _ = directory;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory for one test.
    fn scratch_dir(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mantaray-atomic-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// Every name in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_save_that_fails_half_way_leaves_the_old_file_byte_for_byte() {
        let dir = scratch_dir("half-way");
        let path = dir.join("count.Chn");
        let old: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        fs::write(&path, &old).expect("the operator's file");

        let outcome = write_atomic_with(&path, |file| {
            file.write_all(b"the first half of a new spectrum")?;
            Err(io::Error::other("no space left on device"))
        });

        assert!(outcome.is_err(), "the failure must be reported");
        assert_eq!(fs::read(&path).expect("still there"), old);
        assert_eq!(names(&dir), ["count.Chn"], "no temporary file left");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_save_that_works_replaces_the_file_and_leaves_nothing_else() {
        let dir = scratch_dir("works");
        let path = dir.join("report.rpt");
        fs::write(&path, "an older, longer report than the new one").expect("old");

        write_atomic(&path, "new").expect("the save");

        assert_eq!(fs::read_to_string(&path).expect("read"), "new");
        assert_eq!(names(&dir), ["report.rpt"]);
        // And a first save, with nothing there before.
        let fresh = dir.join("fresh.csv");
        write_atomic(&fresh, "a,b\n").expect("a first save");
        assert_eq!(fs::read_to_string(&fresh).expect("read"), "a,b\n");
        assert_eq!(names(&dir), ["fresh.csv", "report.rpt"]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_error_and_creates_nothing() {
        let dir = scratch_dir("no-folder");
        let path = dir.join("missing").join("x.json");
        assert!(write_atomic(&path, "x").is_err());
        assert!(names(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_read_only_file_is_refused_not_replaced() {
        let dir = scratch_dir("read-only");
        let path = dir.join("locked.Spe");
        fs::write(&path, "keep me").expect("old");
        let mut permissions = fs::metadata(&path).expect("meta").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).expect("make it read-only");
        // Root writes through a read-only mode; the check is then moot.
        let enforced = OpenOptions::new().write(true).open(&path).is_err();

        let outcome = write_atomic(&path, "replaced");

        if enforced {
            assert!(outcome.is_err(), "a read-only file must not be replaced");
            assert_eq!(fs::read_to_string(&path).expect("read"), "keep me");
            assert_eq!(names(&dir), ["locked.Spe"]);
        }
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).ok();
        fs::remove_dir_all(&dir).ok();
    }

    /// Unix only: on Windows the read-only attribute on a folder does not stop
    /// files being created in it, so the case cannot be staged there. Skipped
    /// as root, who creates files in any folder.
    #[cfg(unix)]
    #[test]
    fn a_writable_file_in_a_read_only_folder_still_saves_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("ro-folder");
        let path = dir.join("shared.Spe");
        fs::write(&path, "old").expect("old");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("lock the folder");
        let locked = File::create(dir.join("probe")).is_err();

        let outcome = write_atomic(&path, "new, and longer than the old");
        let left = names(&dir);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("unlock");

        if locked {
            outcome.expect("the save must still work");
            assert_eq!(
                fs::read_to_string(&path).expect("read"),
                "new, and longer than the old"
            );
            assert_eq!(left, ["shared.Spe"]);
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn the_new_file_keeps_the_old_one_s_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("mode");
        let path = dir.join("shared.json");
        fs::write(&path, "old").expect("old");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).expect("mode");
        write_atomic(&path, "new").expect("save");
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o664);
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_followed_and_survives() {
        let dir = scratch_dir("link");
        let real = dir.join("real.Roi");
        let link = dir.join("link.Roi");
        fs::write(&real, "old").expect("old");
        std::os::unix::fs::symlink(&real, &link).expect("link");
        write_atomic(&link, "new").expect("save");
        assert!(
            fs::symlink_metadata(&link)
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).expect("read"), "new");
        fs::remove_dir_all(&dir).ok();
    }

    /// A folder that takes the `.part` but refuses the rename onto the target
    /// (Windows: the target held open by a reader that shares write but not
    /// delete; Unix: a sticky folder and a target owned by another user)
    /// saved on 0.2.4, so it still saves, in place, with no `.part` left
    /// (Lucius, review of 3dfa038). The refusal is injected: neither setup
    /// can be staged on a one-user Linux box.
    #[test]
    fn a_refused_rename_falls_back_to_writing_in_place() {
        let dir = scratch_dir("rename-refused");
        let path = dir.join("count.Spe");
        fs::write(&path, "an old count, longer than the new one").expect("old");

        let outcome = write_atomic_via(
            &path,
            |file| file.write_all(b"new count"),
            |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        );

        outcome.expect("a save 0.2.4 made must still work");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new count");
        assert_eq!(names(&dir), ["count.Spe"], "no temporary file left");
        // And a first save into such a folder.
        let fresh = dir.join("fresh.Chn");
        write_atomic_via(
            &fresh,
            |file| file.write_all(b"first"),
            |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        )
        .expect("a first save");
        assert_eq!(fs::read_to_string(&fresh).expect("read"), "first");
        assert_eq!(names(&dir), ["count.Spe", "fresh.Chn"]);
        fs::remove_dir_all(&dir).ok();
    }

    /// Any other rename failure is still reported, the old file kept.
    #[test]
    fn another_rename_failure_is_reported_and_keeps_the_old_file() {
        let dir = scratch_dir("rename-failed");
        let path = dir.join("count.Spe");
        fs::write(&path, "old").expect("old");

        let outcome = write_atomic_via(
            &path,
            |file| file.write_all(b"new"),
            |_, _| Err(io::Error::other("the stick was pulled")),
        );

        assert!(outcome.is_err());
        assert_eq!(fs::read_to_string(&path).expect("read"), "old");
        assert_eq!(names(&dir), ["count.Spe"], "no temporary file left");
        fs::remove_dir_all(&dir).ok();
    }
}
