//! Where the app keeps its own two settings, and where everything else lives.
//!
//! `settings.json` sits in the application's data directory beside the engine's
//! `device.json`, `peers.json` and `index.json`. It holds the two things a
//! person can change that the engine needs before it starts:
//!
//! ```json
//! { "folder": "/home/you/OwlTransfer", "device_name": "your-machine" }
//! ```
//!
//! On Android `MainActivity` reads the same file to find out where to drop a
//! file shared to the app, so the shape of it is a contract with the Kotlin
//! side, not a private detail.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// The TCP port peers connect on, and the UDP port the beacon uses.
pub const DEFAULT_TCP_PORT: u16 = 52734;
pub const DEFAULT_BEACON_PORT: u16 = 52735;

/// The folder on Android. It is under shared storage on purpose, so that every
/// file manager and gallery on the phone can see what was synced.
#[cfg(target_os = "android")]
const ANDROID_FOLDER: &str = "/storage/emulated/0/OwlTransfer";

const FILE: &str = "settings.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub folder: PathBuf,
    pub device_name: String,
}

impl Settings {
    /// The stored settings, or the defaults for this platform when there are
    /// none yet. Environment overrides win over both, so that two instances can
    /// run side by side on one machine.
    pub fn load<R: Runtime>(app: &AppHandle<R>, dir: &Path) -> Self {
        let mut settings = read(dir).unwrap_or_else(|| defaults(app));
        if let Some(folder) = std::env::var_os("OWL_FOLDER") {
            settings.folder = PathBuf::from(folder);
        }
        settings
    }

    /// The settings for this run, writing the file out on first run.
    ///
    /// The file has to exist for `MainActivity` to read the sync folder out of
    /// it, and a person looking for where their folder is configured should
    /// find a file rather than nothing. An environment override is applied to
    /// the returned value but never written: it belongs to one run.
    pub fn load_or_create<R: Runtime>(app: &AppHandle<R>, dir: &Path) -> Self {
        if read(dir).is_none() {
            let fresh = defaults(app);
            if let Err(error) = fresh.save(dir) {
                tracing::warn!(dir = %dir.display(), %error, "could not write settings.json");
            }
        }
        Self::load(app, dir)
    }

    /// Writes the file through a temporary name, because the Kotlin side reads
    /// it without locking and a half written file would look like a corrupt one.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, dir.join(FILE))?;
        Ok(())
    }
}

fn read(dir: &Path) -> Option<Settings> {
    let path = dir.join(FILE);
    let raw = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&raw) {
        Ok(settings) => Some(settings),
        Err(error) => {
            // Not fatal: a person who hand edited the file into something
            // unparseable gets the defaults back rather than an app that will
            // not start. The defaults are then written over it at the next
            // startup, so the broken file is moved aside first. It is the only
            // record of which folder they had chosen, and one typo should not
            // cost them that silently.
            let kept = set_aside(&path);
            tracing::warn!(
                path = %path.display(),
                %error,
                kept = %kept.as_deref().unwrap_or("nothing, the rename failed too"),
                "settings.json could not be parsed, keeping a copy and using the defaults"
            );
            None
        }
    }
}

/// Renames an unparseable settings file out of the way, returning where it went.
fn set_aside(path: &Path) -> Option<String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    let kept = path.with_file_name(format!("{FILE}.corrupt-{stamp}"));
    std::fs::rename(path, &kept).ok()?;
    Some(kept.display().to_string())
}

/// What a device with no `settings.json` yet starts out with.
pub fn defaults<R: Runtime>(app: &AppHandle<R>) -> Settings {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        Settings {
            folder: PathBuf::from(ANDROID_FOLDER),
            device_name: "Android phone".to_string(),
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let home = app
            .path()
            .home_dir()
            .unwrap_or_else(|_| std::env::temp_dir());
        Settings {
            folder: home.join("OwlTransfer"),
            device_name: hostname().unwrap_or_else(|| "This computer".to_string()),
        }
    }
}

/// The directory holding `settings.json` and everything the engine persists.
///
/// `OWL_DATA_DIR` exists so two instances can run on one machine during
/// development; see CONTRIBUTING.
pub fn data_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Some(dir) = std::env::var_os("OWL_DATA_DIR") {
        return PathBuf::from(dir);
    }

    let dir = app
        .path()
        .app_data_dir()
        .expect("this platform has no application data directory");

    // On Android `app_data_dir()` is the application's data directory, one
    // level above what Kotlin calls `filesDir`. MainActivity reads the sync
    // folder out of `filesDir/settings.json`, so the Rust side has to write it
    // to the same place.
    #[cfg(target_os = "android")]
    let dir = dir.join("files");

    dir
}

/// The TCP and UDP beacon ports, with the same development overrides.
pub fn ports() -> (u16, u16) {
    (
        port("OWL_PORT", DEFAULT_TCP_PORT),
        port("OWL_BEACON_PORT", DEFAULT_BEACON_PORT),
    )
}

fn port(name: &str, fallback: u16) -> u16 {
    parse_port(std::env::var(name).ok().as_deref(), name, fallback)
}

fn parse_port(value: Option<&str>, name: &str, fallback: u16) -> u16 {
    match value {
        None => fallback,
        Some(raw) => raw.parse().unwrap_or_else(|_| {
            tracing::warn!(%name, %raw, "not a port number, using {fallback}");
            fallback
        }),
    }
}

/// The machine's name, for the default device name.
///
/// Read from the OS rather than from `$HOSTNAME`, which is a shell variable and
/// is not exported to an application started from a launcher.
#[cfg(not(target_os = "android"))]
fn hostname() -> Option<String> {
    #[cfg(target_os = "windows")]
    let raw = std::env::var("COMPUTERNAME").ok();

    #[cfg(not(target_os = "windows"))]
    let raw = std::fs::read_to_string("/etc/hostname").ok();

    raw.map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_settings_are_read_back() {
        let dir = std::env::temp_dir().join(format!("owl-settings-{}", std::process::id()));
        let settings = Settings {
            folder: PathBuf::from("/tmp/owl"),
            device_name: "a name".to_string(),
        };

        settings.save(&dir).expect("save");
        assert_eq!(read(&dir), Some(settings));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert_eq!(read(Path::new("/nowhere/at/all")), None);
    }

    #[test]
    fn an_unparseable_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("owl-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create");
        std::fs::write(dir.join(FILE), b"{ folder: oops").expect("write");

        assert_eq!(read(&dir), None);
        assert!(
            !dir.join(FILE).exists(),
            "the broken file was left in place"
        );

        let kept: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept.len(), 1, "expected exactly one file, found {kept:?}");
        assert!(
            kept[0].starts_with("settings.json.corrupt-"),
            "unexpected name {}",
            kept[0]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_port_that_is_not_a_number_falls_back() {
        assert_eq!(parse_port(None, "OWL_PORT", 52734), 52734);
        assert_eq!(parse_port(Some("1234"), "OWL_PORT", 52734), 1234);
        assert_eq!(parse_port(Some("http"), "OWL_PORT", 52734), 52734);
    }
}
