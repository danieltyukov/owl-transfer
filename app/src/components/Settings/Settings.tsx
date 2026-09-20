import type { Backend, State } from '../../backend/types.js';
import type { Theme } from '../../theme.js';

export interface SettingsProps {
  backend: Backend;
  state: State;
  theme: Theme;
  onTheme: (theme: Theme) => void;
}

/** This device's name, the folder, the theme, and what the app is. */
export function Settings({ state }: SettingsProps) {
  return (
    <>
      <div className="pane-title">
        <h1>Settings</h1>
      </div>
      <div className="pane-body">
        <div className="pane-column">
          <p className="panel-note mono">{state.folder}</p>
        </div>
      </div>
    </>
  );
}
