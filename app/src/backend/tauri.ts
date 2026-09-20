import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';

import type {
  Backend,
  DirEntry,
  Permission,
  Platform,
  State,
  Unsubscribe,
  WindowFrame,
} from './types.js';

/*
 * The other implementation of `Backend`: the one with an engine under it.
 *
 * Every method here is one command or one event. There is no logic in this
 * file on purpose, because anything it decided would be a second opinion on a
 * question the engine has already answered, and the two would drift. What it
 * does own is the shape of the seam: the interface never sees an `invoke`, a
 * command name or an event name, which is what lets it be developed and tested
 * against the mock beside this file.
 *
 * Loaded only under Tauri. A browser never downloads it; see `main.tsx`.
 */

/**
 * A subscription that can be cancelled before it exists.
 *
 * `listen` is asynchronous and the contract is not: a component that mounts and
 * unmounts in the same tick, which React does in strict mode, would otherwise
 * leave a listener behind with nothing to cancel it.
 */
function subscribe<T>(start: () => Promise<UnlistenFn>): Unsubscribe {
  let live = true;
  let stop: UnlistenFn | null = null;

  void start().then(
    off => {
      if (live) stop = off;
      else off();
    },
    () => undefined,
  );

  return () => {
    live = false;
    stop?.();
    stop = null;
  };
}

/** The desktop window, which the app draws the title bar for itself. */
function createFrame(): WindowFrame {
  const window = getCurrentWindow();

  return {
    minimize: () => window.minimize(),
    toggleMaximize: () => window.toggleMaximize(),
    close: () => window.close(),
    startDrag: () => window.startDragging(),
    isMaximized: () => window.isMaximized(),

    onMaximizedChange(cb): Unsubscribe {
      // There is no maximised event, only a resize, and a window is resized far
      // more often than it is maximised. The answer is held so that dragging an
      // edge does not call back on every frame of the drag, and it is asked for
      // before the listener goes on: seeding it alongside would let a resize
      // that lands first compare against nothing and call back about no change.
      let last: boolean | null = null;

      return subscribe(async () => {
        last = await window.isMaximized().catch(() => false);
        return window.onResized(() => {
          void window.isMaximized().then(now => {
            if (now === last) return;
            last = now;
            cb(now);
          }, () => undefined);
        });
      });
    },
  };
}

/**
 * Tells the shell which theme the page settled on, and again when it changes.
 *
 * Android pads the web layer in by the system bar insets, so the strips that
 * leaves are the window background. That background can only follow the
 * system's dark mode, and the person may have chosen the other one, so the page
 * is what has to say. `data-theme` on the root is the choice, its absence means
 * the system decides, and `theme.ts` owns both.
 */
function followTheme(): void {
  const query = window.matchMedia('(prefers-color-scheme: dark)');

  const isDark = (): boolean => {
    const chosen = document.documentElement.getAttribute('data-theme');
    if (chosen === 'dark') return true;
    if (chosen === 'light') return false;
    return query.matches;
  };

  let last: boolean | null = null;
  const tell = (): void => {
    const now = isDark();
    if (now === last) return;
    last = now;
    void invoke('set_window_theme', { dark: now }).catch(() => undefined);
  };

  tell();
  new MutationObserver(tell).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });
  query.addEventListener('change', tell);
}

/** The last segment of a path, which is the name the file keeps. */
function fileName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

export async function createTauriBackend(): Promise<Backend> {
  /*
   * The two calls that have to happen before the first render, because the
   * contract has both of these as values rather than promises.
   *
   * The platform is asked of the shell. `import.meta.env.TAURI_ENV_PLATFORM`
   * is what the Tauri template reaches for and it is always undefined: Vite
   * matches `envPrefix` as a literal string, so the template's `TAURI_ENV_*`
   * matches no variable at all. Taking it for 'desktop' draws the title bar
   * and its window buttons on the phone.
   *
   * A version that could not be read is not worth failing the whole app over.
   */
  const [platform, version] = await Promise.all([
    invoke<Platform>('platform'),
    getVersion().catch(() => '0.0.0'),
  ]);

  if (platform === 'android') followTheme();

  return {
    platform,
    version,
    // Android draws its own status bar and back gesture; there is no frame here
    // for the app to take over.
    window: platform === 'android' ? null : createFrame(),

    getState: () => invoke<State>('get_state'),

    onState: cb => subscribe(() => listen<State>('state', event => cb(event.payload))),

    onDirChanged: cb =>
      subscribe(() => listen<string>('dir-changed', event => cb(event.payload))),

    onDrop: cb =>
      subscribe(() =>
        getCurrentWebview().onDragDropEvent(event => {
          // Only the drop carries paths. The hover is `onDragOver` below.
          if (event.payload.type === 'drop') cb(event.payload.paths);
        }),
      ),

    onDragOver: cb =>
      subscribe(() =>
        getCurrentWebview().onDragDropEvent(event => {
          // The shell took the drag off the web layer to read the paths out of
          // it, so this is the only place the hover can come from in a window.
          // A drop ends it as surely as a leave does.
          const { type } = event.payload;
          cb(type === 'enter' || type === 'over');
        }),
      ),

    listDir: path => invoke<DirEntry[]>('list_dir', { path }),

    async pickAndImport(into) {
      const picked = await open({ multiple: true, directory: false });
      if (picked === null || picked.length === 0) return 0;
      // Passed through exactly as the dialog gave them. On Android these are
      // content:// URIs that only this process can read, and rewriting one into
      // anything that looks more like a path makes it unopenable.
      return invoke<number>('import_paths', { paths: picked, into, names: null });
    },

    importPaths: (paths, into) =>
      invoke<number>('import_paths', {
        paths,
        into,
        // A drop is desktop only, so these are paths on disk and the name is
        // the one already on them.
        names: paths.map(fileName),
      }),

    createFolder: path => invoke<void>('create_folder', { path }),
    deleteEntry: path => invoke<void>('delete_entry', { path }),
    renameEntry: (path, newName) => invoke<void>('rename_entry', { path, newName }),
    openEntry: path => invoke<void>('open_entry', { path }),
    revealEntry: path => invoke<void>('reveal_entry', { path }),
    // The empty string is the sync folder itself, which is what the engine's
    // own path validation calls the root.
    revealFolder: (path = '') => invoke<void>('reveal_folder', { path }),

    async pickFolder() {
      // Android's picker hands back a tree URI rather than a path, and the
      // engine cannot make a sync folder out of one. The folder there is fixed.
      if (platform === 'android') return null;
      return open({ directory: true, multiple: false });
    },

    setFolder: path => invoke<void>('set_folder', { path }),
    setDeviceName: name => invoke<void>('set_device_name', { name }),
    pairWithNearby: id => invoke<void>('pair_with_nearby', { id }),
    pairWithAddress: (host, port) => invoke<void>('pair_with_address', { host, port }),
    respondToPairing: (id, accept) => invoke<void>('respond_to_pairing', { id, accept }),
    forgetPeer: id => invoke<void>('forget_peer', { id }),
    setPaused: paused => invoke<void>('set_paused', { paused }),

    allFilesPermission: () => invoke<Permission>('all_files_permission'),
    openAllFilesSettings: () => invoke<void>('open_all_files_settings'),

    openUrl: url => openUrl(url),
  };
}
