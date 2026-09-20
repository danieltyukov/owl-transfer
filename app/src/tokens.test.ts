// @vitest-environment node
// This one reads files off disk rather than rendering anything, and vitest
// stubs CSS imports, so it opts out of jsdom to get a real `import.meta.url`.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { readNameTable } from './test/woff2.js';

const read = (rel: string): string =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');

const readBinary = (rel: string): Buffer =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)));

// The stylesheet is the source of truth, so the test reads it rather than a
// duplicate table of the same hex values. A duplicate would agree with itself
// forever and tell us nothing about what ships.
const CSS = read('./tokens.css');
const NOTICES = ['sans', 'mono'].map(d => read(`./fonts/${d}/OFL.txt`));

interface Rule {
  selector: string;
  decls: Record<string, string>;
}

/**
 * Leaf declaration blocks in source order, comments stripped.
 *
 * An array rather than a map keyed by selector: `:root` legitimately appears
 * more than once (the base palette, and the coarse-pointer override), and a map
 * would silently keep only the last one.
 */
function rules(css: string): Rule[] {
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  for (const [, selector, body] of clean.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const decls: Record<string, string> = {};
    for (const line of body!.split(';')) {
      const at = line.indexOf(':');
      if (at === -1) continue;
      decls[line.slice(0, at).trim()] = line.slice(at + 1).trim();
    }
    out.push({ selector: selector!.trim().replace(/\s+/g, ' '), decls });
  }
  return out;
}

const RULES = rules(CSS);
const first = (selector: string): Record<string, string> =>
  RULES.find(r => r.selector === selector)?.decls ?? {};

const LIGHT = first(':root');
const SYSTEM_DARK = first(":root:not([data-theme='light'])");
const EXPLICIT_DARK = first(":root[data-theme='dark']");

/** The light palette with the dark overrides folded in. */
const DARK: Record<string, string> = { ...LIGHT, ...EXPLICIT_DARK };

function channel(v: number): number {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function rgb(hex: string): [number, number, number] {
  const h = hex.trim().replace('#', '');
  const full = h.length === 3 ? [...h].map(c => c + c).join('') : h;
  return [
    parseInt(full.slice(0, 2), 16),
    parseInt(full.slice(2, 4), 16),
    parseInt(full.slice(4, 6), 16),
  ];
}

function luminance(hex: string): number {
  const [r, g, b] = rgb(hex);
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** Hue in degrees, 0 = red, 120 = green. */
function hue(hex: string): number {
  const [r, g, b] = rgb(hex).map(v => v / 255) as [number, number, number];
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  if (d === 0) return 0;
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
}

const THEMES = [
  ['light', LIGHT],
  ['dark', DARK],
] as const;

/*
 * Each foreground token with the ratio it has to clear.
 *
 * Everything that can be set in type is at 4.5:1, `faint` included. Being small
 * and secondary is what makes contrast matter more, not less, and four of the
 * things `faint` colours here are words a user reads: the modified column, a
 * peer's address, the byte counts, the device id.
 *
 * `accent` is the exception at 3:1, the threshold WCAG 1.4.11 sets for a user
 * interface component. It marks the current pane, draws the focus ring and
 * fills the in-flight arc, and a test below asserts it is never a text colour.
 * Anything that wants amber words uses `accent-hover`.
 */
/*
 * The three grounds text is set on. `surface-2` is the sidebar, and the
 * sidebar carries the folder path, the navigation and the device names, so it
 * is as much a text ground as the page and the panel. Leaving it out is what
 * let `--faint` ship at 4.3:1 there.
 */
const GROUNDS = ['bg', 'surface', 'surface-2'] as const;

const FOREGROUNDS: ReadonlyArray<readonly [token: string, min: number]> = [
  ['text', 4.5],
  ['muted', 4.5],
  ['faint', 4.5],
  ['success', 4.5],
  ['danger', 4.5],
  ['info', 4.5],
  ['accent-hover', 4.5],
  ['accent', 3],
];

describe('tokens.css', () => {
  it('declares the whole light palette on bare :root', () => {
    for (const token of ['bg', 'surface', 'surface-2', 'border', 'text', 'muted', 'faint', 'accent']) {
      expect(LIGHT[`--${token}`], token).toBeDefined();
    }
  });

  it('gives no colour its only definition inside a dark block', () => {
    // Both blocks, not just the system one. They are asserted identical below,
    // which makes checking one enough today and wrong the moment that
    // assertion is the thing that breaks.
    for (const [where, block] of [
      ['prefers-color-scheme', SYSTEM_DARK],
      ["data-theme='dark'", EXPLICIT_DARK],
    ] as const) {
      for (const key of Object.keys(block)) {
        if (!key.startsWith('--')) continue;
        expect(LIGHT[key], `${key} is defined only under ${where}`).toBeDefined();
      }
    }
  });

  it('keeps the system-dark and explicit-dark blocks identical', () => {
    // They drift the moment someone edits one and forgets the other, and the
    // half that rots is the half their own machine does not render.
    expect(SYSTEM_DARK).toEqual(EXPLICIT_DARK);
  });

  it('guards the system-dark block so an explicit light choice still wins', () => {
    expect(CSS).toMatch(
      /@media \(prefers-color-scheme: dark\)\s*\{\s*:root:not\(\[data-theme='light'\]\)/,
    );
  });

  for (const [name, palette] of THEMES) {
    it(`${name}: text reads on every ground it is set on`, () => {
      for (const ground of GROUNDS) {
        expect(contrast(palette['--text']!, palette[`--${ground}`]!)).toBeGreaterThanOrEqual(4.5);
      }
    });

    it(`${name}: the label on a filled accent control clears 4.5:1`, () => {
      // The primary button fills with accent-hover and letters itself in
      // on-accent. White does that on the light ochre and not on the dark
      // amber, which is why the dark palette swaps it for the ink.
      expect(contrast(palette['--on-accent']!, palette['--accent-hover']!)).toBeGreaterThanOrEqual(
        4.5,
      );
    });

    for (const [token, min] of FOREGROUNDS) {
      for (const ground of GROUNDS) {
        it(`${name}: ${token} clears ${min}:1 on ${ground}`, () => {
          expect(contrast(palette[`--${token}`]!, palette[`--${ground}`]!)).toBeGreaterThanOrEqual(
            min,
          );
        });
      }
    }

    it(`${name}: the accent is not in the green hue family`, () => {
      // Green is the synced state. An accent that drifts into it makes "on its
      // way" and "here" the same colour, which is the one thing a sync
      // interface must never do.
      const h = hue(palette['--accent']!);
      expect(h < 60 || h > 180, `accent hue is ${h.toFixed(0)}deg`).toBe(true);
    });

    it(`${name}: success reads as green`, () => {
      const h = hue(palette['--success']!);
      expect(h).toBeGreaterThan(90);
      expect(h).toBeLessThan(180);
    });

    it(`${name}: borders are visible without being lines of text`, () => {
      expect(contrast(palette['--border-strong']!, palette['--bg']!)).toBeGreaterThanOrEqual(1.2);
      expect(contrast(palette['--text']!, palette['--border']!)).toBeGreaterThanOrEqual(4.5);
    });
  }

  it('keeps the accent hue across the theme flip', () => {
    expect(Math.abs(hue(LIGHT['--accent']!) - hue(DARK['--accent']!))).toBeLessThan(12);
  });

  it('exposes a radius token for every radius the design uses', () => {
    // Two, and exactly two: 6px for controls and 10px for panels. A third
    // radius is the first sign of a component pasted in from somewhere else.
    expect(LIGHT['--r-control']).toBe('6px');
    expect(LIGHT['--r-panel']).toBe('10px');
    expect(Object.keys(LIGHT).filter(k => k.startsWith('--r-'))).toEqual([
      '--r-control',
      '--r-panel',
    ]);
  });

  it('carries the three motion durations and the one easing curve', () => {
    expect(LIGHT['--dur-quick']).toBe('100ms');
    expect(LIGHT['--dur-base']).toBe('200ms');
    expect(LIGHT['--dur-slow']).toBe('320ms');
    expect(LIGHT['--ease']).toBe('cubic-bezier(0.165, 0.84, 0.44, 1)');
  });

  it('honours prefers-reduced-motion', () => {
    expect(CSS).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
    expect(CSS).toMatch(/transition-duration: 1ms !important/);
  });

  it('bases the type scale on 15px and gives a row room for a finger', () => {
    expect(LIGHT['--fs-base']).toBe('15px');
    expect(LIGHT['--fs-micro']).toBe('11px');
    expect(LIGHT['--fs-h1']).toBe('24px');
    expect(LIGHT['--ls-micro']).toBe('0.07em');
    expect(LIGHT['--row-h']).toBe('40px');
    expect(CSS).toMatch(/@media \(pointer: coarse\)\s*\{\s*:root\s*\{\s*--row-h: 44px;/);
  });

  it('self-hosts both families under their renamed OFL subsets', () => {
    for (const family of ['Owl Sans', 'Owl Mono']) {
      expect(CSS).toContain(`font-family: '${family}'`);
    }
    // Renamed because subsetting is a modification and the OFL reserves the
    // original names for unmodified files.
    expect(CSS).not.toMatch(/Instrument Sans|JetBrains Mono/);
    expect(CSS).not.toMatch(/fonts\.googleapis\.com|fonts\.gstatic\.com/);
  });

  it('ships every subset beside its unmodified OFL notice', () => {
    for (const notice of NOTICES) {
      expect(notice).toContain('SIL OPEN FONT LICENSE');
      expect(notice).toContain('Reserved Font Name');
    }
  });
});

/*
 * The licence check reads the shipped binaries, not the stylesheet.
 *
 * The stylesheet only proves what the CSS asks for. What the OFL constrains is
 * what is inside the file: nameID 1 is what a font manager shows, 18 is what
 * some systems show, 6 is what a PDF embeds, and an fvar instance carries its
 * own PostScript name that no walk of IDs 1 to 25 would reach.
 */
describe('the shipped font binaries', () => {
  const FONTS = [
    ['sans', 'Owl Sans', 'Instrument Sans'],
    ['mono', 'Owl Mono', 'JetBrains Mono'],
  ] as const;

  // Copyright, trademark and manufacturer. The OFL requires these to survive
  // unchanged: they are attribution, not the font's name.
  const ATTRIBUTION = new Set([0, 7, 8]);

  describe.each(FONTS)('%s', (dir, family, original) => {
    const records = readNameTable(readBinary(`./fonts/${dir}/owl-${dir}.woff2`));

    it('carries the reserved name in no record except attribution', () => {
      const leaks = records
        .filter(r => !ATTRIBUTION.has(r.nameID))
        .filter(r => r.text.includes(original) || r.text.includes(original.replace(/ /g, '')));
      expect(leaks.map(r => `${r.nameID}: ${r.text}`)).toEqual([]);
    });

    it('keeps the upstream attribution intact', () => {
      // The other half of the same licence term. A rename that also wiped the
      // copyright would pass the test above and still breach the OFL.
      const copyright = records.find(r => r.nameID === 0);
      expect(copyright?.text).toContain(original.split(' ')[0]);
    });

    it('renames every record a user or a system reads the family from', () => {
      for (const nid of [1, 4, 6, 16, 18, 21, 25]) {
        for (const record of records.filter(r => r.nameID === nid)) {
          expect(record.text.replace(/[- ]/g, ''), `nameID ${nid}`).toContain(
            family.replace(/[- ]/g, ''),
          );
        }
      }
    });

    it('renames the PostScript name of every named instance', () => {
      const instanceNames = records.filter(r => r.nameID >= 256 && r.text.includes('-'));
      for (const record of instanceNames) {
        expect(record.text).toContain(family.replace(/[- ]/g, ''));
      }
    });
  });
});
