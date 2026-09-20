//! The paired peers, persisted in `peers.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::atomic::write_atomic;
use crate::clock::now_ms;
use crate::config::DeviceKind;

const PEERS_FILE: &str = "peers.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub paired_at_ms: i64,
    /// `host:port` of the last successful or heard address, for redialling.
    pub last_address: Option<String>,
    /// When the peer was last heard from, over the beacon or a connection.
    #[serde(default)]
    pub last_seen_ms: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct PeersFile {
    peers: Vec<PeerRecord>,
}

#[derive(Debug)]
pub struct PeerStore {
    path: PathBuf,
    peers: BTreeMap<String, PeerRecord>,
}

impl PeerStore {
    /// Loads `data_dir/peers.json`; a missing file is an empty store and a
    /// damaged one is moved aside (see `load_with_note`).
    pub fn load(data_dir: &Path) -> Result<PeerStore> {
        Ok(PeerStore::load_with_note(data_dir)?.0)
    }

    /// `load`, returning what happened to a damaged file so the caller can
    /// tell the person that every device needs pairing again.
    pub fn load_with_note(data_dir: &Path) -> Result<(PeerStore, Option<String>)> {
        let path = data_dir.join(PEERS_FILE);
        if !path.exists() {
            return Ok((
                PeerStore {
                    path,
                    peers: BTreeMap::new(),
                },
                None,
            ));
        }
        let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        match serde_json::from_slice::<PeersFile>(&raw) {
            Ok(file) => Ok((
                PeerStore {
                    path,
                    peers: file.peers.into_iter().map(|p| (p.id.clone(), p)).collect(),
                },
                None,
            )),
            Err(e) => {
                let aside = data_dir.join(format!("peers.json.corrupt-{}", now_ms()));
                fs::rename(&path, &aside)
                    .with_context(|| format!("moving {} aside", path.display()))?;
                let note = format!(
                    "the list of paired devices was unreadable ({e}); it was moved to {} and \
                     every device needs pairing again",
                    aside.display()
                );
                warn!("{note}");
                Ok((
                    PeerStore {
                        path,
                        peers: BTreeMap::new(),
                    },
                    Some(note),
                ))
            }
        }
    }

    pub fn list(&self) -> Vec<PeerRecord> {
        self.peers.values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<PeerRecord> {
        self.peers.get(id).cloned()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.peers.contains_key(id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Inserts or replaces a record and persists.
    pub fn upsert(&mut self, rec: PeerRecord) -> Result<()> {
        self.peers.insert(rec.id.clone(), rec);
        self.save()
    }

    /// Removes a record and persists. Returns whether it existed.
    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let existed = self.peers.remove(id).is_some();
        if existed {
            self.save()?;
        }
        Ok(existed)
    }

    pub fn set_address(&mut self, id: &str, addr: String) -> Result<()> {
        if let Some(rec) = self.peers.get_mut(id) {
            if rec.last_address.as_deref() != Some(addr.as_str()) {
                rec.last_address = Some(addr);
                return self.save();
            }
        }
        Ok(())
    }

    /// Records when a peer was last heard. Not persisted on every call, since
    /// the beacon ticks every two seconds; `save` is called by the other
    /// mutators and at shutdown.
    pub fn set_last_seen(&mut self, id: &str, at_ms: i64) {
        if let Some(rec) = self.peers.get_mut(id) {
            rec.last_seen_ms = Some(at_ms);
        }
    }

    pub fn save(&self) -> Result<()> {
        let file = PeersFile {
            peers: self.peers.values().cloned().collect(),
        };
        write_atomic(&self.path, &serde_json::to_vec_pretty(&file)?, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> PeerRecord {
        PeerRecord {
            id: id.into(),
            name: "Pixel".into(),
            kind: DeviceKind::Phone,
            paired_at_ms: 1,
            last_address: None,
            last_seen_ms: None,
        }
    }

    #[test]
    fn upsert_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(dir.path()).unwrap();
        assert!(store.list().is_empty());
        store.upsert(rec("a")).unwrap();
        store.set_address("a", "10.0.0.2:52734".into()).unwrap();

        let again = PeerStore::load(dir.path()).unwrap();
        let got = again.get("a").unwrap();
        assert_eq!(got.name, "Pixel");
        assert_eq!(got.last_address.as_deref(), Some("10.0.0.2:52734"));
        assert_eq!(again.list().len(), 1);
    }

    #[test]
    fn a_damaged_store_is_moved_aside_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("peers.json"), b"").unwrap();
        let (store, note) = PeerStore::load_with_note(dir.path()).unwrap();
        assert!(store.is_empty());
        assert!(note.unwrap().contains("pairing again"));
        assert!(!dir.path().join("peers.json").exists());
    }

    #[test]
    fn remove_returns_whether_it_existed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(dir.path()).unwrap();
        store.upsert(rec("a")).unwrap();
        assert!(store.remove("a").unwrap());
        assert!(!store.remove("a").unwrap());
        assert!(PeerStore::load(dir.path()).unwrap().list().is_empty());
    }
}
