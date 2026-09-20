//! Starting `owl_core::Engine` and keeping the interface told about it.
//!
//! The engine is started in the background rather than in `setup`, because
//! opening a folder with a large index takes longer than the first paint should
//! wait for. Everything that needs the engine goes through [`EngineHandle`],
//! which waits for the start to finish rather than failing while it is still
//! running: a command sent by the first render would otherwise lose that race.
//!
//! A start that fails is not a silent no-op. The port being taken by a second
//! copy of the app is the likely cause, and an app that shows an empty folder
//! and never says why is the worst possible answer to it. The failure is kept
//! here, emitted as a `state` with the message in `errors`, and returned by
//! `get_state` afterwards, so the interface has something to draw and the
//! person has something to read.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use owl_core::{Config, DeviceInfo, Engine, State, SyncSummary, TransferSummary};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{watch, Mutex};

/// The shortest gap between two `state` events.
///
/// The engine republishes its state as often as a transfer moves, which is many
/// times a second. The interface redraws the whole picture from each one and
/// cannot use more than this, so the extra snapshots would only queue up in
/// front of the webview.
const STATE_INTERVAL: Duration = Duration::from_millis(100);

/// The engine, once it has started, or why it did not.
#[derive(Clone)]
enum Startup {
    Running(Engine),
    Failed(Arc<Failure>),
}

/// A start that did not happen: what to tell the person, and the state the
/// interface draws in place of a real one.
struct Failure {
    message: String,
    state: State,
}

/// What every command reaches the engine through.
pub struct EngineHandle {
    app: AppHandle,
    /// Kept so that a start that failed can be tried again. See `set_paused`.
    config: Config,
    /// Where `settings.json` lives, so a command that changes a setting can
    /// write it back.
    pub data_dir: PathBuf,
    started: watch::Sender<Option<Startup>>,
    /// Held for the length of a start, so two of them cannot run at once and
    /// find the port taken by each other.
    starting: Mutex<()>,
}

impl EngineHandle {
    pub fn new(app: AppHandle, config: Config) -> Self {
        let (started, _) = watch::channel(None);
        Self {
            app,
            data_dir: config.data_dir.clone(),
            config,
            started,
            starting: Mutex::new(()),
        }
    }

    /// Starts the engine and returns at once. Everything that needs it waits.
    pub fn start(&self) {
        let app = self.app.clone();
        let started = self.started.clone();
        let config = self.config.clone();
        tauri::async_runtime::spawn(async move {
            let _ = attempt(app, started, config).await;
        });
    }

    /// The engine, waiting for the start to finish if it has not.
    pub async fn engine(&self) -> Result<Engine, String> {
        match self.startup().await {
            Some(Startup::Running(engine)) => Ok(engine),
            Some(Startup::Failed(failure)) => Err(failure.message.clone()),
            None => Err("the engine is not running".to_string()),
        }
    }

    /// The snapshot to draw, which is a real one when the engine started and
    /// the failure otherwise.
    pub async fn state(&self) -> Result<State, String> {
        match self.startup().await {
            Some(Startup::Running(engine)) => Ok(engine.state()),
            Some(Startup::Failed(failure)) => Ok(failure.state.clone()),
            None => Err("the engine is not running".to_string()),
        }
    }

    /// Stops or restarts watching, scanning and syncing.
    ///
    /// Resuming also starts an engine whose own start failed. On Android the
    /// reason it failed is almost always that the sync folder could not be
    /// created, because it is in shared storage and the person had not granted
    /// all files access yet, and this is called the moment they do. Without
    /// this the app would have to be killed and reopened to use the permission
    /// it had just been given.
    pub async fn set_paused(&self, paused: bool) -> Result<(), String> {
        if let Some(Startup::Running(engine)) = self.startup().await {
            engine.set_paused(paused).await;
            return Ok(());
        }

        let failure = match self.startup().await {
            Some(Startup::Failed(failure)) => failure,
            // Nothing to pause, and nothing a retry could fix.
            _ => return Err("the engine is not running".to_string()),
        };
        if paused {
            return Err(failure.message.clone());
        }

        let _guard = self.starting.lock().await;
        // Another caller may have got here first while this one waited.
        if let Some(Startup::Running(engine)) = self.startup().await {
            engine.set_paused(false).await;
            return Ok(());
        }
        let config = Config {
            paused: false,
            ..self.config.clone()
        };
        attempt(self.app.clone(), self.started.clone(), config).await
    }

    /// The engine if it has already started, without waiting for one that has
    /// not. Used on the way out, where there is nothing left to wait for.
    pub fn running(&self) -> Option<Engine> {
        match self.started.borrow().as_ref() {
            Some(Startup::Running(engine)) => Some(engine.clone()),
            _ => None,
        }
    }

    async fn startup(&self) -> Option<Startup> {
        let mut rx = self.started.subscribe();
        loop {
            // Cloned out of the guard rather than held across the await: a
            // watch borrow blocks the sender.
            let current = rx.borrow_and_update().as_ref().cloned();
            if current.is_some() {
                return current;
            }
            rx.changed().await.ok()?;
        }
    }
}

/// One attempt at starting, recording how it went either way.
async fn attempt(
    app: AppHandle,
    started: watch::Sender<Option<Startup>>,
    config: Config,
) -> Result<(), String> {
    // Built before the config moves, so a failure has a folder and a device
    // name to show rather than blanks.
    let blank = blank_state(&config);

    match Engine::start(config).await {
        Ok(engine) => {
            tracing::info!(port = engine.local_port(), "the engine is running");
            forward_state(app.clone(), engine.clone());
            forward_dirs(app, engine.clone());
            // `send_replace` and not `send`. `send` refuses, and throws the
            // value away, while no receiver exists, which is the normal case
            // here: the engine usually finishes starting before the interface
            // has asked it anything. Every command after that would then wait
            // for a result that had already been dropped.
            started.send_replace(Some(Startup::Running(engine)));
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            tracing::error!(%message, "the engine did not start");
            let state = State {
                paused: true,
                errors: vec![format!("Owl Transfer could not start: {message}")],
                ..blank
            };
            // Recorded before the event, so a `get_state` racing it gets the
            // same answer the event carries.
            started.send_replace(Some(Startup::Failed(Arc::new(Failure {
                message: message.clone(),
                state: state.clone(),
            }))));
            if let Err(error) = app.emit("state", &state) {
                tracing::error!(%error, "could not tell the interface why");
            }
            Err(message)
        }
    }
}

/// The whole snapshot, on every change, at most ten times a second.
fn forward_state(app: AppHandle, engine: Engine) {
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let state = rx.borrow_and_update().clone();
            if let Err(error) = app.emit("state", &state) {
                // Serialising a State cannot fail on a value the engine built,
                // so this is the webview going away rather than a bad value.
                tracing::warn!(%error, "the state event stopped being delivered");
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
            tokio::time::sleep(STATE_INTERVAL).await;
        }
    });
}

/// Which directory listings went stale. `""` is the sync folder itself.
fn forward_dirs(app: AppHandle, engine: Engine) {
    let mut rx = engine.dir_events();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(dir) => {
                    let _ = app.emit("dir-changed", &dir);
                }
                Err(RecvError::Lagged(missed)) => {
                    // Which directories went by is exactly what was lost, so
                    // the root stands in for them: the interface reloads
                    // whatever it is showing when the root changes.
                    tracing::debug!(
                        missed,
                        "directory events were dropped, reloading everything"
                    );
                    let _ = app.emit("dir-changed", "");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// A state with nothing in it but what the configuration already said.
fn blank_state(config: &Config) -> State {
    State {
        device: DeviceInfo {
            // Empty rather than invented: the real one is the hash of a
            // certificate that was never loaded.
            id: String::new(),
            name: config.device_name.clone(),
            kind: config.kind,
            port: config.tcp_port,
            addresses: Vec::new(),
        },
        folder: config.folder.display().to_string(),
        paused: config.paused,
        peers: Vec::new(),
        nearby: Vec::new(),
        pending_pairing: None,
        transfers: TransferSummary::default(),
        summary: SyncSummary::default(),
        errors: Vec::new(),
    }
}
