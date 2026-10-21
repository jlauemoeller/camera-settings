//! File naming and writing for captures (SPEC §5).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Local};

pub const MIN_FREE_START: u64 = 1 << 30;
/// Stop a running recording below this much free space.
pub const MIN_FREE_RUNNING: u64 = 256 << 20;

fn base_name(t: DateTime<Local>) -> String {
    t.format("BRIO_%Y-%m-%d_%H-%M-%S").to_string()
}

fn candidates(dir: &Path, t: DateTime<Local>, ext: &str) -> impl Iterator<Item = PathBuf> {
    let base = base_name(t);
    let dir = dir.to_owned();
    let ext = ext.to_owned();
    (1..).map(move |n| match n {
        1 => dir.join(format!("{base}.{ext}")),
        n => dir.join(format!("{base}_{n}.{ext}")),
    })
}

/// Create (and reserve) a new, empty capture file: `BRIO_<date>_<time>[_N].<ext>`.
pub fn reserve(dir: &Path, ext: &str) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(dir)?;
    for path in candidates(dir, Local::now(), ext).take(1000) {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("too many captures with the same timestamp"))
}

/// Write a still so that a complete file or no file exists, never a partial one: write a hidden
/// temp file, fsync, then hard-link it to a free name (fails rather than overwrites) and unlink.
pub fn write_still(dir: &Path, jpeg: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".camera-settings-{}-{seq}.jpg.part", std::process::id()));
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(jpeg)?;
        f.sync_all()?;
        for path in candidates(dir, Local::now(), "jpg").take(1000) {
            match fs::hard_link(&tmp, &path) {
                Ok(()) => return Ok(path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("too many captures with the same timestamp"))
    })();
    let _ = fs::remove_file(&tmp);
    result
}

/// Bytes available to unprivileged users on the filesystem holding `dir`.
pub fn free_space(dir: &Path) -> io::Result<u64> {
    let s = nix::sys::statvfs::statvfs(dir).map_err(io::Error::from)?;
    #[allow(clippy::useless_conversion)]
    Ok(u64::from(s.blocks_available()) * u64::from(s.fragment_size()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("camera-settings-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn names_and_collisions() {
        let dir = tmpdir("names");
        let a = write_still(&dir, b"\xff\xd8x\xff\xd9").unwrap();
        let b = write_still(&dir, b"\xff\xd8y\xff\xd9").unwrap();
        let (c, _) = reserve(&dir, "jpg").unwrap();
        let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name(&a).starts_with("BRIO_") && name(&a).ends_with(".jpg"));
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(fs::read(&a).unwrap(), b"\xff\xd8x\xff\xd9");
        // No temp files left behind.
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter(|e| name(&e.as_ref().unwrap().path()).starts_with('.'))
            .count();
        assert_eq!(leftovers, 0);
        assert!(free_space(&dir).unwrap() > 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn collision_suffix_format() {
        let t = Local::now();
        let names: Vec<_> = candidates(Path::new("/x"), t, "mkv").take(3).collect();
        let base = base_name(t);
        assert_eq!(names[0], PathBuf::from(format!("/x/{base}.mkv")));
        assert_eq!(names[1], PathBuf::from(format!("/x/{base}_2.mkv")));
        assert_eq!(names[2], PathBuf::from(format!("/x/{base}_3.mkv")));
    }
}
