//! One command per method of the interface's `Backend` contract.
//!
//! Every one of them waits for the engine through [`EngineHandle`] and turns an
//! `anyhow::Error` into the string the interface shows. Nothing here decides
//! anything: the rules live in the engine, and what is left over is the part
//! that is a property of the platform rather than of syncing, which is why
//! `reveal_folder` and the two permission commands have two bodies.

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
    let mut streamed = 0u32;

    for (index, picked) in paths.into_iter().enumerate() {
        let url = match picked {
            FilePath::Path(path) => {
                on_disk.push(path);
                continue;
            }
            FilePath::Url(url) => url,
        };
        // A file:// URL is a path this process can open like any other, and the
        // engine keeps the name and the modification time when it copies one.
        if let Ok(path) = url.to_file_path() {
            on_disk.push(path);
            continue;
        }

        let given = names.as_ref().and_then(|list| list.get(index));
        let name = picked_name(&app, url.as_str(), given.map(String::as_str));
        let file = open_picked(&app, FilePath::Url(url))?;
        engine
            .import_reader(&name, &into, tokio::fs::File::from_std(file))
            .await
            .map_err(failed)?;
        streamed += 1;
    }

    let copied = if on_disk.is_empty() {
        0
    } else {
        engine.import_files(on_disk, &into).await.map_err(failed)?
    };
    Ok(copied + streamed)
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
    use tauri_plugin_opener::OpenerExt;

    let absolute = handle
        .engine()
        .await?
        .absolute_path(&path)
        .map_err(failed)?;
    app.opener()
        .open_path(absolute.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}

/// Shows the sync folder in the system's file manager.
///
/// Android has no file manager to hand a path to, and the folder is in shared
/// storage where every one of them can already see it, so there it does nothing
/// rather than failing. The interface does not draw the button there either.
#[tauri::command]
pub async fn reveal_folder(handle: State<'_, EngineHandle>) -> Answer<()> {
    let engine = handle.engine().await?;

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    tauri_plugin_opener::reveal_item_in_dir(engine.folder()).map_err(|error| error.to_string())?;

    #[cfg(any(target_os = "android", target_os = "ios"))]
    let _ = engine;

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
    remember(&handle, &engine);
    Ok(())
}

#[tauri::command]
pub async fn set_device_name(name: String, handle: State<'_, EngineHandle>) -> Answer<()> {
    let engine = handle.engine().await?;
    engine.set_device_name(name).await.map_err(failed)?;
    remember(&handle, &engine);
    Ok(())
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
        use crate::android::OwlExt;
        // A phone that cannot answer has not granted anything.
        Ok(app.owl().all_files_permission().unwrap_or_else(|error| {
            tracing::warn!(%error, "could not read the storage permission");
            "denied".to_string()
        }))
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
        use crate::android::OwlExt;
        app.owl().open_all_files_settings().map_err(failed)
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(())
    }
}

/// Stops or restarts watching, scanning and syncing.
///
/// Android starts paused, because the folder is out of reach until the person
/// grants all files access. The permission card calls this with `false` when it
/// sees the grant.
#[tauri::command]
pub async fn set_paused(paused: bool, handle: State<'_, EngineHandle>) -> Answer<()> {
    handle.engine().await?.set_paused(paused).await;
    Ok(())
}

/// Writes the two settings the engine holds back to `settings.json`.
///
/// Both of them live in the engine while the app runs, so this takes them from
/// there rather than from the argument: whatever the engine accepted is what
/// should be on disk. `MainActivity` reads the same file to find the folder a
/// shared file goes into, so this is what keeps the two in step.
fn remember(handle: &EngineHandle, engine: &Engine) {
    let settings = Settings {
        folder: engine.folder(),
        device_name: engine.state().device.name,
    };
    if let Err(error) = settings.save(&handle.data_dir) {
        tracing::warn!(dir = %handle.data_dir.display(), %error, "could not save settings.json");
    }
}

/// Opens a picked file, whatever kind of reference the dialog gave for it.
fn open_picked(app: &AppHandle, path: FilePath) -> Result<std::fs::File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    app.fs()
        .open(path, options)
        .map_err(|error| format!("opening the picked file: {error}"))
}

/// The name a picked file keeps.
///
/// A `content://` URI carries no name, so the one the sending app published is
/// asked for through the `owl` plugin, the same column `MainActivity` reads for
/// a share. When there is no answer the file still has to land somewhere, and
/// a timestamp is a name a person can at least find.
fn picked_name(app: &AppHandle, uri: &str, given: Option<&str>) -> String {
    given
        .and_then(usable_name)
        .or_else(|| display_name(app, uri).as_deref().and_then(usable_name))
        .unwrap_or_else(|| {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_millis())
                .unwrap_or(0);
            format!("shared-{millis}")
        })
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
fn display_name(app: &AppHandle, uri: &str) -> Option<String> {
    use crate::android::OwlExt;

    match app.owl().display_name(uri) {
        Ok(name) => name,
        Err(error) => {
            tracing::warn!(%error, "could not read the picked file's name");
            None
        }
    }
}

#[cfg(not(target_os = "android"))]
fn display_name(_app: &AppHandle, _uri: &str) -> Option<String> {
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
