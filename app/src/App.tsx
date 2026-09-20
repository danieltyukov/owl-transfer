import { useState } from 'react';

import type { Backend } from './backend/types.js';
import { Devices } from './components/Devices/Devices.js';
import { Files } from './components/Files/Files.js';
import { useStorageAccess } from './components/PermissionCard.js';
import { Settings } from './components/Settings/Settings.js';
import { Sidebar } from './components/Sidebar.js';
import { StatusStrip } from './components/StatusStrip.js';
import { TitleBar } from './components/TitleBar.js';
import { Toasts, useToasts } from './components/Toast.js';
import { windowGrab } from './components/WindowControls.js';
import { PANES, PaneGlyph, type Pane } from './panes.js';
import { useBackendState } from './state.js';
import { useTheme } from './theme.js';
import './App.css';

export interface AppProps {
  backend: Backend;
}

/*
 * The shell.
 *
 * A sidebar beside one pane on a desktop, the same pane alone with tabs under
 * it on a phone, and the status strip across the bottom of both. The panes are
 * the same components at either width: nothing is rebuilt for the small screen,
 * so a fix to the file list is made once.
 *
 * Only the open pane is mounted, and the folder the file list is showing is
 * held here rather than inside it, so moving to Devices and back comes home to
 * the same directory.
 *
 * Storage access is watched here for the same reason. On Android the engine
 * starts paused, and the grant that unpauses it is made on a system screen the
 * app never sees; watching for it from the root means it is noticed from
 * whichever pane the person happens to be on.
 */
const EMPTY: readonly string[] = [];

export function App({ backend }: AppProps) {
  const state = useBackendState(backend);
  const [pane, setPane] = useState<Pane>('files');
  const [dir, setDir] = useState('');
  const [theme, setTheme] = useTheme();
  const { toasts, push, dismiss } = useToasts(state?.errors ?? EMPTY);
  const storage = useStorageAccess(backend, state?.paused ?? false);

  const frame = backend.window;
  const grab = windowGrab(frame, push);

  // One frame on a desktop, a little longer on a phone still opening the
  // folder. An empty shell rather than a spinner: there is nothing to wait for
  // that is worth drawing a spinner about.
  if (state === null) {
    return <div className="shell" data-framed={frame === null ? undefined : ''} />;
  }

  const title = PANES.find(([id]) => id === pane)?.[1] ?? 'Files';

  return (
    <div className="shell" data-framed={frame === null ? undefined : ''} onMouseDown={grab}>
      {frame === null ? null : <TitleBar frame={frame} title={title} onError={push} />}

      <div className="app" data-pane={pane}>
        <Sidebar pane={pane} onPane={setPane} state={state} framed={frame !== null} />

        <main className="pane" key={pane}>
          {pane === 'files' ? (
            <Files
              backend={backend}
              state={state}
              dir={dir}
              onDir={setDir}
              onPane={setPane}
              onError={push}
              storage={storage}
            />
          ) : null}
          {pane === 'devices' ? (
            <Devices backend={backend} state={state} onError={push} />
          ) : null}
          {pane === 'settings' ? (
            <Settings
              backend={backend}
              state={state}
              theme={theme}
              onTheme={setTheme}
              onError={push}
              storage={storage}
            />
          ) : null}
        </main>

        <StatusStrip state={state} />

        <nav className="tabs" aria-label="Sections">
          {PANES.map(([id, label]) => (
            <button
              key={id}
              type="button"
              className="tab"
              aria-current={pane === id ? 'page' : undefined}
              onClick={() => setPane(id)}
            >
              <span className="tab-glyph">
                <PaneGlyph pane={id} />
              </span>
              {label}
            </button>
          ))}
        </nav>
      </div>

      <Toasts toasts={toasts} onDismiss={dismiss} />
    </div>
  );
}
