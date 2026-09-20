//! Relative path rules. Every path the engine stores or sends is relative to
//! the sync folder, uses forward slashes and is NFC normalised.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use unicode_normalization::UnicodeNormalization;

/// Strips `root`, joins the remaining components with `/` and normalises to
/// NFC. Returns `None` if `path` is not under `root` or has a component that
/// is not a plain name. The root itself maps to `""`.
pub fn normalize_rel(path: &Path, root: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.nfc().collect::<String>()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// Rejects the empty path, `..` or `.` components, empty components, a
/// leading slash, backslashes and NUL bytes.
pub fn validate_rel(rel: &str) -> Result<()> {
    if rel.is_empty() {
        bail!("path is empty");
    }
    if rel.starts_with('/') {
        bail!("path must be relative: {rel}");
    }
    if rel.contains('\\') {
        bail!("path must use forward slashes: {rel}");
    }
    if rel.contains('\0') {
        bail!("path contains a NUL byte");
    }
    for component in rel.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            bail!("path has an invalid component: {rel}");
        }
    }
    Ok(())
}

/// The absolute path of a relative one, refusing any component that is a
/// symbolic link, so a peer can never write or delete through a link the
/// person keeps inside the folder. Components that do not exist yet are
/// fine; `""` is the root itself.
pub fn safe_abs(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Ok(root.to_path_buf());
    }
    validate_rel(rel)?;
    let mut abs = root.to_path_buf();
    for component in rel.split('/') {
        abs.push(component);
        match std::fs::symlink_metadata(&abs) {
            Ok(md) if md.file_type().is_symlink() => {
                bail!("{rel} goes through a symbolic link")
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(root.join(rel)),
            Err(e) => return Err(e).with_context(|| format!("checking {}", abs.display())),
        }
    }
    Ok(abs)
}

/// Rejects anything that cannot be a single file or folder name on every
/// platform the folder may reach.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("name is empty");
    }
    if name == "." || name == ".." {
        bail!("name is reserved: {name}");
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        bail!("name contains a path separator: {name}");
    }
    portable_component(name)
}

/// Rejects a relative path with a component that Windows or Android's
/// shared storage cannot hold: the characters `< > : " | ? *`, control
/// characters, a trailing dot or space, and the reserved DOS device names.
pub fn validate_portable(rel: &str) -> Result<()> {
    for component in rel.split('/') {
        portable_component(component)?;
    }
    Ok(())
}

fn portable_component(name: &str) -> Result<()> {
    if let Some(c) = name
        .chars()
        .find(|c| "<>:\"|?*".contains(*c) || c.is_control())
    {
        bail!("{name:?} contains {c:?}, which not every device can store");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        bail!("{name:?} ends with a dot or a space, which not every device can store");
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        bail!("{name:?} is a reserved name on Windows");
    }
    Ok(())
}

/// The directory part of a relative path; `""` for a top-level entry.
pub fn parent_of(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(i) => &rel[..i],
        None => "",
    }
}

/// The last component of a relative path.
pub fn file_name_of(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(i) => &rel[i + 1..],
        None => rel,
    }
}

pub fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Whether `child` is `dir` or lies under it. `""` is under everything.
pub fn is_under(child: &str, dir: &str) -> bool {
    dir.is_empty() || child == dir || child.starts_with(dir) && child[dir.len()..].starts_with('/')
}

const CONFLICT_MARK: &str = " (conflict from ";

/// `report (conflict from Pixel 2026-09-20 13-45).pdf`. The time is UTC,
/// which stands as a decision: the crate carries no timezone data, and the
/// name only has to be unique and readable.
pub fn conflict_name(name: &str, device_name: &str, when_ms: i64) -> String {
    let (stem, ext) = split_extension(name);
    let (y, mo, d, h, mi) = civil_from_ms(when_ms);
    let device: String = device_name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("{stem}{CONFLICT_MARK}{device} {y:04}-{mo:02}-{d:02} {h:02}-{mi:02}){ext}")
}

pub fn is_conflict_name(name: &str) -> bool {
    name.contains(CONFLICT_MARK)
}

/// Splits `name` into stem and extension including the dot; a leading dot
/// does not start an extension.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Year, month, day, hour and minute of a Unix millisecond timestamp in UTC,
/// after Howard Hinnant's civil-from-days.
fn civil_from_ms(ms: i64) -> (i64, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hour, minute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn normalize_strips_root_and_uses_forward_slashes() {
        let root = PathBuf::from("/tmp/owl");
        let joined = root.join("a").join("b.txt");
        assert_eq!(normalize_rel(&joined, &root).as_deref(), Some("a/b.txt"));
        assert_eq!(normalize_rel(&root, &root).as_deref(), Some(""));
        assert_eq!(normalize_rel(Path::new("/elsewhere/x"), &root), None);
    }

    #[test]
    fn normalize_converts_nfd_to_nfc() {
        let root = PathBuf::from("/tmp/owl");
        let nfd = "e\u{0301}.txt";
        let path = root.join(nfd);
        assert_eq!(normalize_rel(&path, &root).as_deref(), Some("\u{e9}.txt"));
    }

    #[test]
    fn validate_rejects_bad_paths() {
        assert!(validate_rel("../x").is_err());
        assert!(validate_rel("a/../x").is_err());
        assert!(validate_rel("/x").is_err());
        assert!(validate_rel("").is_err());
        assert!(validate_rel("a\\b").is_err());
        assert!(validate_rel("a//b").is_err());
        assert!(validate_rel("./a").is_err());
        assert!(validate_rel("a\0b").is_err());
        assert!(validate_rel("a/b.txt").is_ok());
        assert!(validate_rel("..hidden").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn safe_abs_refuses_symbolic_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("file"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("file"), root.join("f")).unwrap();

        assert!(safe_abs(&root, "link/x").is_err());
        assert!(safe_abs(&root, "link").is_err());
        assert!(safe_abs(&root, "f").is_err());
        assert_eq!(safe_abs(&root, "real/x").unwrap(), root.join("real/x"));
        assert_eq!(
            safe_abs(&root, "new/deeper/x").unwrap(),
            root.join("new/deeper/x")
        );
        assert_eq!(safe_abs(&root, "").unwrap(), root);
        assert!(safe_abs(&root, "../x").is_err());
    }

    #[test]
    fn validate_name_rejects_separators() {
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("report.pdf").is_ok());
    }

    #[test]
    fn portable_names_reject_what_windows_cannot_store() {
        for bad in [
            "a:b.txt",
            "what?.txt",
            "star*.txt",
            "quote\".txt",
            "pipe|.txt",
            "lt<.txt",
            "gt>.txt",
            "trailing.",
            "trailing ",
            "CON",
            "con.txt",
            "COM1",
            "lpt9.log",
            "nul.ls.ok",
            "tab\t.txt",
        ] {
            assert!(validate_portable(bad).is_err(), "{bad}");
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        for good in [
            "report.pdf",
            "com.txt",
            "COM0",
            "COM10",
            "console",
            ".hidden",
            "a b.txt",
            "caf\u{e9}.txt",
        ] {
            assert!(validate_portable(good).is_ok(), "{good}");
        }
        assert!(validate_portable("ok/CON/x").is_err());
        assert!(validate_portable("ok/fine/x").is_ok());
    }

    #[test]
    fn parent_and_name() {
        assert_eq!(parent_of("a/b/c.txt"), "a/b");
        assert_eq!(parent_of("c.txt"), "");
        assert_eq!(file_name_of("a/b/c.txt"), "c.txt");
        assert_eq!(file_name_of("c.txt"), "c.txt");
        assert_eq!(join_rel("", "x"), "x");
        assert_eq!(join_rel("a", "x"), "a/x");
        assert!(is_under("a/b", "a"));
        assert!(is_under("a", "a"));
        assert!(!is_under("ab", "a"));
        assert!(is_under("anything", ""));
    }

    #[test]
    fn conflict_name_keeps_the_extension() {
        // 2026-09-20 13:45 UTC
        let when = 1_789_911_900_000;
        assert_eq!(
            conflict_name("report.pdf", "Pixel", when),
            "report (conflict from Pixel 2026-09-20 13-45).pdf"
        );
        assert_eq!(
            conflict_name("notes", "Desk:top", when),
            "notes (conflict from Desk-top 2026-09-20 13-45)"
        );
        assert_eq!(
            conflict_name(".env", "Pixel", when),
            ".env (conflict from Pixel 2026-09-20 13-45)"
        );
        assert!(is_conflict_name(&conflict_name("a.txt", "b", when)));
        assert!(!is_conflict_name("a.txt"));
    }

    #[test]
    fn civil_dates_are_right() {
        assert_eq!(civil_from_ms(0), (1970, 1, 1, 0, 0));
        assert_eq!(civil_from_ms(951_782_400_000), (2000, 2, 29, 0, 0));
        assert_eq!(civil_from_ms(1_789_911_900_000), (2026, 9, 20, 13, 45));
    }
}
