//! Paths the engine never indexes: its own temporary files, its metadata
//! directory, operating system litter and browsers' partial downloads.

/// `rel_path` is a relative path with forward slashes.
pub fn is_ignored(rel_path: &str) -> bool {
    for component in rel_path.split('/') {
        if component == ".owl"
            || component.starts_with(".owl-tmp-")
            || component == ".DS_Store"
            || component.eq_ignore_ascii_case("Thumbs.db")
            || component.eq_ignore_ascii_case("desktop.ini")
            || component.starts_with("~$")
        {
            return true;
        }
    }
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    name.ends_with(".crdownload") || name.ends_with(".part")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_rule_matches() {
        assert!(is_ignored(".owl"));
        assert!(is_ignored(".owl/index.json"));
        assert!(is_ignored("a/.owl/x"));
        assert!(is_ignored(".owl-tmp-abc"));
        assert!(is_ignored("docs/.owl-tmp-abc"));
        assert!(is_ignored(".DS_Store"));
        assert!(is_ignored("photos/.DS_Store"));
        assert!(is_ignored("Thumbs.db"));
        assert!(is_ignored("thumbs.db"));
        assert!(is_ignored("desktop.ini"));
        assert!(is_ignored("~$report.docx"));
        assert!(is_ignored("a/~$report.docx"));
        assert!(is_ignored("movie.mp4.crdownload"));
        assert!(is_ignored("movie.mp4.part"));
    }

    #[test]
    fn normal_paths_are_not_ignored() {
        assert!(!is_ignored("report.pdf"));
        assert!(!is_ignored("a/b/c.txt"));
        assert!(!is_ignored(".hidden"));
        assert!(!is_ignored("owl-tmp-x"));
        assert!(!is_ignored("partial"));
        assert!(!is_ignored(""));
    }
}
