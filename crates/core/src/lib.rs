//! The Owl Transfer engine: identity, discovery, TLS transport, index,
//! change detection and sync, with one `Engine` type as the public surface.
//! See docs/ARCHITECTURE.md for the model this implements.

pub mod clock;
pub mod config;
pub mod identity;
pub mod ignore;
pub mod index;
pub mod pairing;
pub mod paths;
pub mod peers;
pub mod state;
pub mod vv;

pub use config::{Config, DeviceKind};
pub use state::{
    DeviceInfo, DirEntry, EntryStatus, NearbyInfo, PairingDirection, PairingInfo, PeerInfo, State,
    SyncSummary, Transfer, TransferDirection, TransferSummary,
};
