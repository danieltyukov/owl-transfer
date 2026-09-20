// @vitest-environment node
// Reads the stylesheets off disk rather than rendering them, so it opts out of
// jsdom to get a real `import.meta.url`.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/*
 * The rules the token sheet exists to enforce, checked against every stylesheet
 * that ships. They are invariants rather than preferences: each one is a thing
 * that looks fine in the file it is written in and wrong in the theme nobody
 * had open at the time.
 */

const ROOT = fileURLToPath(new URL('.', import.meta.url));

function stylesheets(dir: string): string[] {
  const out: string[] = [];
  for (const item of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, item.name);
    if (item.isDirectory()) out.push(...stylesheets(path));
    else if (item.name.endsWith('.css')) out.push(path);
  }
  return out;
}

const SHEETS = stylesheets(ROOT)
  .map(path => ({ name: path.slice(ROOT.length), text: readFileSync(path, 'utf8') }))
  .map(sheet => ({ ...sheet, code: sheet.text.replace(/\/\*[\s\S]*?\*\//g, '') }));

const COMPONENTS = SHEETS.filter(s => s.name !== 'tokens.css');

describe('the stylesheets', () => {
  it('finds more than a couple, so a broken walk cannot pass silently', () => {
    expect(SHEETS.length).toBeGreaterThan(10);
    expect(SHEETS.map(s => s.name)).toContain('tokens.css');
  });

  it('writes every colour in tokens.css and nowhere else', () => {
    for (const sheet of COMPONENTS) {
      const literals = sheet.code.match(/#[0-9a-fA-F]{3,8}\b|\brgba?\(/g) ?? [];
      expect(literals, sheet.name).toEqual([]);
    }
  });

  it('takes every size from the type scale', () => {
    for (const sheet of SHEETS) {
      const sizes = [...sheet.code.matchAll(/font-size:\s*([^;]+);/g)].map(m => m[1]!.trim());
      const off = sizes.filter(size => !size.startsWith('var(--fs-'));
      expect(off, sheet.name).toEqual([]);
    }
  });

  it('rounds to the two radii, a circle or a pill and nothing between', () => {
    for (const sheet of SHEETS) {
      const radii = [...sheet.code.matchAll(/border-radius:\s*([^;]+);/g)].map(m => m[1]!.trim());
      const off = radii.filter(
        radius => !['var(--r-control)', 'var(--r-panel)', '50%', '999px'].includes(radius),
      );
      expect(off, sheet.name).toEqual([]);
    }
  });

  it('casts a shadow only where something is above the page', () => {
    // An inset shadow is a ring or a wash over a fill, not elevation. Only the
    // dialog and the row menu lift off the page, and both take the same token.
    for (const sheet of SHEETS) {
      const shadows = [...sheet.code.matchAll(/box-shadow:\s*([^;]+);/g)].map(m => m[1]!.trim());
      const off = shadows.filter(
        shadow => !shadow.startsWith('inset') && !['var(--shadow-pop)', 'none'].includes(shadow),
      );
      expect(off, sheet.name).toEqual([]);
    }
  });

  it('times every transition from the three duration tokens', () => {
    for (const sheet of SHEETS) {
      const timed = [...sheet.code.matchAll(/(?:transition|animation):\s*([^;]+);/g)].map(m =>
        m[1]!.trim(),
      );
      const off = timed.filter(value => !value.includes('var(--dur-'));
      expect(off, sheet.name).toEqual([]);
    }
  });

  it('never sets words in the accent, which only clears 3:1', () => {
    // Amber is a user interface colour here: the focus ring, the in-flight arc,
    // the current-pane tint, the mark. Anything that wants amber words takes
    // accent-hover, which the token test holds at 4.5:1 in both themes.
    for (const sheet of COMPONENTS) {
      for (const [, selector] of sheet.code.matchAll(
        // The lookbehind keeps `border-color` and `background-color` out of it.
        /([^{}]+)\{[^{}]*(?<![-\w])color:\s*var\(--accent\)\s*;/g,
      )) {
        // The brand mark is a drawing, not a line of text.
        expect(selector!.trim(), sheet.name).toMatch(/-(mark|brand)\b/);
      }
    }
  });

  it('switches to the phone layout at one width', () => {
    const breakpoints = new Set<string>();
    for (const sheet of SHEETS) {
      for (const [, width] of sheet.code.matchAll(/@media \(max-width:\s*(\d+)px\)/g)) {
        breakpoints.add(width!);
      }
    }
    // 899 is the pane switch. The two below it are content giving way on a
    // narrow phone, not a second layout.
    expect([...breakpoints].sort()).toEqual(['480', '560', '899']);
  });
});
