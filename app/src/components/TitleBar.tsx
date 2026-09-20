import type { WindowFrame } from '../backend/types.js';
import { OwlMark } from '../icons/OwlMark.js';
import { WindowControls } from './WindowControls.js';
import { Wordmark } from './Wordmark.js';
import './TitleBar.css';

export interface TitleBarProps {
  frame: WindowFrame;
  /** What the window is showing: Files, Devices or Settings. */
  title: string;
  onError: (message: string) => void;
}

/*
 * The window's own title bar, for a window the shell draws none for.
 *
 * Product at the left, what the window is showing in the middle, controls at
 * the right, the whole strip a drag handle. That is the shape the platform's
 * users already read, and it lets a pane header go back to being a pane header.
 */
export function TitleBar({ frame, title, onError }: TitleBarProps) {
  return (
    <header className="titlebar" data-window-drag="">
      <div className="titlebar-brand">
        <OwlMark size={18} label={null} className="brand-mark" />
        <Wordmark className="brand-word" />
      </div>

      {/*
        Chrome, not content. Every word here is a heading somewhere in the
        window underneath, so announcing it again would read the same thing
        twice to anyone who cannot see that this is a title bar.
      */}
      <p className="titlebar-title" aria-hidden="true">
        {title}
      </p>

      <WindowControls frame={frame} onError={onError} />
    </header>
  );
}
