import { describe, expect, it } from 'vitest';

import { DEFAULT_W, renderBoxPlots } from './boxplot';
import { summarise, type Distribution } from './stats';

function rows(): Distribution[] {
  const summary = summarise([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
  if (!summary) throw new Error('fixture should summarise');
  return [{ label: 'a', detail: '10 samples', summary }];
}

/** The `width` in an `<svg width="...">` attribute. */
function widthAttr(svg: string): number {
  const m = /<svg[^>]*\swidth="(\d+)"/.exec(svg);
  if (!m) throw new Error(`no width attribute in: ${svg.slice(0, 120)}`);
  return Number(m[1]);
}

describe('renderBoxPlots sizing', () => {
  it('draws at the width it is given', () => {
    // Real pixel dimensions are what hold the scale factor at 1. A viewBox
    // stretched to fit would scale the 11px labels and the 34px rows along
    // with it — unreadably small in a narrow window, bloated on a wide one.
    const svg = renderBoxPlots(rows(), { format: String, width: 1180 });
    expect(widthAttr(svg)).toBe(1180);
    expect(svg).toContain('viewBox="0 0 1180');
  });

  it('falls back to a usable width when the container has not been measured', () => {
    const svg = renderBoxPlots(rows(), { format: String });
    expect(widthAttr(svg)).toBe(DEFAULT_W);
  });

  it('stops shrinking before the axis labels collide', () => {
    // Below the floor the drawing is scaled down by CSS instead, which keeps
    // the tick labels legible rather than letting them overlap.
    const svg = renderBoxPlots(rows(), { format: String, width: 120 });
    expect(widthAttr(svg)).toBeGreaterThanOrEqual(360);
  });

  it('draws nothing at all when there is nothing to draw', () => {
    expect(renderBoxPlots([], { format: String, width: 800 })).toBe('');
  });
});
