import { useEffect, useRef, useState, type ReactNode } from 'react';

import type { Backend } from '../../backend/types.js';
import './DropZone.css';

export interface DropZoneProps {
  backend: Backend;
  /** Where a dropped file would land, named in the hint. */
  label: string;
  children: ReactNode;
}

/*
 * The highlight a window shows while something is being dragged over it.
 *
 * The drop itself never happens here. A browser hands over File objects, and
 * the engine needs paths on disk, so the shell catches the real drop and calls
 * `onDrop` on the backend with paths. This is the half of it a person sees.
 *
 * It lights up from either of two places, because neither covers both cases. In
 * a browser the drag is the page's own and arrives as `dragenter`; enter and
 * leave fire once per child element under the pointer, so they are counted
 * rather than treated as a pair. In a desktop window the shell has already
 * taken the drag off the web layer to read the paths out of it, and nothing
 * arrives here at all: `onDragOver` is the shell saying so.
 */
export function DropZone({ backend, label, children }: DropZoneProps) {
  const [pageOver, setPageOver] = useState(false);
  const [shellOver, setShellOver] = useState(false);
  const depth = useRef(0);

  useEffect(() => backend.onDragOver(setShellOver), [backend]);

  const reset = (): void => {
    depth.current = 0;
    setPageOver(false);
  };

  return (
    <div
      className="dropzone"
      data-over={pageOver || shellOver ? '' : undefined}
      onDragEnter={event => {
        if (![...event.dataTransfer.types].includes('Files')) return;
        depth.current += 1;
        setPageOver(true);
      }}
      onDragOver={event => {
        if (![...event.dataTransfer.types].includes('Files')) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'copy';
      }}
      onDragLeave={() => {
        // Only a drag that carried files ever incremented this, so only one
        // that did may decrement it. Otherwise a drag of something else across
        // the pane drives the count below zero.
        if (!pageOver) return;
        depth.current -= 1;
        if (depth.current <= 0) reset();
      }}
      onDrop={event => {
        event.preventDefault();
        reset();
      }}
    >
      {children}
      <p className="dropzone-hint" aria-hidden="true">
        Drop to add to {label}
      </p>
    </div>
  );
}
