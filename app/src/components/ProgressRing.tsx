import './ProgressRing.css';

export interface ProgressRingProps {
  /** 0 to 1. Anything outside that is clamped rather than drawn past the ring. */
  progress: number;
  size?: number;
  className?: string;
}

const BOX = 20;
const STROKE = 3;
const RADIUS = (BOX - STROKE) / 2;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/*
 * The one shape in the interface that means "on its way".
 *
 * It appears at two sizes: 14px in a file row, where it is that file's share of
 * the transfer, and 10px in the status strip, where it is the whole folder's.
 * Same arc, same amber, so the row and the strip are visibly talking about the
 * same thing.
 *
 * Decorative on purpose. Every place it is drawn, the text beside it already
 * says what is happening and how far along it is.
 */
export function ProgressRing({ progress, size = 14, className }: ProgressRingProps) {
  const filled = Math.min(Math.max(progress, 0), 1);
  return (
    <svg
      className={className === undefined ? 'ring' : `ring ${className}`}
      viewBox={`0 0 ${BOX} ${BOX}`}
      width={size}
      height={size}
      aria-hidden="true"
      focusable="false"
    >
      <circle className="ring-track" cx={BOX / 2} cy={BOX / 2} r={RADIUS} strokeWidth={STROKE} />
      <circle
        className="ring-arc"
        cx={BOX / 2}
        cy={BOX / 2}
        r={RADIUS}
        strokeWidth={STROKE}
        strokeDasharray={`${CIRCUMFERENCE * filled} ${CIRCUMFERENCE}`}
      />
    </svg>
  );
}
