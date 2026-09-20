import './Wordmark.css';

export interface WordmarkProps {
  className?: string;
}

/*
 * "OwlTransfer", as one lockup.
 *
 * A weight step and nothing else: "Owl" in the semibold, "Transfer" in the
 * regular, one word, one size, one colour. The step is what keeps the single
 * word readable as a name and a verb rather than a run of letters.
 *
 * Built from real characters rather than a picture of them, so the name stays
 * selectable, searchable, and read by a screen reader as "OwlTransfer".
 */
export function Wordmark({ className }: WordmarkProps) {
  return (
    <span className={className === undefined ? 'wordmark' : `wordmark ${className}`}>
      Owl<span className="wordmark-tail">Transfer</span>
    </span>
  );
}
