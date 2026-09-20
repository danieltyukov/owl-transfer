import { useCallback, useEffect, useRef, useState } from 'react';

import { CloseGlyph } from '../icons/glyphs.js';
import './Toast.css';

export interface ToastItem {
  id: number;
  text: string;
}

const LIFETIME_MS = 6_000;

let counter = 0;

/*
 * Things that went wrong, said once and then out of the way.
 *
 * Two sources feed it: the `errors` the engine carries in its state, and the
 * failures of whatever the person just pressed. The engine's list is treated as
 * a log, so only what arrived since the last look is shown; a list that shrank
 * means it was cleared, and the count starts again.
 */
export function useToasts(errors: readonly string[]): {
  toasts: ToastItem[];
  push: (text: string) => void;
  dismiss: (id: number) => void;
} {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const seen = useRef(0);

  const push = useCallback((text: string) => {
    counter += 1;
    const item = { id: counter, text };
    setToasts(list => [...list, item]);
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts(list => list.filter(toast => toast.id !== id));
  }, []);

  useEffect(() => {
    if (errors.length < seen.current) {
      seen.current = errors.length;
      return;
    }
    const fresh = errors.slice(seen.current);
    seen.current = errors.length;
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
