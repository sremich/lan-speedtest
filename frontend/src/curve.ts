/**
 * Monotone cubic interpolation, for drawing measurement series as a curve.
 *
 * A polyline through measurement samples is honest but hard to read; a smooth
 * curve makes the shape of a run legible at a glance. The choice of curve
 * matters more than it looks. An ordinary Catmull-Rom or cardinal spline
 * *overshoots* between points, so a chart of a link that ramped from 0 to
 * 900 Mbps would be drawn dipping below zero on the way up and peaking above
 * the fastest sample ever recorded — a curve that is smooth and lying.
 *
 * This is the Fritsch-Carlson / Steffen construction (the same one d3 calls
 * `curveMonotoneX`): tangents are clamped so that each segment stays inside
 * the range of its own two endpoints. The curve passes exactly through every
 * sample and invents nothing between them.
 *
 * x must be strictly increasing, which it is for every series here — they are
 * plotted by sample index.
 */

export interface Point {
  x: number;
  y: number;
}

/** `Math.sign` but with no zero case, matching the Steffen formulation. */
function sign(x: number): number {
  return x < 0 ? -1 : 1;
}

/**
 * Tangent at an interior sample.
 *
 * The `(sign(s0) + sign(s1))` factor is what enforces monotonicity: when the
 * neighbouring secants disagree in direction the sample is a local extremum,
 * the tangent is forced flat, and the curve cannot overshoot past it.
 */
function interiorSlope(p0: Point, p1: Point, p2: Point): number {
  const h0 = p1.x - p0.x;
  const h1 = p2.x - p1.x;
  if (h0 === 0 || h1 === 0) return 0;

  const s0 = (p1.y - p0.y) / h0;
  const s1 = (p2.y - p1.y) / h1;
  const parabolic = (s0 * h1 + s1 * h0) / (h0 + h1);

  const m =
    (sign(s0) + sign(s1)) * Math.min(Math.abs(s0), Math.abs(s1), 0.5 * Math.abs(parabolic));
  return Number.isFinite(m) ? m : 0;
}

/** Tangent at an endpoint, given the tangent at its inward neighbour. */
function endSlope(end: Point, inward: Point, inwardSlope: number): number {
  const h = inward.x - end.x;
  if (h === 0) return inwardSlope;
  const m = (3 * ((inward.y - end.y) / h) - inwardSlope) / 2;
  return Number.isFinite(m) ? m : 0;
}

function n(v: number): string {
  return v.toFixed(2);
}

/**
 * An SVG path through every point, as cubic Béziers.
 *
 * Returns an empty string for no points, and falls back to a straight line for
 * two — a curve needs three samples before it has a shape to describe.
 */
export function monotonePath(points: readonly Point[]): string {
  const count = points.length;
  if (count === 0) return '';

  const first = points[0]!;
  if (count === 1) return `M${n(first.x)},${n(first.y)}`;
  if (count === 2) {
    const second = points[1]!;
    return `M${n(first.x)},${n(first.y)} L${n(second.x)},${n(second.y)}`;
  }

  const slopes = new Array<number>(count);
  for (let i = 1; i < count - 1; i += 1) {
    slopes[i] = interiorSlope(points[i - 1]!, points[i]!, points[i + 1]!);
  }
  slopes[0] = endSlope(first, points[1]!, slopes[1]!);
  slopes[count - 1] = endSlope(points[count - 1]!, points[count - 2]!, slopes[count - 2]!);

  let d = `M${n(first.x)},${n(first.y)}`;
  for (let i = 0; i < count - 1; i += 1) {
    const a = points[i]!;
    const b = points[i + 1]!;
    // Control points a third of the way along, riding each endpoint's tangent.
    const dx = (b.x - a.x) / 3;
    d +=
      ` C${n(a.x + dx)},${n(a.y + dx * slopes[i]!)}` +
      ` ${n(b.x - dx)},${n(b.y - dx * slopes[i + 1]!)}` +
      ` ${n(b.x)},${n(b.y)}`;
  }
  return d;
}
