import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { OWL_FACE, OWL_FACE_PATH, OwlMark } from './OwlMark.js';

describe('OwlMark', () => {
  it('names itself, since it is the only thing identifying the app in the header', () => {
    render(<OwlMark />);
    expect(screen.getByRole('img')).toHaveAccessibleName('OwlTransfer');
  });

  it('goes silent beside a visible wordmark rather than saying the name twice', () => {
    const { container } = render(<OwlMark label={null} />);
    expect(screen.queryByRole('img')).toBeNull();
    expect(container.querySelector('svg')).toHaveAttribute('aria-hidden', 'true');
  });

  it('scales from one viewBox, so it stays on the same grid at every size', () => {
    const { container } = render(<OwlMark size={64} />);
    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('viewBox', '0 0 32 32');
    expect(svg).toHaveAttribute('width', '64');
    expect(svg).toHaveAttribute('height', '64');
  });

  it('takes its colour from the text colour around it', () => {
    const { container } = render(<OwlMark />);
    const painted = [...container.querySelectorAll('[fill], [stroke]')].filter(
      el => el.getAttribute('fill') !== 'none' || el.getAttribute('stroke') !== null,
    );
    expect(painted.length).toBeGreaterThan(0);
    for (const el of painted) {
      const paint = el.getAttribute('stroke') ?? el.getAttribute('fill');
      expect(paint).toBe('currentColor');
    }
  });

  it('draws the face from the shared constants, which the icon renderer also reads', () => {
    // The launcher icons are generated from these strings. A mark redrawn here
    // and there is a mark that drifts in one of the two places.
    const { container } = render(<OwlMark />);
    const drawn = [...container.querySelectorAll('path')].map(p => p.getAttribute('d'));
    expect(drawn).toContain(OWL_FACE_PATH.brow);
    expect(drawn).toContain(OWL_FACE_PATH.beak);

    const rings = [...container.querySelectorAll('circle')].filter(
      c => c.getAttribute('fill') === 'none',
    );
    expect(rings.map(c => c.getAttribute('cx'))).toEqual(
      OWL_FACE.eyes.map(eye => String(eye.cx)),
    );
  });
});
