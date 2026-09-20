import type { State } from '../backend/types.js';
import { formatRate, formatRelative, plural } from '../format.js';
import { ProgressRing } from './ProgressRing.js';
import './StatusStrip.css';

export interface StatusStripProps {
  state: State;
  now?: number;
}

type Tone = 'ok' | 'work' | 'idle';

/*
 * The one line that answers the only question this app exists to answer: is my
 * other device up to date.
 *
 * It runs the whole width of the window, under the sidebar as well as the
 * files, because it is about the folder rather than about whatever pane is
 * open. On a phone it sits above the tabs for the same reason.
 *
 * The rate is left out of what a screen reader announces. It changes several
 * times a second and adds nothing to "syncing 3 files"; the count is the part
 * that means something changed.
 */
export function StatusStrip({ state, now = Date.now() }: StatusStripProps) {
  const { transfers, summary, peers } = state;
  const connected = peers.filter(p => p.connected).length;
  const active = transfers.active;

  const done = active.reduce((sum, t) => sum + t.bytesDone, 0);
  const total = active.reduce((sum, t) => sum + t.bytesTotal, 0);

  let tone: Tone;
  let text: string;
  let rate: string | null = null;

  if (active.length > 0) {
    tone = 'work';
    text = `Syncing ${plural(active.length, 'file')}`;
    rate = formatRate(transfers.bytesPerSec);
  } else if (connected === 0) {
    tone = 'idle';
    text = peers.length === 0 ? 'No device paired' : 'No device connected';
  } else if (summary.upToDate) {
    tone = 'ok';
    text = 'Up to date';
  } else {
    tone = 'work';
    text = 'Catching up';
  }

  return (
    <footer className="status" role="status" data-tone={tone}>
      {tone === 'work' && total > 0 ? (
        <ProgressRing progress={done / total} size={11} className="status-ring" />
      ) : (
        <span className="status-dot" aria-hidden="true" />
      )}

      <p className="status-text">
        {text}
        {rate === null ? null : (
          <span className="status-rate mono" aria-hidden="true">
            , {rate}
          </span>
        )}
      </p>

      <p className="status-meta">
        <span>{plural(peers.length, 'device')}</span>
        {summary.lastChangeMs === null ? null : (
          <>
            <span className="status-sep" aria-hidden="true">
              ·
            </span>
            <span>changed {formatRelative(summary.lastChangeMs, now)}</span>
          </>
        )}
      </p>
    </footer>
  );
}
