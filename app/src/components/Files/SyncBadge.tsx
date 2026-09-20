import type { EntryStatus } from '../../backend/types.js';
import { CheckGlyph } from '../../icons/glyphs.js';
import { ProgressRing } from '../ProgressRing.js';
import './SyncBadge.css';

export interface SyncBadgeProps {
  status: EntryStatus;
}

/*
 * Where a file has got to, in the width of a glyph.
 *
 * Five states, and the colour tells them apart before the shape does: green is
 * here on both devices, amber is on its way or waiting for a turn, red is a
 * disagreement that needs a person. The accent is amber, which is why it never
 * marks the finished state.
 *
 * The synced check is the quiet one on purpose. Most rows most of the time are
 * synced, and a list where every row shouts is a list that says nothing.
 */
export function SyncBadge({ status }: SyncBadgeProps) {
  if (status.kind === 'synced') {
    return (
      <span className="badge badge-synced" title="Synced">
        <CheckGlyph />
        <span className="visually-hidden">Synced</span>
      </span>
    );
  }

  if (status.kind === 'syncing') {
    const percent = Math.round(Math.min(Math.max(status.progress, 0), 1) * 100);
    return (
      <span className="badge" title={`Syncing, ${percent} percent`}>
        <ProgressRing progress={status.progress} size={14} />
        <span className="visually-hidden">Syncing, {percent} percent</span>
      </span>
    );
  }

  if (status.kind === 'waiting') {
    return (
      <span className="badge badge-waiting" title="Waiting">
        <span className="badge-dot" aria-hidden="true" />
        <span className="visually-hidden">Waiting</span>
      </span>
    );
  }

  if (status.kind === 'conflict') {
    return <span className="pill pill-conflict">Conflict</span>;
  }

  return <span className="pill">Only here</span>;
}
