import { describe, expect, it } from 'vitest';

import { monotonePath, type Point } from './curve';

/** Parses a path of one M followed by C segments into its raw numbers. */
function segments(d: string): Array<{ c1: Point; c2: Point; end: Point }> {
  const start = /^M(-?[\d.]+),(-?[\d.]+)/.exec(d);
  expect(start, `no move-to in ${d}`).not.toBeNull();
  const out: Array<{ c1: Point; c2: Point; end: Point }> = [];
  const re = /C(-?[\d.]+),(-?[\d.]+) (-?[\d.]+),(-?[\d.]+) (-?[\d.]+),(-?[\d.]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(d)) !== null) {
    out.push({
      c1: { x: Number(m[1]), y: Number(m[2]) },
      c2: { x: Number(m[3]), y: Number(m[4]) },
      end: { x: Number(m[5]), y: Number(m[6]) },
    });
  }
  return out;
}

/** Evaluates a cubic Bézier's y at parameter t. */
function bezierY(y0: number, y1: number, y2: number, y3: number, t: number): number {
  const u = 1 - t;
  return u * u * u * y0 + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t * t * t * y3;
}

/** Every y the curve actually passes through, sampled densely. */
function sampleCurve(points: readonly Point[]): Array<{ segment: number; y: number }> {
  const segs = segments(monotonePath(points));
  const out: Array<{ segment: number; y: number }> = [];
  segs.forEach((s, i) => {
    const y0 = points[i]!.y;
    for (let step = 0; step <= 40; step += 1) {
      out.push({ segment: i, y: bezierY(y0, s.c1.y, s.c2.y, s.end.y, step / 40) });
    }
  });
  return out;
}

const indexed = (ys: number[]): Point[] => ys.map((y, x) => ({ x, y }));

describe('monotonePath', () => {
  it('passes exactly through every sample', () => {
    // Interpolation, not approximation: a chart that misses its own data
    // points is not a chart of that data.
    const points = indexed([10, 40, 35, 90, 20, 60]);
    const segs = segments(monotonePath(points));
    expect(segs).toHaveLength(points.length - 1);
    segs.forEach((s, i) => {
      expect(s.end.x).toBeCloseTo(points[i + 1]!.x, 1);
      expect(s.end.y).toBeCloseTo(points[i + 1]!.y, 1);
    });
  });

  it('never overshoots the range of a segment', () => {
    // The reason this curve was chosen over a cardinal spline. An overshoot on
    // a bandwidth chart draws a dip below zero on a ramp-up, and a peak above
    // the fastest sample ever recorded — smooth, and false.
    const points = indexed([0, 0, 5, 900, 905, 900, 40, 0, 0]);
    for (const { segment, y } of sampleCurve(points)) {
      const lo = Math.min(points[segment]!.y, points[segment + 1]!.y);
      const hi = Math.max(points[segment]!.y, points[segment + 1]!.y);
      expect(y, `segment ${segment} left [${lo}, ${hi}] at y=${y}`).toBeGreaterThanOrEqual(
        lo - 1e-6,
      );
      expect(y).toBeLessThanOrEqual(hi + 1e-6);
    }
  });

  it('keeps a rising series rising', () => {
    const points = indexed([1, 2, 3, 10, 11, 50, 51]);
    const ys = sampleCurve(points).map((s) => s.y);
    for (let i = 1; i < ys.length; i += 1) {
      expect(ys[i]!, `fell at ${i}`).toBeGreaterThanOrEqual(ys[i - 1]! - 1e-6);
    }
  });

  it('stays flat through a flat series', () => {
    // A perfectly steady link should not be drawn as gently undulating.
    for (const { y } of sampleCurve(indexed([500, 500, 500, 500, 500]))) {
      expect(y).toBeCloseTo(500, 6);
    }
  });

  it('degrades sensibly below three points', () => {
    expect(monotonePath([])).toBe('');
    expect(monotonePath([{ x: 1, y: 2 }])).toBe('M1.00,2.00');
    // Two samples describe a line and nothing more; drawing a curve through
    // them would be inventing a shape.
    expect(monotonePath(indexed([3, 7]))).toBe('M0.00,3.00 L1.00,7.00');
  });

  it('does not emit NaN when samples repeat an x', () => {
    // Defensive: a degenerate series must produce a drawable path rather than
    // silently poisoning the whole SVG.
    const d = monotonePath([
      { x: 0, y: 1 },
      { x: 0, y: 5 },
      { x: 1, y: 3 },
    ]);
    expect(d).not.toMatch(/NaN|Infinity/);
  });
});
