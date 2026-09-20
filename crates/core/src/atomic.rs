//! Durable file replacement: write a temporary file, flush it to disk,
//! rename it over the target, then flush the directory so the rename
//! itself survives a power loss.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// Replaces `path` with `bytes`. With `private`, the file is created
/// readable by the owner only where the platform has such a notion.
pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    let dir = path.parent().context("the path has a parent")?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("the path has a file name")?;
    let suffix: u32 = rand::random();
    let tmp = dir.join(format!(".{name}.tmp-{suffix:08x}"));

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;

    let written = options
        .open(&tmp)
        .and_then(|mut file| file.write_all(bytes).and_then(|_| file.sync_all()));
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", tmp.display()));
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("renaming into {}", path.display()));
    }
    // Directories cannot be opened as files everywhere; where they can,
    // this makes the new name durable too.
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_target_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"one", false).unwrap();
        write_atomic(&path, b"two", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["state.json"]);
    }
}
