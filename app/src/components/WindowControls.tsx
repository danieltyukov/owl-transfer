import type { MouseEvent } from 'react';

import type { WindowFrame } from '../backend/types.js';
import { CloseGlyph, MaximizeGlyph, MinimizeGlyph } from '../icons/glyphs.js';
import './WindowControls.css';

/*
 * The three buttons a native title bar would have carried, at the end of the
 * one the app draws itself.
 *
 * The desktop window is undecorated, so `TitleBar` is the window's top strip
 * and this is the cluster in its corner. Every press goes back to the shell
 * through `WindowFrame`, which is what keeps the interface free of any mention
 * of Tauri.
 *
 * Rendered only where there is a frame to drive. A browser tab and an Android
 * activity have chrome already, and a second close button inside the page
 * would be a button that closes the wrong thing.
 */

export interface WindowControlsProps {
  frame: WindowFrame;
}

export function WindowControls({ frame }: WindowControlsProps) {
  return (
    <div className="window-controls" role="group" aria-label="Window">
      <button
        type="button"
        className="window-control"
        aria-label="Minimize"
        onClick={() => void frame.minimize()}
      >
        <MinimizeGlyph />
      </button>
      {/*
        One label for both directions. The frame is a toggle and cannot say
        which way it is pointing, so naming it "Maximize" would tell a screen
        reader the wrong thing in half of all windows.
      */}
      <button
        type="button"
        className="window-control"
        aria-label="Maximize or restore"
        onClick={() => void frame.toggleMaximize()}
      >
        <MaximizeGlyph />
      </button>
      <button
        type="button"
        className="window-control window-close"
        aria-label="Close"
        onClick={() => void frame.close()}
      >
        <CloseGlyph />
      </button>
    </div>
  );
}

/**
 * What a press on a drag region must not start a drag from: anything that is
 * itself a control. The same list a native title bar honours, and the same one
 * Tauri's own drag-region script uses, so behaviour matches what a user of any
 * other undecorated window expects.
 */
const CLICKABLE = [
  'a',
  'button',
  'input',
  'select',
  'textarea',
  'label',
  'summary',
  '[contenteditable]:not([contenteditable="false"])',
  '[tabindex]:not([tabindex="-1"])',
  '[role="button"]',
  '[role="link"]',
  '[role="menuitem"]',
  '[role="tab"]',
  '[role="checkbox"]',
  '[role="radio"]',
  '[role="switch"]',
  '[role="option"]',
].join(', ');

/**
 * The gesture a title bar gives a window: press and drag to move it, press
 * twice to maximise or restore it.
 *
 * One handler on the root rather than one per region. Anything carrying
 * `data-window-drag` is a handle, and the root decides what a press inside one
 * means: a press on a control inside a handle is a press on the control, a
 * press anywhere else in it is a grab. Mouse events rather than pointer
 * events, because `detail` (the click count) is what tells a grab from a double
 * press and pointer events do not carry it.
 *
 * Returns undefined when there is no frame, so a browser has no handler at all
 * rather than one that returns early on every press.
 */
export function windowGrab(
  frame: WindowFrame | null,
): ((e: MouseEvent<HTMLElement>) => void) | undefined {
  if (frame === null) return undefined;
  return e => {
    if (e.button !== 0) return;
    const target = e.target as Element;
    if (target.closest('[data-window-drag]') === null) return;
    if (target.closest(CLICKABLE) !== null) return;
    // The browser would otherwise start a text selection under the pointer and
    // carry it along for the whole drag.
    e.preventDefault();
    if (e.detail === 2) void frame.toggleMaximize();
    else if (e.detail === 1) void frame.startDrag();
  };
}
