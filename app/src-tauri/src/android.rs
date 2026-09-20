//! The `owl` mobile plugin: the things only Kotlin can answer or do.
//!
//! The sync folder is `/storage/emulated/0/OwlTransfer`, in shared storage, so
//! that every file manager and gallery on the phone can see what was synced.
//! Reaching it needs `MANAGE_EXTERNAL_STORAGE`, which no runtime permission
//! dialog grants: the user has to turn it on from a system settings screen. So
//! the interface has to be able to ask whether it is on, and to send the person
//! to that screen. Both live in `OwlPlugin.kt`; this is the Rust half.
//!
//! The whole module is Android only. See `lib.rs` for where it is registered.

use serde::Deserialize;
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime,
};

/// The Java package `OwlPlugin.kt` is in.
const PLUGIN_IDENTIFIER: &str = "com.owltransfer.app";

#[derive(Deserialize)]
struct PermissionResponse {
    state: String,
}

#[derive(Deserialize)]
struct DisplayNameResponse {
    /// Absent rather than null when there is no name: putting a null into a
    /// JSONObject on the Kotlin side removes the key.
    #[serde(default)]
    name: Option<String>,
}

#[derive(serde::Serialize)]
struct UriArgs<'a> {
    uri: &'a str,
}

#[derive(serde::Serialize)]
struct PathArgs<'a> {
    path: &'a str,
}

#[derive(serde::Serialize)]
struct ThemeArgs {
    dark: bool,
}

pub struct Owl<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> Owl<R> {
    /// "granted" or "denied", or an error when the plugin call itself failed.
    /// A caller should read that error as "denied": a phone that cannot answer
    /// has not granted anything.
    pub fn all_files_permission(&self) -> anyhow::Result<String> {
        let response: PermissionResponse = self.0.run_mobile_plugin("allFilesPermission", ())?;
        Ok(response.state)
    }

    /// The name the sending application published for a `content://` URI, if
    /// it published one.
    ///
    /// The file picker hands the interface a URI and nothing else, and a URI
    /// carries no name: the name lives in the content resolver, which only
    /// Kotlin can ask. Without this every file picked on a phone would land
    /// under a timestamp.
    pub fn display_name(&self, uri: &str) -> anyhow::Result<Option<String>> {
        let response: DisplayNameResponse =
            self.0.run_mobile_plugin("displayName", UriArgs { uri })?;
        Ok(response.name)
    }

    /// Opens a file in the sync folder with whatever application handles it.
    ///
    /// The opener plugin cannot: its Android side reads the string it is given
    /// as a URI, and an absolute path has no scheme, while a `file://` one
    /// throws `FileUriExposedException`. Kotlin turns the path into a
    /// `FileProvider` content URI, which is a thing another application is
    /// allowed to read, and starts a chooser on it.
    pub fn open_path(&self, path: &str) -> anyhow::Result<()> {
        let _: serde_json::Value = self.0.run_mobile_plugin("openPath", PathArgs { path })?;
        Ok(())
    }

    /// Sets the window background to the colour the page is drawn on.
    ///
    /// The web layer is padded in by the system bar insets, so what is left
    /// behind the status bar and the gesture handle is the window background.
    /// Only the page knows which theme is in force, because the person can
    /// choose one that is not the system's.
    pub fn set_window_theme(&self, dark: bool) -> anyhow::Result<()> {
        let _: serde_json::Value = self
            .0
            .run_mobile_plugin("setWindowTheme", ThemeArgs { dark })?;
        Ok(())
    }

    /// Opens the system screen that grants all files access. It returns as soon
    /// as the screen is asked for, not when the person decides, so the caller
    /// asks again for the permission when the app comes back to the front.
    pub fn open_all_files_settings(&self) -> anyhow::Result<()> {
        // The command resolves with an empty object rather than nothing, so the
        // response is read as JSON and thrown away.
        let _: serde_json::Value = self.0.run_mobile_plugin("openAllFilesSettings", ())?;
        Ok(())
    }
}

/// `app.owl()` from anywhere that can reach the app handle.
pub trait OwlExt<R: Runtime> {
    fn owl(&self) -> &Owl<R>;
}

impl<R: Runtime, T: Manager<R>> OwlExt<R> for T {
    fn owl(&self) -> &Owl<R> {
        self.state::<Owl<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("owl")
        .setup(|app, api| {
            let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "OwlPlugin")?;
            app.manage(Owl(handle));
            Ok(())
        })
        .build()
}
