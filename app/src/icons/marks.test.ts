import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { OWL_FACE, OWL_FACE_PATH } from './OwlMark.js';

/*
 * The drawing is in four places and has to stay one drawing.
 *
 * `OwlMark.tsx` renders it in the interface, `scripts/render-icons.mjs` renders
 * it into the desktop artwork, the browser tab and the Android launcher. The
 * two written SVGs are committed, so nothing recomputes them on a build and
 * nothing would notice if the script and the component drifted apart.
 *
 * This reads the committed files and asserts they still carry the component's
 * own constants. A change to the mark that does not reach the icons fails here
 * rather than shipping an app whose logo and launcher are different owls.
 *
 * Substrings rather than a parse, deliberately: the point is that the exact
 * path strings survive, and a parser that normalised `M5,11` into `M 5 11`
 * would let exactly the drift this exists to catch go through.
 */

/** Resolved against this file, so the test does not care where it was run. */
const read = (relative: string): string =>
  readFileSync(new URL(relative, import.meta.url), 'utf8');

const MARKS: ReadonlyArray<readonly [string, string]> = [
  ['app/icon-source.svg', read('../../icon-source.svg')],
  ['app/public/favicon.svg', read('../../public/favicon.svg')],
];

describe.each(MARKS)('%s', (_name, svg) => {
  it('draws the brow from the same string the component does', () => {
    expect(svg).toContain(`d="${OWL_FACE_PATH.brow}"`);
  });

  it('draws the beak from the same string the component does', () => {
    expect(svg).toContain(`d="${OWL_FACE_PATH.beak}"`);
  });

  it('puts both eye rings where the component puts them', () => {
    for (const eye of OWL_FACE.eyes) {
      expect(svg).toContain(`cx="${eye.cx}" cy="${eye.cy}" r="${OWL_FACE.eyeRadius}"`);
    }
  });

  it('strokes at the weight the component strokes at', () => {
    expect(svg).toContain(`stroke-width="${OWL_FACE.stroke}"`);
  });
});
