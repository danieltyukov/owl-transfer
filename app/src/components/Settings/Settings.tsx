import { useEffect, useState } from 'react';

import type { Backend, State } from '../../backend/types.js';
import { formatBytes, plural } from '../../format.js';
import { ExternalGlyph } from '../../icons/glyphs.js';
import type { Theme } from '../../theme.js';
import { PermissionCard } from '../PermissionCard.js';
import './Settings.css';

export interface SettingsProps {
  backend: Backend;
  state: State;
  theme: Theme;
  onTheme: (theme: Theme) => void;
  onError: (message: string) => void;
}

const REPOSITORY = 'https://github.com/danieltyukov/owl-transfer';

const THEMES: ReadonlyArray<readonly [Theme, string]> = [
  ['system', 'System'],
  ['light', 'Light'],
  ['dark', 'Dark'],
];

/*
 * What this device is called, where its folder is, how it looks, and what it
 * is. Five cards, in the order a person needs them.
 */
export function Settings({ backend, state, theme, onTheme, onError }: SettingsProps) {
  const [name, setName] = useState(state.device.name);

  // The engine is the owner of the name. If it changes underneath us, the field
  // follows rather than holding a stale draft nobody typed.
  useEffect(() => setName(state.device.name), [state.device.name]);

  const commitName = (): void => {
    const next = name.trim();
    if (next === '' || next === state.device.name) {
      setName(state.device.name);
      return;
    }
    void backend.setDeviceName(next).catch(() => onError('That name could not be saved.'));
  };

  const changeFolder = (): void => {
    void backend
      .pickFolder()
      .then(picked => (picked === null ? undefined : backend.setFolder(picked)))
      .catch(() => onError('That folder could not be used.'));
  };

  return (
    <>
      <div className="pane-title">
        <h1>Settings</h1>
      </div>

      <div className="pane-body">
        <div className="pane-column">
          <section className="panel" aria-labelledby="settings-device">
            <p className="panel-title" id="settings-device">
              Device
            </p>
            <p className="panel-note">The name the other device shows for this one.</p>
            <input
              className="input"
              aria-label="Device name"
              value={name}
              onChange={event => setName(event.target.value)}
              onBlur={commitName}
              onKeyDown={event => {
                if (event.key === 'Enter') event.currentTarget.blur();
                if (event.key === 'Escape') setName(state.device.name);
              }}
            />
          </section>

          <section className="panel" aria-labelledby="settings-folder">
            <p className="panel-title" id="settings-folder">
              Sync folder
            </p>
            <p className="panel-note mono settings-path">{state.folder}</p>
            <p className="panel-note">
              {plural(state.summary.files, 'file')} and {plural(state.summary.dirs, 'folder')},{' '}
              {formatBytes(state.summary.bytes)}.
            </p>
            <div className="settings-row">
              <button type="button" className="button" onClick={changeFolder}>
                Change
              </button>
              {backend.platform === 'desktop' ? (
                <button
                  type="button"
                  className="button"
                  onClick={() => {
                    void backend.revealFolder().catch(() => onError('The folder would not open.'));
                  }}
                >
                  Open folder
                </button>
              ) : null}
            </div>
          </section>

          <section className="panel" aria-labelledby="settings-appearance">
            <p className="panel-title" id="settings-appearance">
              Appearance
            </p>
            <div className="segmented" role="group" aria-labelledby="settings-appearance">
              {THEMES.map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  className="segment"
                  aria-pressed={theme === id}
                  onClick={() => onTheme(id)}
                >
                  {label}
                </button>
              ))}
            </div>
          </section>

          <PermissionCard backend={backend} />

          <section className="panel" aria-labelledby="settings-about">
            <p className="panel-title" id="settings-about">
              About
            </p>
            <p className="panel-note">
              Owl Transfer <span className="mono">{backend.version}</span>. One folder, two
              devices, no account and no server.
            </p>
            <div className="settings-row">
              <button
                type="button"
                className="button"
                onClick={() => {
                  void backend.openUrl(REPOSITORY).catch(() => onError('That link would not open.'));
                }}
              >
                <ExternalGlyph />
                Owl Transfer is open source
              </button>
            </div>
          </section>
        </div>
      </div>
    </>
  );
}
