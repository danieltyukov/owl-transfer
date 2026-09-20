import { useEffect, useRef, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';

import type { DirEntry } from '../../backend/types.js';
import './EntryMenu.css';

export interface EntryMenuProps {
  entry: DirEntry;
  /** Viewport coordinates of the press that opened it. */
  at: { x: number; y: number };
  onOpen: () => void;
  onRename: () => void;
  onDelete: () => void;
  onClose: () => void;
}

const WIDTH = 168;
const MARGIN = 8;

/*
 * What can be done to one file: open it, rename it, delete it.
 *
 * Opened by the right button, by the row's overflow control, and by a long
 * press on a touch screen. All three land here, so a phone and a desktop offer
 * the same three things in the same order.
 *
 * Placed at the press and then pulled back inside the window, which is what a
 * menu opened near the right edge has to do.
 */
export function EntryMenu({ entry, at, onOpen, onRename, onDelete, onClose }: EntryMenuProps) {
  const surface = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const returnTo = document.activeElement;
    surface.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    return () => {
      if (returnTo instanceof HTMLElement) returnTo.focus();
    };
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;

    const items = [...(surface.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    const here = items.indexOf(document.activeElement as HTMLElement);
    const step = event.key === 'ArrowDown' ? 1 : -1;
    const next = items[(here + step + items.length) % items.length];
    if (next !== undefined) {
      event.preventDefault();
      next.focus();
    }
  };

  const left = Math.max(MARGIN, Math.min(at.x, window.innerWidth - WIDTH - MARGIN));
  const top = Math.max(MARGIN, Math.min(at.y, window.innerHeight - 132));

  return createPortal(
    <div className="menu-layer" onMouseDown={onClose}>
      <div
        ref={surface}
        className="menu"
        role="menu"
        aria-label={entry.name}
        style={{ left, top, width: WIDTH }}
        onKeyDown={onKeyDown}
        onMouseDown={event => event.stopPropagation()}
      >
        <button type="button" role="menuitem" className="menu-item" onClick={onOpen}>
          Open
        </button>
        <button type="button" role="menuitem" className="menu-item" onClick={onRename}>
          Rename
        </button>
        <button
          type="button"
          role="menuitem"
          className="menu-item menu-item-danger"
          onClick={onDelete}
        >
          Delete
        </button>
      </div>
    </div>,
    document.body,
  );
}
