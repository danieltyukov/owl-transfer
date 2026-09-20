//! The sync rules: what to do with a remote entry given the local one.
//! Pure logic, tested without IO.

use crate::index::{Entry, EntryKind, Index};
use crate::paths::{conflict_name, file_name_of, join_rel, parent_of};
use crate::vv::{bump, compare, merge, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Ours is the same or newer; the peer has or will get it from our index.
    Ignore,
    /// Take the remote entry (download, create the directory or record the
    /// tombstone).
    Adopt,
    /// Both changed. `winner_remote` says whose version keeps the name; the
    /// loser becomes a conflict copy on the side that holds it.
    Conflict { winner_remote: bool },
}

/// Decides for one path. `remote_device` is the peer that sent the entry.
pub fn decide(
    local: Option<&Entry>,
    remote: &Entry,
    local_device: &str,
    remote_device: &str,
) -> Decision {
    let Some(local) = local else {
        return Decision::Adopt;
    };
    match compare(&local.vv, &remote.vv) {
        Ordering::Equal | Ordering::Dominates => Decision::Ignore,
        Ordering::Dominated => Decision::Adopt,
        Ordering::Concurrent => match (local.deleted, remote.deleted) {
            // Two tombstones only differ in their vectors; merge them.
            (true, true) => Decision::Adopt,
            // A deletion never destroys an edit: the live side wins. When
            // ours is live we keep it and bump so the peer takes it back.
            (true, false) => Decision::Adopt,
            (false, true) => Decision::Conflict {
                winner_remote: false,
            },
            (false, false) => {
                let same_dir = local.kind == EntryKind::Dir && remote.kind == EntryKind::Dir;
                let same_file = local.kind == EntryKind::File
                    && remote.kind == EntryKind::File
                    && local.hash == remote.hash;
                if same_dir || same_file {
                    return Decision::Adopt;
                }
                let remote_wins = remote.mtime_ms > local.mtime_ms
                    || (remote.mtime_ms == local.mtime_ms && remote_device > local_device);
                Decision::Conflict {
                    winner_remote: remote_wins,
                }
            }
        },
    }
}

/// Whether adopting `remote` needs the file's bytes: it is a live file and
/// the local entry is not already a live file with the same content.
pub fn needs_bytes(local: Option<&Entry>, remote: &Entry) -> bool {
    if !remote.is_live_file() {
        return false;
    }
    !matches!(local, Some(l) if l.is_live_file() && l.hash == remote.hash)
}

/// Records the remote entry in the index with the merge of both vectors and
/// returns what was stored.
pub fn apply_adopt(index: &mut Index, remote: &Entry, now_ms: i64) -> Entry {
    let mut entry = remote.clone();
    if let Some(local) = index.get(&entry.path) {
        entry.vv = merge(&local.vv, &entry.vv);
    }
    entry.seen_at_ms = now_ms;
    index.insert(entry.clone());
    entry
}

/// Records a conflict's outcome. The winner keeps the path with the merge of
/// both vectors plus a bump, so it dominates both sides. When the remote
/// side wins and ours is live, ours is recorded under a conflict name with a
/// fresh vector and the same hash; the caller moves the file. When ours
/// wins there is nothing to copy here: the peer makes its own copy.
pub fn resolve_conflict(
    index: &mut Index,
    local: &Entry,
    remote: &Entry,
    winner_remote: bool,
    local_device: &str,
    local_name: &str,
    now_ms: i64,
) -> (Entry, Option<Entry>) {
    let mut winner = if winner_remote {
        remote.clone()
    } else {
        local.clone()
    };
    let floor = index.floor();
    winner.vv = merge(&local.vv, &remote.vv);
    bump(&mut winner.vv, local_device, floor);
    winner.seen_at_ms = now_ms;

    let loser = if winner_remote && !local.deleted {
        let name = conflict_name(file_name_of(&local.path), local_name, now_ms);
        let mut copy = local.clone();
        copy.path = join_rel(parent_of(&local.path), &name);
        copy.vv.clear();
        bump(&mut copy.vv, local_device, floor);
        copy.seen_at_ms = now_ms;
        index.insert(copy.clone());
        Some(copy)
    } else {
        None
    };
    index.insert(winner.clone());
    (winner, loser)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const ME: &str = "aaaa";
    const PEER: &str = "bbbb";

    fn vv(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn file(path: &str, hash: &str, mtime: i64, v: &[(&str, u64)]) -> Entry {
        Entry {
            path: path.into(),
            kind: EntryKind::File,
            size: 3,
            mtime_ms: mtime,
            hash: Some(hash.into()),
            deleted: false,
            vv: vv(v),
            seen_at_ms: 0,
        }
    }

    fn tomb(path: &str, v: &[(&str, u64)]) -> Entry {
        Entry {
            path: path.into(),
            kind: EntryKind::File,
            size: 0,
            mtime_ms: 0,
            hash: None,
            deleted: true,
            vv: vv(v),
            seen_at_ms: 0,
        }
    }

    #[test]
    fn no_local_adopts_even_a_tombstone() {
        let r = tomb("a", &[(PEER, 1)]);
        assert_eq!(decide(None, &r, ME, PEER), Decision::Adopt);
        assert!(!needs_bytes(None, &r));
        assert!(needs_bytes(None, &file("a", "h", 1, &[(PEER, 1)])));
    }

    #[test]
    fn ordering_decides_the_easy_cases() {
        let l = file("a", "h1", 1, &[(ME, 1), (PEER, 1)]);
        assert_eq!(decide(Some(&l), &l, ME, PEER), Decision::Ignore);
        let older = file("a", "h0", 1, &[(PEER, 1)]);
        assert_eq!(decide(Some(&l), &older, ME, PEER), Decision::Ignore);
        let newer = file("a", "h2", 1, &[(ME, 1), (PEER, 2)]);
        assert_eq!(decide(Some(&l), &newer, ME, PEER), Decision::Adopt);
        assert!(needs_bytes(Some(&l), &newer));
    }

    #[test]
    fn concurrent_same_content_adopts_without_bytes() {
        let l = file("a", "h", 1, &[(ME, 2), (PEER, 1)]);
        let r = file("a", "h", 9, &[(ME, 1), (PEER, 2)]);
        assert_eq!(decide(Some(&l), &r, ME, PEER), Decision::Adopt);
        assert!(!needs_bytes(Some(&l), &r));
        let mut index = Index::default();
        index.insert(l.clone());
        let stored = apply_adopt(&mut index, &r, 50);
        assert_eq!(stored.vv, vv(&[(ME, 2), (PEER, 2)]));
        assert_eq!(stored.seen_at_ms, 50);
        assert_eq!(index.get("a"), Some(&stored));
    }

    #[test]
    fn deletion_never_destroys_an_edit() {
        let local_tomb = tomb("a", &[(ME, 2), (PEER, 1)]);
        let remote_live = file("a", "h", 1, &[(ME, 1), (PEER, 2)]);
        assert_eq!(
            decide(Some(&local_tomb), &remote_live, ME, PEER),
            Decision::Adopt
        );

        let local_live = file("a", "h", 1, &[(ME, 2), (PEER, 1)]);
        let remote_tomb = tomb("a", &[(ME, 1), (PEER, 2)]);
        assert_eq!(
            decide(Some(&local_live), &remote_tomb, ME, PEER),
            Decision::Conflict {
                winner_remote: false
            }
        );
        let mut index = Index::default();
        index.insert(local_live.clone());
        let (winner, loser) =
            resolve_conflict(&mut index, &local_live, &remote_tomb, false, ME, "Me", 7);
        assert!(loser.is_none());
        assert!(!winner.deleted);
        assert_eq!(winner.vv, vv(&[(ME, 3), (PEER, 2)]));
        assert_eq!(compare(&winner.vv, &remote_tomb.vv), Ordering::Dominates);
    }

    #[test]
    fn dominated_tombstone_is_adopted() {
        let local_live = file("a", "h", 1, &[(ME, 1)]);
        let remote_tomb = tomb("a", &[(ME, 1), (PEER, 2)]);
        assert_eq!(
            decide(Some(&local_live), &remote_tomb, ME, PEER),
            Decision::Adopt
        );
    }

    #[test]
    fn newer_mtime_wins_and_ties_go_to_the_larger_id() {
        let l = file("a", "h1", 100, &[(ME, 2), (PEER, 1)]);
        let r_newer = file("a", "h2", 200, &[(ME, 1), (PEER, 2)]);
        assert_eq!(
            decide(Some(&l), &r_newer, ME, PEER),
            Decision::Conflict {
                winner_remote: true
            }
        );
        let r_older = file("a", "h2", 50, &[(ME, 1), (PEER, 2)]);
        assert_eq!(
            decide(Some(&l), &r_older, ME, PEER),
            Decision::Conflict {
                winner_remote: false
            }
        );
        let r_tie = file("a", "h2", 100, &[(ME, 1), (PEER, 2)]);
        assert_eq!(
            decide(Some(&l), &r_tie, ME, PEER),
            Decision::Conflict {
                winner_remote: true
            }
        );
        assert_eq!(
            decide(Some(&l), &r_tie, PEER, ME),
            Decision::Conflict {
                winner_remote: false
            }
        );
    }

    #[test]
    fn remote_win_makes_a_conflict_copy_of_ours() {
        let l = file("docs/a.txt", "h1", 100, &[(ME, 2), (PEER, 1)]);
        let r = file("docs/a.txt", "h2", 200, &[(ME, 1), (PEER, 2)]);
        let mut index = Index::default();
        index.insert(l.clone());
        let when = 1_789_911_900_000;
        let (winner, loser) = resolve_conflict(&mut index, &l, &r, true, ME, "Desk", when);
        assert_eq!(winner.hash.as_deref(), Some("h2"));
        assert_eq!(winner.vv, vv(&[(ME, 3), (PEER, 2)]));
        let loser = loser.unwrap();
        assert_eq!(
            loser.path,
            "docs/a (conflict from Desk 2026-09-20 13-45).txt"
        );
        assert_eq!(loser.hash.as_deref(), Some("h1"));
        assert_eq!(loser.vv, vv(&[(ME, 1)]));
        assert_eq!(index.get("docs/a.txt"), Some(&winner));
        assert_eq!(index.get(&loser.path), Some(&loser));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn local_win_bumps_and_copies_nothing() {
        let l = file("a", "h1", 300, &[(ME, 2), (PEER, 1)]);
        let r = file("a", "h2", 200, &[(ME, 1), (PEER, 2)]);
        let mut index = Index::default();
        index.insert(l.clone());
        let (winner, loser) = resolve_conflict(&mut index, &l, &r, false, ME, "Desk", 1);
        assert!(loser.is_none());
        assert_eq!(winner.hash.as_deref(), Some("h1"));
        assert_eq!(winner.vv, vv(&[(ME, 3), (PEER, 2)]));
        assert_eq!(index.len(), 1);
    }
}
