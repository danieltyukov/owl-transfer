import type { State } from '../backend/types.js';
import { DesktopGlyph, PhoneGlyph, PlusGlyph } from '../icons/glyphs.js';
import { OwlMark } from '../icons/OwlMark.js';
import { PANES, PaneGlyph, type Pane } from '../panes.js';
import { Wordmark } from './Wordmark.js';
import './Sidebar.css';

export interface SidebarProps {
  pane: Pane;
  onPane: (pane: Pane) => void;
  state: State;
  /** True when the window draws its own title bar, which carries the brand. */
  framed: boolean;
}

/*
 * The desktop's left column: where the folder is, where the app can go, and who
 * it is talking to.
 *
 * The paired devices are listed here rather than only on the Devices pane
 * because their dots are the ambient answer to "is the other one awake", and a
 * person should not have to change panes to see it. Pressing one opens Devices,
 * where the detail lives.
 *
 * On a phone this is not drawn at all. The tabs at the bottom do the
 * navigating, and the pane itself carries the brand.
 */
export function Sidebar({ pane, onPane, state, framed }: SidebarProps) {
  return (
    <nav className="sidebar" aria-label="Main">
      {framed ? null : (
        <div className="brand" data-window-drag="">
          <OwlMark size={20} label={null} className="brand-mark" />
          <Wordmark className="brand-word" />
        </div>
      )}

      <p className="sidebar-folder mono" title={state.folder}>
        <bdi>{state.folder}</bdi>
      </p>

      <ul className="nav">
        {PANES.map(([id, label]) => (
          <li key={id}>
            <button
              type="button"
              className="nav-item"
              aria-current={pane === id ? 'page' : undefined}
              onClick={() => onPane(id)}
            >
              <span className="nav-glyph">
                <PaneGlyph pane={id} />
              </span>
              <span className="nav-label">{label}</span>
            </button>
          </li>
        ))}
      </ul>

      <p className="section-title sidebar-head">
        {state.peers.length === 0 ? 'No devices yet' : 'Devices'}
      </p>

      <ul className="nav">
        {state.peers.map(peer => (
          <li key={peer.id}>
            <button type="button" className="nav-item" onClick={() => onPane('devices')}>
              <span className="nav-glyph">
                {peer.kind === 'phone' ? <PhoneGlyph /> : <DesktopGlyph />}
              </span>
              <span className="nav-label">{peer.name}</span>
              <span
                className="nav-dot"
                data-connected={peer.connected ? '' : undefined}
                title={peer.connected ? 'Connected' : 'Not connected'}
              />
              <span className="visually-hidden">
                {peer.connected ? 'Connected' : 'Not connected'}
              </span>
            </button>
          </li>
        ))}
        <li>
          <button type="button" className="nav-item" onClick={() => onPane('devices')}>
            <span className="nav-glyph">
              <PlusGlyph />
            </span>
            <span className="nav-label">Pair a device</span>
          </button>
        </li>
      </ul>
    </nav>
  );
}
