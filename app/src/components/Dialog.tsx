import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

import './Dialog.css';

const FOCUSABLE =
  'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])';

export interface DialogProps {
  title: string;
  /** One sentence under the title, where a choice needs explaining. */
  description?: string;
  submitLabel: string;
  /** Deleting. The confirming button says what it costs. */
  danger?: boolean;
  submitDisabled?: boolean;
  onSubmit: () => void;
  onClose: () => void;
  children?: ReactNode;
}

/*
 * The one modal shape in the app: rename, new folder, confirm a delete.
 *
 * A form rather than a pair of buttons, so Enter submits and Escape closes
 * without either being wired up by hand. Focus moves to the first control when
 * it opens and back to whatever opened it when it closes, which is what makes
 * a menu item that opens a dialog usable without a pointer.
 *
 * Portalled to the body: the pane it is opened from clips its own overflow, and
 * a dialog clipped by a file list is a dialog nobody can finish.
 */
export function Dialog({
  title,
  description,
  submitLabel,
  danger = false,
  submitDisabled = false,
  onSubmit,
  onClose,
  children,
}: DialogProps) {
  const surface = useRef<HTMLFormElement>(null);
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    const returnTo = document.activeElement;
    surface.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    return () => {
      if (returnTo instanceof HTMLElement) returnTo.focus();
    };
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;

    // The trap. Without it, tab walks out of the dialog and into the page
    // behind it, which is still there and still looks pressable.
    const items = [...(surface.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])];
    const first = items[0];
    const last = items[items.length - 1];
    if (first === undefined || last === undefined) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div
      className="scrim"
      onMouseDown={event => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <form
        ref={surface}
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description === undefined ? undefined : descriptionId}
        onKeyDown={onKeyDown}
        onSubmit={event => {
          event.preventDefault();
          if (!submitDisabled) onSubmit();
        }}
      >
        <h2 id={titleId} className="dialog-title">
          {title}
        </h2>
        {description === undefined ? null : (
          <p id={descriptionId} className="dialog-text">
            {description}
          </p>
        )}
        {children}
        <div className="dialog-actions">
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="submit"
            className={danger ? 'button button-danger-solid' : 'button button-primary'}
            disabled={submitDisabled}
          >
            {submitLabel}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  );
}
