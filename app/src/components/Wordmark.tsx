import './Wordmark.css';

export interface WordmarkProps {
  className?: string;
}

/*
 * "Owl Transfer", as one lockup.
 *
 * A weight step and nothing else: the name in the semibold, what it does in the
 * regular, both at one size and one colour. The two words are a noun and a verb
 * and the type says so, which is all a two-word wordmark has to do.
 *
 * Built from real characters rather than a picture of them, so the name stays
 * selectable, searchable, and read by a screen reader as "Owl Transfer".
 */
export function Wordmark({ className }: WordmarkProps) {
  return (
    <span className={className === undefined ? 'wordmark' : `wordmark ${className}`}>
      Owl <span className="wordmark-tail">Transfer</span>
    </span>
  );
}
