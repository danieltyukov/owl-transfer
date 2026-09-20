//! Engine configuration, supplied by the application shell.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The TCP port peers connect to.
pub const DEFAULT_TCP_PORT: u16 = 52734;
/// The UDP port the discovery beacon uses.
pub const DEFAULT_BEACON_PORT: u16 = 52735;

/// What kind of device this is. It only affects how peers show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Desktop,
    Phone,
}

/// Everything the engine needs to start.
#[derive(Clone, Debug)]
pub struct Config {
    /// Where device.json, peers.json and index.json live.
    pub data_dir: PathBuf,
    /// The sync folder. Created if missing.
    pub folder: PathBuf,
    pub device_name: String,
    pub kind: DeviceKind,
    /// 52734 in production; 0 asks for any free port (tests).
    pub tcp_port: u16,
    /// 52735 in production; 0 disables the beacon (tests).
    pub beacon_port: u16,
    /// Add a two second `PollWatcher` beside the native watcher (Android).
    pub poll_watch: bool,
    /// Start paused (Android before the storage permission is granted).
    pub paused: bool,
}
