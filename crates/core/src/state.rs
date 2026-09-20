//! The state snapshot the interface renders. `State` is the whole picture in
//! one value; the shell forwards it as an event and nothing in the UI keeps
//! sync state of its own.
//!
//! JSON shape: every struct is `camelCase`; the enums are lowercase tags.
//!
//! How `EntryStatus` is chosen for a listing (`Engine::list_dir`): the engine
//! does not track acknowledgements from peers, so the rule is deliberately
//! simple and honest:
//!
//! - `Conflict` if the name matches the conflict copy pattern
//!   `name (conflict from <device> <date>).ext`.
//! - `Syncing { progress }` if a download or upload for the path is active.
//! - `Local` if no peer is connected: nothing else can have it yet.
//! - `Waiting` if the entry changed in the last two seconds or is queued for
//!   download, meaning a connected peer has not necessarily caught up.
//! - `Synced` otherwise.

use serde::{Deserialize, Serialize};

use crate::config::DeviceKind;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub device: DeviceInfo,
    pub folder: String,
    pub paused: bool,
    pub peers: Vec<PeerInfo>,
    pub nearby: Vec<NearbyInfo>,
    pub pending_pairing: Option<PairingInfo>,
    pub transfers: TransferSummary,
    pub summary: SyncSummary,
    /// The last five errors, newest last.
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub connected: bool,
    pub address: Option<String>,
    pub last_seen_ms: Option<i64>,
    pub paired_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyInfo {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Six digits with a space in the middle, for example "482 913".
    pub code: String,
    pub direction: PairingDirection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairingDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferSummary {
    pub active: Vec<Transfer>,
    pub queued: u32,
    pub bytes_per_sec: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub path: String,
    pub peer_id: String,
    pub direction: TransferDirection,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Download,
    Upload,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub last_change_ms: Option<i64>,
    pub up_to_date: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub status: EntryStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EntryStatus {
    Synced,
    Syncing {
        progress: f32,
    },
    Waiting,
    Conflict,
    /// No connected peer can have it yet.
    Local,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_matches_the_contract() {
        let s = EntryStatus::Syncing { progress: 0.5 };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"kind":"syncing","progress":0.5}"#
        );
        assert_eq!(
            serde_json::to_string(&EntryStatus::Local).unwrap(),
            r#"{"kind":"local"}"#
        );
        let t = Transfer {
            path: "a".into(),
            peer_id: "p".into(),
            direction: TransferDirection::Download,
            bytes_done: 1,
            bytes_total: 2,
        };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"path":"a","peerId":"p","direction":"download","bytesDone":1,"bytesTotal":2}"#
        );
        let p = PairingInfo {
            id: "i".into(),
            name: "n".into(),
            kind: DeviceKind::Phone,
            code: "482 913".into(),
            direction: PairingDirection::Incoming,
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"id":"i","name":"n","kind":"phone","code":"482 913","direction":"incoming"}"#
        );
    }
}
