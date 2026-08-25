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
 */

export interface AreaChartOptions {
  /** Stroke and fill colour. */
  colour: string;
  /** Unique id for the gradient, since several charts share a document. */
  id: string;
  /** Marked with a horizontal line, e.g. the reported 90th percentile. */
  marker?: { value: number; label: string };
  /** Formats a value for the axis label. */
  format: (value: number) => string;
}

const W = 600;
const H = 150;
const PAD = { top: 12, right: 6, bottom: 6, left: 6 };

/**
 * Renders samples as SVG markup, or an empty string when there is nothing yet
 * to draw — a caller should leave the space blank rather than show an axis
 * with no data on it.
 */
export function renderAreaChart(samples: readonly number[], opts: AreaChartOptions): string {
  const usable = samples.filter((v) => Number.isFinite(v) && v >= 0);
  if (usable.length < 2) return '';

  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;

  // Include the marker so its line cannot fall outside the drawing.
  const peak = Math.max(...usable, opts.marker?.value ?? 0);
  // A little headroom, and never divide by zero on a flat trace.
  const ceiling = peak > 0 ? peak * 1.08 : 1;

  const x = (i: number) => PAD.left + (i / (usable.length - 1)) * plotW;
  const y = (v: number) => PAD.top + plotH - (v / ceiling) * plotH;

  const line = usable
    .map((v, i) => `${i === 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(v).toFixed(1)}`)
    .join(' ');

  // Close the path down to the baseline so the area can be filled.
  const area = `${line} L${x(usable.length - 1).toFixed(1)},${(PAD.top + plotH).toFixed(
    1,
  )} L${x(0).toFixed(1)},${(PAD.top + plotH).toFixed(1)} Z`;

  let markerMarkup = '';
  if (opts.marker && opts.marker.value > 0) {
    const my = y(opts.marker.value);
    markerMarkup = `
      <line x1="${PAD.left}" y1="${my.toFixed(1)}" x2="${W - PAD.right}" y2="${my.toFixed(1)}"
            class="trace__marker" />
      <text x="${PAD.left + 2}" y="${(my - 4).toFixed(1)}" class="trace__marker-label">${
        opts.marker.label
      }</text>`;
  }

  return `<svg viewBox="0 0 ${W} ${H}" class="trace__svg" preserveAspectRatio="none"
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
    ${markerMarkup}
  </svg>`;
}
