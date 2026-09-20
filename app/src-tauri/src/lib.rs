//! The Tauri shell: one window on the desktop, one activity on Android, and
//! the platform pieces the engine cannot provide for itself.
//!
//! The engine lives in `owl-core` and knows nothing about Tauri. This crate
//! decides where its data goes, starts it, and forwards its state to the
//! interface. The commands and that forwarding are Task D2; what is here is the
//! shell those hang off: the window, the plugins, the settings file and the
//! Android hooks.

#[cfg(target_os = "android")]
mod android;
mod settings;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAIN_WINDOW: &str = "main";

/// Brings the existing window back rather than opening another one.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn focus_main(app: &tauri::AppHandle) {
    // Imported here rather than at the top of the file: Android has no window
    // to focus, so on that target the import would be unused.
    use tauri::Manager;

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
        .setup(|app| {
            let handle = app.handle();
            let data_dir = settings::data_dir(handle);
            let current = settings::Settings::load_or_create(handle, &data_dir);
            let (tcp_port, beacon_port) = settings::ports();

            tracing::info!(
                data_dir = %data_dir.display(),
                folder = %current.folder.display(),
                device_name = %current.device_name,
                tcp_port,
                beacon_port,
                "Owl Transfer starting"
            );

            // Task D2 starts the engine here with this configuration and
            // forwards its state snapshots to the webview.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Owl Transfer");
}
