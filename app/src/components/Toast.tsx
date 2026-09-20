import { useCallback, useEffect, useRef, useState } from 'react';

import { CloseGlyph } from '../icons/glyphs.js';
import './Toast.css';

export interface ToastItem {
  id: number;
  text: string;
}

const LIFETIME_MS = 6_000;

let counter = 0;

/**
 * What in `next` was not in `previous`, for a list that rolls.
 *
 * `State.errors` is the last five, newest last, so a sixth error pushes the
 * first off the front and the length never changes again. Counting is
 * therefore the one thing that cannot work: the window has to be lined up by
 * what is in it. The longest tail of `previous` that is also the head of
 * `next` is the overlap, and everything past it is new.
 *
 * No overlap means the whole window turned over between looks, and all of it
 * is new. Exported so the arithmetic is testable on its own.
 */
export function freshEntries(previous: readonly string[], next: readonly string[]): string[] {
  const most = Math.min(previous.length, next.length);
  for (let overlap = most; overlap > 0; overlap--) {
    const tail = previous.slice(previous.length - overlap);
    if (tail.every((line, index) => line === next[index])) return next.slice(overlap);
  }
  return [...next];
}

/*
 * Things that went wrong, said once and then out of the way.
 *
 * Two sources feed it: the `errors` the engine carries in its state, and the
 * failures of whatever the person just pressed.
 */
export function useToasts(errors: readonly string[]): {
  toasts: ToastItem[];
  push: (text: string) => void;
  dismiss: (id: number) => void;
} {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const seen = useRef<readonly string[]>([]);

  const push = useCallback((text: string) => {
    counter += 1;
    const item = { id: counter, text };
    setToasts(list => [...list, item]);
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts(list => list.filter(toast => toast.id !== id));
  }, []);

  useEffect(() => {
    const fresh = freshEntries(seen.current, errors);
    seen.current = errors;
    for (const text of fresh) push(text);
  }, [errors, push]);

  return { toasts, push, dismiss };
}

export interface ToastsProps {
  toasts: readonly ToastItem[];
  onDismiss: (id: number) => void;
}

export function Toasts({ toasts, onDismiss }: ToastsProps) {
  if (toasts.length === 0) return null;
  return (
    <ul className="toasts">
      {toasts.map(toast => (
        <Toast key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </ul>
  );
}

function Toast({ toast, onDismiss }: { toast: ToastItem; onDismiss: (id: number) => void }) {
  const { id } = toast;
  useEffect(() => {
    const timer = window.setTimeout(() => onDismiss(id), LIFETIME_MS);
    return () => window.clearTimeout(timer);
  }, [id, onDismiss]);

  return (
    <li className="toast" role="alert">
      <span className="toast-text">{toast.text}</span>
      <button
        type="button"
        className="toast-close"
        aria-label="Dismiss"
        onClick={() => onDismiss(id)}
      >
        <CloseGlyph />
      </button>
    </li>
  );
}
