//! The Tauri shell: one window on the desktop, one activity on Android, and
//! the platform pieces the engine cannot provide for itself.
//!
//! The engine lives in `owl-core` and knows nothing about Tauri. This crate
//! decides where its data goes, starts it, forwards its state to the interface
//! and exposes one command per thing the interface can do. What is left is the
//! part the engine cannot do for itself: the window, the plugins, the settings
//! file and the Android hooks.

#[cfg(target_os = "android")]
mod android;
mod commands;
mod engine;
mod settings;

use owl_core::{Config, DeviceKind};
use tauri::Manager;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAIN_WINDOW: &str = "main";

/// Brings the existing window back rather than opening another one.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn focus_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Logs to logcat on Android and to the terminal everywhere else.
///
/// Android has no stdout worth writing to, and `android_logger` is a `log`
/// sink rather than a `tracing` subscriber. `tracing`'s `log` feature covers
/// the gap: with no tracing subscriber installed, every `tracing` event is
/// emitted as a `log` record, which `android_logger` then writes to logcat.
fn init_logging() {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("owl-transfer"),
    );

    #[cfg(not(target_os = "android"))]
    {
        use tracing_subscriber::EnvFilter;
        // OWL_LOG rather than RUST_LOG, so that turning this app up to debug
        // does not also turn up every other Rust program in the same shell.
        let filter = EnvFilter::try_from_env("OWL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

/// Sets the environment WebKitGTK needs before anything creates a webview.
///
/// On NVIDIA, WebKitGTK without `__NV_DISABLE_EXPLICIT_SYNC=1` hands back an
/// empty frame: the window comes up blank white with no error anywhere. The
/// packaged `.desktop` file sets it, but that only covers launches that go
/// through it, not `tauri dev` and not running the binary directly.
///
/// An explicit value in the environment is left alone: someone who set it to 0
/// on purpose, to see the failure or because a driver update fixed it, means it.
#[cfg(target_os = "linux")]
fn apply_webkit_workarounds() {
    if std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
        // Safe here and nowhere later: this runs before the builder, so no
        // other thread exists to observe the environment changing.
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_webkit_workarounds() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_webkit_workarounds();
    init_logging();

    let builder = tauri::Builder::default();

    // Single instance is registered first, and must stay first: registered
    // after another plugin it stops deduplicating, silently, and a second
    // launch opens a second copy watching and writing the same folder.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        focus_main(app);
    }));

    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(target_os = "android")]
    let builder = builder.plugin(android::init());

    builder
        .invoke_handler(tauri::generate_handler![
            commands::platform,
            commands::get_state,
            commands::list_dir,
            commands::import_paths,
            commands::create_folder,
            commands::delete_entry,
            commands::rename_entry,
            commands::open_entry,
            commands::reveal_folder,
            commands::set_folder,
            commands::set_device_name,
            commands::pair_with_nearby,
            commands::pair_with_address,
            commands::respond_to_pairing,
            commands::forget_peer,
            commands::all_files_permission,
            commands::open_all_files_settings,
            commands::set_paused,
        ])
        .setup(|app| {
            let handle = app.handle();
            let data_dir = settings::data_dir(handle);
            let current = settings::Settings::load_or_create(handle, &data_dir);
            let (tcp_port, beacon_port) = settings::ports();
            let paused = start_paused(handle);

            tracing::info!(
                data_dir = %data_dir.display(),
                folder = %current.folder.display(),
                device_name = %current.device_name,
                tcp_port,
                beacon_port,
                paused,
                "Owl Transfer starting"
            );

            let config = Config {
                data_dir,
                folder: current.folder,
                device_name: current.device_name,
                kind: if cfg!(target_os = "android") {
                    DeviceKind::Phone
                } else {
                    DeviceKind::Desktop
                },
                tcp_port,
                beacon_port,
                // Android's FUSE backed shared storage does not deliver inotify
                // events for writes made by other applications, so the watcher
                // needs a poll beside it there and nowhere else.
                poll_watch: cfg!(target_os = "android"),
                paused,
            };

            let engine_handle = engine::EngineHandle::new(handle.clone(), config);
            engine_handle.start();
            app.manage(engine_handle);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Owl Transfer")
        .run(|app, event| {
            // The index and the peer list are written here. Everything else the
            // engine holds is already on disk, but an index that was never
            // saved means the whole folder is rehashed at the next start.
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(engine) = app.state::<engine::EngineHandle>().running() {
                    tauri::async_runtime::block_on(engine.shutdown());
                }
            }
        });
}

/// Whether the engine starts paused.
///
/// On Android the sync folder is in shared storage, which is out of reach until
/// the person grants all files access on a system screen. Starting there
/// unpaused would mean a scanner failing on every file and an error list the
/// person can do nothing about, so it waits: the permission card calls
/// `set_paused(false)` when the grant arrives.
#[cfg(target_os = "android")]
fn start_paused(app: &tauri::AppHandle) -> bool {
    use android::OwlExt;

    match app.owl().all_files_permission() {
        Ok(state) => state != "granted",
        Err(error) => {
            tracing::warn!(%error, "could not read the storage permission, starting paused");
            true
        }
    }
}

#[cfg(not(target_os = "android"))]
fn start_paused(_app: &tauri::AppHandle) -> bool {
    false
}
