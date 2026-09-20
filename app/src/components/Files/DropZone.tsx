import { useRef, useState, type ReactNode } from 'react';

import './DropZone.css';

export interface DropZoneProps {
  /** Where a dropped file would land, named in the hint. */
  label: string;
  children: ReactNode;
}

/*
 * The highlight a window shows while something is being dragged over it.
 *
 * The drop itself never happens here. A browser hands over File objects, and
 * the engine needs paths on disk, so the shell catches the real drop and calls
 * `onDrop` on the backend with paths. This is the half of it a person sees, and
 * it is the half that works the same in a browser and in a window.
 *
 * Enter and leave fire once per child element under the pointer, so they are
 * counted rather than treated as a pair.
 */
export function DropZone({ label, children }: DropZoneProps) {
  const [over, setOver] = useState(false);
  const depth = useRef(0);

  const reset = (): void => {
    depth.current = 0;
    setOver(false);
  };

  return (
    <div
      className="dropzone"
      data-over={over ? '' : undefined}
      onDragEnter={event => {
        if (![...event.dataTransfer.types].includes('Files')) return;
        depth.current += 1;
        setOver(true);
      }}
      onDragOver={event => {
        if (![...event.dataTransfer.types].includes('Files')) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'copy';
      }}
      onDragLeave={() => {
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
