//! One command per method of the interface's `Backend` contract.
//!
//! Every one of them waits for the engine through [`EngineHandle`] and turns an
//! `anyhow::Error` into the string the interface shows. Nothing here decides
//! anything: the rules live in the engine, and what is left over is the part
//! that is a property of the platform rather than of syncing, which is why
//! `open_entry`, `reveal_folder` and the permission commands have two bodies.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use owl_core::{DirEntry, Engine, State as EngineState};
use tauri::{AppHandle, State};
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

use crate::engine::EngineHandle;
use crate::settings::Settings;

/// Everything the interface can be told, which is a value or a sentence.
type Answer<T> = Result<T, String>;

/// `{e:#}` rather than `to_string`, so the context the engine attached ("
/// creating /home/you/OwlTransfer: permission denied") arrives with it.
fn failed(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// Runs something that blocks the thread it is on, off the async runtime.
///
/// The calls that need this are the ones that leave Rust: a mobile plugin call
/// is a round trip through JNI that waits on the Android UI thread, and the
/// desktop opener hands over to the system file manager. Holding a runtime
/// worker while either happens is how a phone with one core stops answering.
async fn blocking<T, F>(work: F) -> Answer<T>
where
    F: FnOnce() -> Answer<T> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("the call did not finish: {error}"))?
}

/// Which shell this is: "android" or "desktop".
///
/// Asked of the shell rather than read from a build time variable. The Tauri
/// CLI sets `TAURI_ENV_PLATFORM`, but it never reaches `import.meta.env`: Vite
/// matches `envPrefix` as a literal string, and the template's `TAURI_ENV_*`
/// is a prefix no variable starts with. The interface used it to decide
/// whether to draw a title bar, and drew the desktop one on the phone.
#[tauri::command]
pub fn platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else {
        "desktop"
    }
}

#[tauri::command]
pub async fn get_state(handle: State<'_, EngineHandle>) -> Answer<EngineState> {
    handle.state().await
}

#[tauri::command]
pub async fn list_dir(path: String, handle: State<'_, EngineHandle>) -> Answer<Vec<DirEntry>> {
    handle.engine().await?.list_dir(&path).await.map_err(failed)
}

/// Copies picked or dropped files into `into`, and returns how many landed.
///
/// A desktop dialog and a drop both hand over paths on disk, which the engine
/// copies itself. Android hands over `content://` URIs instead, which only this
/// process can read and only through the fs plugin, so each one is opened here
/// and streamed in. The two kinds arrive mixed in one call because the dialog
/// decides which it gives, not the caller.
#[tauri::command]
pub async fn import_paths(
    paths: Vec<FilePath>,
    into: String,
    names: Option<Vec<String>>,
    app: AppHandle,
    handle: State<'_, EngineHandle>,
) -> Answer<u32> {
    let engine = handle.engine().await?;
    let mut on_disk: Vec<PathBuf> = Vec::new();
    let mut streams: Vec<(FilePath, Option<String>)> = Vec::new();

    for (index, picked) in paths.into_iter().enumerate() {
        let given = || names.as_ref().and_then(|list| list.get(index)).cloned();
        match picked {
            FilePath::Path(path) => on_disk.push(path),
            // A file:// URL is a path this process can open like any other, and
            // the engine keeps the name and the modification time when it
            // copies one. Anything else is an Android content:// URI.
            FilePath::Url(url) => match url.to_file_path() {
                Ok(path) => on_disk.push(path),
                Err(()) => streams.push((FilePath::Url(url), given())),
            },
        }
    }

    // The batch goes first, so that a stream that cannot be opened does not
    // throw away the files picked beside it in the same call.
    let mut imported = if on_disk.is_empty() {
        0
    } else {
        engine.import_files(on_disk, &into).await.map_err(failed)?
    };

    for (stream, given) in streams {
        let uri = stream.to_string();
        let name = picked_name(&app, &uri, given.as_deref()).await;
        let file = open_picked(&app, stream).await?;
        engine
            .import_reader(&name, &into, tokio::fs::File::from_std(file))
            .await
            .map_err(failed)?;
        imported += 1;
    }

    Ok(imported)
}

#[tauri::command]
pub async fn create_folder(path: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    handle
        .engine()
        .await?
        .create_folder(&path)
        .await
        .map_err(failed)
}

#[tauri::command]
pub async fn delete_entry(path: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    handle
        .engine()
        .await?
        .delete_entry(&path)
        .await
        .map_err(failed)
}

#[tauri::command]
pub async fn rename_entry(
    path: String,
    new_name: String,
    handle: State<'_, EngineHandle>,
) -> Answer<()> {
    handle
        .engine()
        .await?
        .rename_entry(&path, &new_name)
        .await
        .map_err(failed)
}

/// Hands the file to whatever the system opens it with.
#[tauri::command]
pub async fn open_entry(
    path: String,
    app: AppHandle,
    handle: State<'_, EngineHandle>,
) -> Answer<()> {
    let absolute = handle
        .engine()
        .await?
        .absolute_path(&path)
        .map_err(failed)?;
    open_absolute(app, absolute).await
}

/// On Android the opener plugin cannot do this.
///
/// Its mobile side builds an `ACTION_VIEW` out of the string it is given read
/// as a URI, and an absolute filesystem path has no scheme at all, while a
/// `file://` one throws `FileUriExposedException` on Android 7 and later. The
/// `owl` plugin hands out a `FileProvider` content URI instead, which is what
/// every other application on the phone is allowed to read.
#[cfg(target_os = "android")]
async fn open_absolute(app: AppHandle, path: PathBuf) -> Answer<()> {
    use crate::android::OwlExt;

    blocking(move || app.owl().open_path(&path.to_string_lossy()).map_err(failed)).await
}

#[cfg(not(target_os = "android"))]
async fn open_absolute(app: AppHandle, path: PathBuf) -> Answer<()> {
    use tauri_plugin_opener::OpenerExt;

    blocking(move || {
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|error| error.to_string())
    })
    .await
}

/// Shows one entry where it lives, in the system's file manager.
///
/// The path is relative and goes through the engine, which is what rejects
/// `..`, an absolute path and an empty component before any of it reaches a
/// system that would happily open whatever it was handed.
#[tauri::command]
pub async fn reveal_entry(
    path: String,
    app: AppHandle,
    handle: State<'_, EngineHandle>,
) -> Answer<()> {
    let absolute = handle
        .engine()
        .await?
        .absolute_path(&path)
        .map_err(failed)?;
    reveal_item(app, absolute).await
}

/// Opens a directory itself: the sync folder, or one under it.
#[tauri::command]
pub async fn reveal_folder(
    path: Option<String>,
    app: AppHandle,
    handle: State<'_, EngineHandle>,
) -> Answer<()> {
    let absolute = handle
        .engine()
        .await?
        .absolute_path(path.as_deref().unwrap_or_default())
        .map_err(failed)?;
    reveal_dir(app, absolute).await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn reveal_item(_app: AppHandle, path: PathBuf) -> Answer<()> {
    blocking(move || {
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
    })
    .await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn reveal_dir(app: AppHandle, dir: PathBuf) -> Answer<()> {
    use tauri_plugin_opener::OpenerExt;

    // `open_path` and not `reveal_item_in_dir`: revealing a directory opens its
    // parent with it selected, and what was asked for is the directory.
    blocking(move || {
        app.opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|error| error.to_string())
    })
    .await
}

/// Android's document picker has no selection to ask for, so the closest it
/// gets is opening the folder the entry is in.
#[cfg(any(target_os = "android", target_os = "ios"))]
async fn reveal_item(app: AppHandle, path: PathBuf) -> Answer<()> {
    let folder = path
        .parent()
        .map_or(path.clone(), |parent| parent.to_path_buf());
    reveal_dir(app, folder).await
}

/// The sync folder is in shared storage, which every file manager on the phone
/// can already see. `OwlPlugin` asks the documents provider for it by name.
#[cfg(target_os = "android")]
async fn reveal_dir(app: AppHandle, dir: PathBuf) -> Answer<()> {
    use crate::android::OwlExt;

    blocking(move || {
        app.owl()
            .open_folder(&dir.to_string_lossy())
            .map_err(failed)
    })
    .await
}

#[cfg(target_os = "ios")]
async fn reveal_dir(_app: AppHandle, _dir: PathBuf) -> Answer<()> {
    Ok(())
}

/// Moves the sync folder, and remembers where it went.
#[tauri::command]
pub async fn set_folder(path: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    let engine = handle.engine().await?;
    engine
        .set_folder(PathBuf::from(path))
        .await
        .map_err(failed)?;
    remember(&handle, &engine).await
}

#[tauri::command]
pub async fn set_device_name(name: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    let engine = handle.engine().await?;
    engine.set_device_name(name).await.map_err(failed)?;
    remember(&handle, &engine).await
}

#[tauri::command]
pub async fn pair_with_nearby(id: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    handle
        .engine()
        .await?
        .pair_with_nearby(&id)
        .await
        .map_err(failed)
}

#[tauri::command]
pub async fn pair_with_address(
    host: String,
    port: u16,
    handle: State<'_, EngineHandle>,
) -> Answer<()> {
    handle
        .engine()
        .await?
        .pair_with_address(&host, port)
        .await
        .map_err(failed)
}

#[tauri::command]
pub async fn respond_to_pairing(
    id: String,
    accept: bool,
    handle: State<'_, EngineHandle>,
) -> Answer<()> {
    handle
        .engine()
        .await?
        .respond_to_pairing(&id, accept)
        .await
        .map_err(failed)
}

#[tauri::command]
pub async fn forget_peer(id: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    handle
        .engine()
        .await?
        .forget_peer(&id)
        .await
        .map_err(failed)
}

/// Whether Android has granted all files access. "not-applicable" everywhere
/// else, which is what makes the interface leave the card out.
#[tauri::command]
pub async fn all_files_permission(app: AppHandle) -> Answer<String> {
    #[cfg(target_os = "android")]
    {
        blocking(move || {
            use crate::android::OwlExt;

            // A phone that cannot answer has not granted anything.
            Ok(app.owl().all_files_permission().unwrap_or_else(|error| {
                tracing::warn!(%error, "could not read the storage permission");
                "denied".to_string()
            }))
        })
        .await
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok("not-applicable".to_string())
    }
}

/// Opens the system screen that grants all files access.
#[tauri::command]
pub async fn open_all_files_settings(app: AppHandle) -> Answer<()> {
    #[cfg(target_os = "android")]
    {
        blocking(move || {
            use crate::android::OwlExt;

            app.owl().open_all_files_settings().map_err(failed)
        })
        .await
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(())
    }
}

/// Tells the shell what colour the page is drawn on.
///
/// Android only. The web layer is padded in by the system bar insets, so the
/// strips that leaves show the window background, and that background follows
/// the system's dark mode while the page follows the person's own choice. They
/// disagree the moment someone picks Light on a phone that is in dark mode, and
/// the result is a dark band above and below a light page.
#[tauri::command]
pub async fn set_window_theme(dark: bool, app: AppHandle) -> Answer<()> {
    #[cfg(target_os = "android")]
    {
        blocking(move || {
            use crate::android::OwlExt;

            app.owl().set_window_theme(dark).map_err(failed)
        })
        .await
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, dark);
        Ok(())
    }
}

/// Stops or restarts watching, scanning and syncing.
///
/// Android starts paused, because the folder is out of reach until the person
/// grants all files access. The permission card calls this with `false` when it
/// sees the grant, and resuming also starts an engine whose own start failed:
/// see `EngineHandle::set_paused` for why that is the same moment.
#[tauri::command]
pub async fn set_paused(paused: bool, handle: State<'_, EngineHandle>) -> Answer<()> {
    handle.set_paused(paused).await
}

/// Writes the two settings the engine holds back to `settings.json`.
///
/// Both of them live in the engine while the app runs, so this takes them from
/// there rather than from the argument: whatever the engine accepted is what
/// should be on disk. `MainActivity` reads the same file to find the folder a
/// shared file goes into, so this is what keeps the two in step, and a write
/// that failed is the command failing: the alternative is an interface showing
/// the new folder while every shared file keeps going to the old one.
async fn remember(handle: &EngineHandle, engine: &Engine) -> Answer<()> {
    let settings = Settings {
        folder: engine.folder(),
        device_name: engine.state().device.name,
    };
    let dir = handle.data_dir.clone();
    blocking(move || {
        settings.save(&dir).map_err(|error| {
            format!(
                "the change was made but could not be written to {}: {error:#}",
                dir.display()
            )
        })
    })
    .await
}

/// Opens a picked file, whatever kind of reference the dialog gave for it.
async fn open_picked(app: &AppHandle, path: FilePath) -> Answer<std::fs::File> {
    let app = app.clone();
    blocking(move || {
        let mut options = OpenOptions::new();
        options.read(true);
        app.fs()
            .open(path, options)
            .map_err(|error| format!("opening the picked file: {error}"))
    })
    .await
}

/// The name a picked file keeps.
///
/// A `content://` URI carries no name, so the one the sending app published is
/// asked for through the `owl` plugin, the same column `MainActivity` reads for
/// a share. When there is no answer the file still has to land somewhere, and
/// a timestamp is a name a person can at least find.
async fn picked_name(app: &AppHandle, uri: &str, given: Option<&str>) -> String {
    if let Some(name) = given.and_then(usable_name) {
        return name;
    }
    if let Some(name) = display_name(app, uri)
        .await
        .as_deref()
        .and_then(usable_name)
    {
        return name;
    }
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    format!("shared-{millis}")
}

/// The last segment of `raw`, or nothing when that is not a name a file can
/// have. The engine rejects the rest, and an import that fails over a name the
/// person never typed is worse than one that lands under a made up name.
fn usable_name(raw: &str) -> Option<String> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(name.to_string())
}

#[cfg(target_os = "android")]
async fn display_name(app: &AppHandle, uri: &str) -> Option<String> {
    use crate::android::OwlExt;

    let app = app.clone();
    let uri = uri.to_string();
    match tauri::async_runtime::spawn_blocking(move || app.owl().display_name(&uri)).await {
        Ok(Ok(name)) => name,
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not read the picked file's name");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "the name lookup did not finish");
            None
        }
    }
}

#[cfg(not(target_os = "android"))]
async fn display_name(_app: &AppHandle, _uri: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::usable_name;

    #[test]
    fn a_name_is_the_last_segment_of_what_was_given() {
        assert_eq!(usable_name("photo.jpg").as_deref(), Some("photo.jpg"));
        assert_eq!(usable_name("a/b/photo.jpg").as_deref(), Some("photo.jpg"));
        assert_eq!(usable_name("a\\b\\photo.jpg").as_deref(), Some("photo.jpg"));
    }

    #[test]
    fn a_name_that_is_not_one_is_refused() {
        assert_eq!(usable_name(""), None);
        assert_eq!(usable_name("   "), None);
        assert_eq!(usable_name("."), None);
        assert_eq!(usable_name(".."), None);
        assert_eq!(usable_name("a/b/"), None);
    }
}
