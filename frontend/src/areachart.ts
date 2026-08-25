/**
 * The live bandwidth trace shown while a test runs.
 *
 * speed.cloudflare.com draws each direction as a filled area over the samples
 * as they arrive, with the reported percentile marked as a horizontal line —
 * so you can see the shape of the measurement, not just its final number. A
 * ramp that climbs and holds looks very different from one that spikes and
 * collapses, and only the second is a problem.
 *
 * Samples are plotted by index rather than by timestamp. The engine issues
 * requests back to back but of varying sizes, so a time axis would bunch the
 * small early transfers into a sliver and stretch the large ones; index gives
 * every sample equal width, which is what makes the shape readable.
 *
 * The curve is monotone cubic rather than a polyline or an ordinary spline.
 * See `curve.ts` for why that particular curve.
 */

import { monotonePath, type Point } from './curve';

export interface AreaChartOptions {
  /** Stroke and fill colour. */
  colour: string;
  /** Unique id for the gradient, since several charts share a document. */
  id: string;
  /** Marked with a horizontal line, e.g. the reported 90th percentile. */
  marker?: { value: number; label: string };
}

/** Where a sample sits, as a percentage of the plot box. */
export interface SamplePosition {
  xPct: number;
  yPct: number;
}

export interface AreaChart {
  html: string;
  /**
   * One entry per sample, in the order given.
   *
   * Emitted alongside the markup rather than recomputed by the caller: the
   * hover cursor has to land exactly on the drawn curve, and two
   * implementations of the same scaling would drift apart the first time
   * either changed.
   */
  positions: readonly SamplePosition[];
}

const W = 600;
const H = 150;
const PAD = { top: 12, right: 6, bottom: 6, left: 6 };

const EMPTY: AreaChart = { html: '', positions: [] };

/** Escapes text destined for HTML. */
function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * Renders samples as markup plus their positions, or nothing at all when there
 * is too little to draw — a caller should leave the space blank rather than
 * show an axis with no data on it.
 *
 * Values are expected to be finite and non-negative; the caller filters, so
 * that the positions returned here stay index-aligned with whatever richer
 * per-sample data it holds.
 */
export function renderAreaChart(samples: readonly number[], opts: AreaChartOptions): AreaChart {
  if (samples.length < 2) return EMPTY;

  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;

  // Include the marker so its line cannot fall outside the drawing.
  const peak = Math.max(...samples, opts.marker?.value ?? 0);
  // A little headroom, and never divide by zero on a flat trace.
  const ceiling = peak > 0 ? peak * 1.08 : 1;

  const x = (i: number) => PAD.left + (i / (samples.length - 1)) * plotW;
  const y = (v: number) => PAD.top + plotH - (v / ceiling) * plotH;

  const points: Point[] = samples.map((v, i) => ({ x: x(i), y: y(v) }));
  const line = monotonePath(points);

  // Close the path down to the baseline so the area can be filled.
  const baseline = PAD.top + plotH;
  const area = `${line} L${x(samples.length - 1).toFixed(2)},${baseline.toFixed(
    2,
  )} L${x(0).toFixed(2)},${baseline.toFixed(2)} Z`;

  // The card stretches this drawing to whatever width and height it has, so
  // the marker LINE can live in the SVG (a stroke is scale-independent given
  // `vector-effect`) but its LABEL cannot: text inside a
  // `preserveAspectRatio="none"` viewBox is squashed by whatever the current
  // aspect ratio happens to be, which made it unreadable in a narrow window
  // and oversized in a wide one. Position the label as HTML instead, where a
  // CSS font size means what it says at every window size.
  let markerLine = '';
  let markerLabel = '';
  if (opts.marker && opts.marker.value > 0) {
    const my = y(opts.marker.value);
    markerLine = `<line x1="${PAD.left}" y1="${my.toFixed(1)}" x2="${W - PAD.right}" y2="${my.toFixed(
      1,
    )}" class="trace__marker" />`;
    // The label sits above its line, so a marker near the top of a short
    // chart would push it out of the card. 18% clears one line of text at the
    // smallest height `--trace-h` allows.
    const top = Math.min(97, Math.max(18, (my / H) * 100));
    markerLabel = `<span class="trace__marker-label" style="top: ${top.toFixed(2)}%">${esc(
      opts.marker.label,
    )}</span>`;
  }

  const html = `<div class="trace__plot">
    <svg viewBox="0 0 ${W} ${H}" class="trace__svg" preserveAspectRatio="none"
         role="img" aria-label="Bandwidth over the course of the test">
      <defs>
        <linearGradient id="${opts.id}" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color="${opts.colour}" stop-opacity="0.45" />
          <stop offset="100%" stop-color="${opts.colour}" stop-opacity="0.02" />
        </linearGradient>
      </defs>
      <path d="${area}" fill="url(#${opts.id})" stroke="none" />
      <path d="${line}" fill="none" stroke="${opts.colour}" stroke-width="1.5"
            stroke-linejoin="round" stroke-linecap="round" vector-effect="non-scaling-stroke" />
      ${markerLine}
    </svg>
    ${markerLabel}
    <div class="trace__cursor" hidden>
      <span class="trace__cursor-line"></span>
      <span class="trace__cursor-dot" style="--dot: ${opts.colour}"></span>
    </div>
    <div class="trace__tip" hidden></div>
  </div>`;

  return {
    html,
    positions: points.map((p) => ({ xPct: (p.x / W) * 100, yPct: (p.y / H) * 100 })),
  };
}
