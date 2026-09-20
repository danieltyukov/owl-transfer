import { useRef, type ComponentType } from 'react';

import type { DirEntry } from '../../backend/types.js';
import { formatBytes, formatRelative } from '../../format.js';
import {
  ArchiveGlyph,
  AudioGlyph,
  DocumentGlyph,
  FileGlyph,
  FolderGlyph,
  ImageGlyph,
  MoreGlyph,
  VideoGlyph,
} from '../../icons/glyphs.js';
import { SyncBadge } from './SyncBadge.js';
import './FileRow.css';

const BY_EXTENSION: ReadonlyArray<readonly [ReadonlyArray<string>, ComponentType]> = [
  [['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic', 'avif', 'svg', 'bmp', 'tiff'], ImageGlyph],
  [['mp3', 'wav', 'flac', 'm4a', 'aac', 'ogg', 'opus'], AudioGlyph],
  [['mp4', 'mov', 'mkv', 'webm', 'avi', 'm4v'], VideoGlyph],
  [['zip', 'tar', 'gz', 'bz2', 'xz', '7z', 'rar', 'zst'], ArchiveGlyph],
  [['txt', 'md', 'pdf', 'doc', 'docx', 'odt', 'rtf', 'csv', 'json', 'yaml', 'yml'], DocumentGlyph],
];

/** The glyph for an entry, by its extension. Anything unrecognised is a sheet. */
export function glyphFor(entry: DirEntry): ComponentType {
  if (entry.isDir) return FolderGlyph;
  const dot = entry.name.lastIndexOf('.');
  if (dot <= 0) return FileGlyph;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  for (const [extensions, glyph] of BY_EXTENSION) {
    if (extensions.includes(ext)) return glyph;
  }
  return FileGlyph;
}

export interface FileRowProps {
  entry: DirEntry;
  now: number;
  onOpen: (entry: DirEntry) => void;
  onMenu: (entry: DirEntry, at: { x: number; y: number }) => void;
}

const LONG_PRESS_MS = 500;
const SLOP_PX = 6;

/*
 * One file or folder.
 *
 * The row is a button, so it opens from the keyboard as well as the pointer.
 * The overflow control beside it is a second button rather than one nested
 * inside the first, which would be invalid and unreachable by tab.
 *
 * A long press opens the same menu the right button does. It is cancelled by
 * any movement past a few pixels, so a scroll that starts on a row is a scroll
 * and not a menu.
 */
export function FileRow({ entry, now, onOpen, onMenu }: FileRowProps) {
  const Glyph = glyphFor(entry);
  const press = useRef<{ timer: number; x: number; y: number } | null>(null);

  const cancelPress = (): void => {
    if (press.current !== null) window.clearTimeout(press.current.timer);
    press.current = null;
  };

  return (
    <li
      className="row"
      onContextMenu={e => {
        e.preventDefault();
        onMenu(entry, { x: e.clientX, y: e.clientY });
      }}
      onPointerDown={e => {
        if (e.pointerType === 'mouse') return;
        const at = { x: e.clientX, y: e.clientY };
        press.current = {
          x: at.x,
          y: at.y,
          timer: window.setTimeout(() => {
            press.current = null;
            onMenu(entry, at);
          }, LONG_PRESS_MS),
        };
      }}
      onPointerMove={e => {
        const started = press.current;
        if (started === null) return;
        if (Math.abs(e.clientX - started.x) > SLOP_PX || Math.abs(e.clientY - started.y) > SLOP_PX) {
          cancelPress();
        }
      }}
      onPointerUp={cancelPress}
      onPointerCancel={cancelPress}
    >
      <button type="button" className="row-main" onClick={() => onOpen(entry)}>
        <span className="row-glyph" aria-hidden="true">
          <Glyph />
        </span>
        <span className="row-name">{entry.name}</span>
        <SyncBadge status={entry.status} />
        <span className="row-size mono">{entry.isDir ? '' : formatBytes(entry.size)}</span>
        <span className="row-when">{formatRelative(entry.mtimeMs, now)}</span>
      </button>

      <button
        type="button"
        className="row-more"
        aria-label={`More for ${entry.name}`}
        aria-haspopup="menu"
        onClick={e => {
          const box = e.currentTarget.getBoundingClientRect();
          onMenu(entry, { x: box.right, y: box.bottom });
        }}
      >
        <MoreGlyph />
      </button>
    </li>
  );
}
