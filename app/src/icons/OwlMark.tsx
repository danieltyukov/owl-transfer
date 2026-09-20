import type { SVGProps } from 'react';

/*
 * The drawing, as data.
 *
 * The launcher and desktop icons are rendered from these same strings by
 * scripts/render-icons.mjs, and a test asserts the rendered source still
 * carries them. A mark that is redrawn in three places drifts in three places.
 */
export const OWL_FACE_PATH = {
  /** One V across the top. A brow, not a pair of tufts: tufts read as a cat. */
  brow: 'M5,11 L16,6.5 L27,11',
  beak: 'M14.4,23.2 L17.6,23.2 L16,26.4 Z',
} as const;

export const OWL_FACE = {
  viewBox: '0 0 32 32',
  stroke: 2.2,
  /** Two rings on one line: an owl's eyes, and two devices watching each other. */
  eyes: [
    { cx: 10.5, cy: 17 },
    { cx: 21.5, cy: 17 },
  ],
  eyeRadius: 5,
  pupilRadius: 2.1,
  ...OWL_FACE_PATH,
} as const;

export interface OwlMarkProps extends Omit<SVGProps<SVGSVGElement>, 'width' | 'height'> {
  size?: number;
  /**
   * Pass null where the mark sits beside the words "OwlTransfer" already: two
   * accessible names for one thing makes a screen reader say it twice.
   */
  label?: string | null;
}

/*
 * The logo: an owl's face on a 32 unit grid.
 *
 * A single V brow across the top, two ringed eyes, a small beak. The rings are
 * the point of it. They are an owl at a glance and a pair of lenses at a
 * second look, which is the app: two devices facing each other, each holding
 * the whole picture.
 *
 * Deliberately not to-hoot's open heart brow. The two applications are
 * siblings, drawn on the same grid with the same stroke, and they should read
 * as a family rather than as the same mark twice.
 *
 * The 2.2 stroke and the gap between ring and pupil close up below about 18px.
 * Nothing in the interface draws it smaller than that.
 */
export function OwlMark({ size = 32, label = 'OwlTransfer', ...rest }: OwlMarkProps) {
  const a11y =
    label === null ? { 'aria-hidden': true as const } : { role: 'img', 'aria-label': label };
  return (
    <svg viewBox={OWL_FACE.viewBox} width={size} height={size} {...a11y} {...rest}>
      <path
        d={OWL_FACE.brow}
        fill="none"
        stroke="currentColor"
        strokeWidth={OWL_FACE.stroke}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {OWL_FACE.eyes.map(eye => (
        <circle
          key={eye.cx}
          cx={eye.cx}
          cy={eye.cy}
          r={OWL_FACE.eyeRadius}
          fill="none"
          stroke="currentColor"
          strokeWidth={OWL_FACE.stroke}
        />
      ))}
      {OWL_FACE.eyes.map(eye => (
        <circle
          key={`pupil-${eye.cx}`}
          cx={eye.cx}
          cy={eye.cy}
          r={OWL_FACE.pupilRadius}
          fill="currentColor"
        />
      ))}
      <path d={OWL_FACE.beak} fill="currentColor" />
    </svg>
  );
}
