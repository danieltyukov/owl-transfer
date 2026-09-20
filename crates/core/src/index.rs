//! The index: one entry per path in the sync folder, persisted as a single
//! JSON file and rewritten atomically.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{is_under, parent_of};
use crate::vv::VersionVector;

const INDEX_FILE: &str = "index.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    /// Relative, `/` separated, NFC.
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_ms: i64,
    /// BLAKE3 hex, files only.
    pub hash: Option<String>,
    /// A tombstone.
    pub deleted: bool,
    pub vv: VersionVector,
    /// Local wall clock when this entry was last written, for tombstone expiry.
    pub seen_at_ms: i64,
}

impl Entry {
    pub fn is_live_file(&self) -> bool {
        !self.deleted && self.kind == EntryKind::File
    }

    pub fn is_live_dir(&self) -> bool {
        !self.deleted && self.kind == EntryKind::Dir
    }
}

#[derive(Default, Serialize, Deserialize)]
struct IndexFile {
    entries: Vec<Entry>,
}

#[derive(Debug, Default, Clone)]
pub struct Index {
    entries: BTreeMap<String, Entry>,
}

impl Index {
    /// Loads `data_dir/index.json`; a missing file is an empty index.
    pub fn load(data_dir: &Path) -> Result<Index> {
        let path = data_dir.join(INDEX_FILE);
        if !path.exists() {
            return Ok(Index::default());
        }
        let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let file: IndexFile =
            serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Index {
            entries: file
                .entries
                .into_iter()
                .map(|e| (e.path.clone(), e))
                .collect(),
        })
    }

    /// Deletes the persisted file, for a folder change.
    pub fn delete_file(data_dir: &Path) -> Result<()> {
        let path = data_dir.join(INDEX_FILE);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        }
    }

    pub fn get(&self, path: &str) -> Option<&Entry> {
        self.entries.get(path)
    }

    pub fn insert(&mut self, e: Entry) {
        self.entries.insert(e.path.clone(), e);
    }

    pub fn remove(&mut self, path: &str) -> Option<Entry> {
        self.entries.remove(path)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remove_tombstones_older_than(&mut self, ms: i64) {
        self.entries
            .retain(|_, e| !(e.deleted && e.seen_at_ms < ms));
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// Live entries that are `dir` itself or lie under it.
    pub fn live_within(&self, dir: &str) -> Vec<&Entry> {
        self.entries
            .values()
            .filter(|e| !e.deleted && is_under(&e.path, dir))
            .collect()
    }

    /// Direct live children of `dir` (`""` is the root).
    pub fn live_under(&self, dir: &str) -> Vec<&Entry> {
        self.entries
            .values()
            .filter(|e| !e.deleted && !e.path.is_empty() && parent_of(&e.path) == dir)
            .collect()
    }

    /// A live file with this hash, if any.
    pub fn find_by_hash(&self, hash: &str) -> Option<&Entry> {
        self.entries
            .values()
            .find(|e| e.is_live_file() && e.hash.as_deref() == Some(hash))
    }

    /// Writes `index.json.tmp` then renames it over `index.json`.
    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = data_dir.join(INDEX_FILE);
        let tmp = data_dir.join("index.json.tmp");
        let file = IndexFile {
            entries: self.entries.values().cloned().collect(),
        };
        fs::write(&tmp, serde_json::to_vec(&file)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
        Ok(())
    }

    /// Live files, live directories and the bytes of the live files.
    pub fn summary(&self) -> (u64, u64, u64) {
        let mut files = 0;
        let mut dirs = 0;
        let mut bytes = 0;
        for e in self.entries.values().filter(|e| !e.deleted) {
            match e.kind {
                EntryKind::File => {
                    files += 1;
                    bytes += e.size;
                }
                EntryKind::Dir => dirs += 1,
            }
        }
        (files, dirs, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: EntryKind, deleted: bool) -> Entry {
        Entry {
            path: path.into(),
            kind,
            size: if kind == EntryKind::File { 3 } else { 0 },
            mtime_ms: 1,
            hash: (kind == EntryKind::File).then(|| format!("h-{path}")),
            deleted,
            vv: BTreeMap::from([("dev".to_string(), 1)]),
            seen_at_ms: 5,
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::load(dir.path()).unwrap();
        assert!(index.is_empty());
        index.insert(entry("a.txt", EntryKind::File, false));
        index.insert(entry("d", EntryKind::Dir, false));
        index.insert(entry("gone.txt", EntryKind::File, true));
        index.save(dir.path()).unwrap();

        let again = Index::load(dir.path()).unwrap();
        assert_eq!(again.len(), 3);
        assert_eq!(again.get("a.txt"), index.get("a.txt"));
        assert!(again.get("gone.txt").unwrap().deleted);
        assert_eq!(again.summary(), (1, 1, 3));
        assert!(!dir.path().join("index.json.tmp").exists());
    }

    #[test]
    fn live_under_excludes_tombstones_and_nested_paths() {
        let mut index = Index::default();
        index.insert(entry("a.txt", EntryKind::File, false));
        index.insert(entry("d", EntryKind::Dir, false));
        index.insert(entry("d/inner.txt", EntryKind::File, false));
        index.insert(entry("gone.txt", EntryKind::File, true));
        let names: Vec<&str> = index
            .live_under("")
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(names, vec!["a.txt", "d"]);
        let inner: Vec<&str> = index
            .live_under("d")
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(inner, vec!["d/inner.txt"]);
        let within: Vec<&str> = index
            .live_within("d")
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(within, vec!["d", "d/inner.txt"]);
    }

    #[test]
    fn find_by_hash_only_returns_live_files() {
        let mut index = Index::default();
        index.insert(entry("gone.txt", EntryKind::File, true));
        assert!(index.find_by_hash("h-gone.txt").is_none());
        index.insert(entry("a.txt", EntryKind::File, false));
        assert_eq!(index.find_by_hash("h-a.txt").unwrap().path, "a.txt");
    }

    #[test]
    fn tombstone_expiry_keeps_live_entries() {
        let mut index = Index::default();
        index.insert(entry("a.txt", EntryKind::File, false));
        index.insert(entry("gone.txt", EntryKind::File, true));
        index.remove_tombstones_older_than(10);
        assert_eq!(index.len(), 1);
        assert!(index.get("a.txt").is_some());
    }
}
