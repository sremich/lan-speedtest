/**
 * Horizontal box plots for the detail view.
 *
 * One row per distribution: whiskers to the furthest sample inside the Tukey
 * fence, a box from p25 to p75, a median line, a mean marker, and a dot per
 * outlier. Rows in a group share one scale so they can be compared by eye.
 *
 * Hand-drawn SVG, like the history chart, for the same reason: a charting
 * library would be more bundle than this page's two visualisations justify.
 */

import type { Distribution } from './stats';

export interface BoxPlotOptions {
  /** Formats a value for a tick or tooltip, e.g. "940 Mbps". */
  format: (value: number) => string;
  /** Force the axis to start at zero. Right for bandwidth, wrong for latency. */
  zeroBased?: boolean;
  /**
   * Drawing width in CSS pixels — normally the container's measured width.
   *
   * A fixed viewBox scaled to fit by CSS would scale the *text* and the row
   * heights along with it, so the same plot rendered as cramped 8px labels in
   * a narrow window and bloated 20px ones on a wide monitor. Drawing at the
   * real pixel width keeps one row 34px tall and one label 11px whatever the
   * window is doing.
   */
  width?: number;
}

const ROW_H = 34;
/** Minimum drawn box width, so a zero-spread distribution stays visible. */
export const MIN_BOX_W = 2;
const LABEL_W = 96;
const PAD = { top: 10, right: 16, bottom: 26 };
/** Used when no container width is known, e.g. in unit tests. */
export const DEFAULT_W = 760;
/** Below this the axis labels start colliding, so stop shrinking and scroll. */
const MIN_W = 360;

/** Escapes text destined for an SVG attribute or text node. */
function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * Renders a group of distributions as SVG markup.
 *
 * Returns an empty string when there is nothing to draw, so a caller can hide
 * the section rather than showing an empty frame.
 */
export function renderBoxPlots(rows: Distribution[], opts: BoxPlotOptions): string {
  if (rows.length === 0) return '';

  const W = Math.max(MIN_W, Math.round(opts.width ?? DEFAULT_W));
  const H = PAD.top + rows.length * ROW_H + PAD.bottom;
  const plotL = LABEL_W;
  const plotW = W - plotL - PAD.right;

  // One shared scale across the group, including outliers — an outlier drawn
  // off the edge is worse than a slightly compressed box.
  const all = rows.flatMap((r) => [r.summary.min, r.summary.max, ...r.summary.outliers]);
  let lo = Math.min(...all);
  let hi = Math.max(...all);

  if (opts.zeroBased) lo = 0;
  if (hi === lo) {
    // A single value, or a perfectly flat set: give it room so the box is
    // visible rather than collapsing to a line at the edge.
    hi = lo === 0 ? 1 : lo * 1.1;
    if (!opts.zeroBased) lo = Math.max(0, lo * 0.9);
  }
  // A little headroom so the largest sample is not flush against the frame.
  const span = hi - lo;
  hi += span * 0.04;

  const x = (v: number) => plotL + ((v - lo) / (hi - lo)) * plotW;

  const ticks = [0, 0.5, 1]
    .map((f) => {
      const value = lo + f * (hi - lo);
      const tx = x(value);
      return `<line x1="${tx.toFixed(1)}" y1="${PAD.top}" x2="${tx.toFixed(1)}" y2="${(
        H - PAD.bottom
      ).toFixed(1)}" class="box__grid" />
        <text x="${tx.toFixed(1)}" y="${H - 8}" class="box__tick" text-anchor="${
          f === 0 ? 'start' : f === 1 ? 'end' : 'middle'
        }">${esc(opts.format(value))}</text>`;
    })
    .join('');

  const body = rows
    .map((row, i) => {
      const s = row.summary;
      const cy = PAD.top + i * ROW_H + ROW_H / 2;
      const boxTop = cy - 9;
      const boxH = 18;

      // A perfectly consistent set has p25 === p75, so the box would be
      // invisible. Give it a minimum width, but centre that on the median
      // rather than anchoring it at p25 — otherwise the degenerate box sits
      // beside the whisker instead of on it, which reads as a real offset.
      const rawW = x(s.p75) - x(s.p25);
      const boxW = Math.max(MIN_BOX_W, rawW);
      const boxX = rawW >= MIN_BOX_W ? x(s.p25) : x(s.median) - MIN_BOX_W / 2;

      const tooltip = [
        `${row.label} — ${row.detail}`,
        `min ${opts.format(s.min)}`,
        `p25 ${opts.format(s.p25)}`,
        `median ${opts.format(s.median)}`,
        `mean ${opts.format(s.mean)}`,
        `p75 ${opts.format(s.p75)}`,
        `max ${opts.format(s.max)}`,
        s.outliers.length > 0
          ? `${s.outliers.length} outlier${s.outliers.length === 1 ? '' : 's'}`
          : 'no outliers',
      ].join('\n');

      const outlierDots = s.outliers
        .map(
          (o) =>
            `<circle cx="${x(o).toFixed(1)}" cy="${cy.toFixed(1)}" r="2.5" class="box__outlier" />`,
        )
        .join('');

      return `<g class="box__row">
        <title>${esc(tooltip)}</title>
        <text x="0" y="${(cy + 4).toFixed(1)}" class="box__label">${esc(row.label)}</text>

        <line x1="${x(s.lowerWhisker).toFixed(1)}" y1="${cy.toFixed(1)}"
              x2="${x(s.upperWhisker).toFixed(1)}" y2="${cy.toFixed(1)}" class="box__whisker" />
        <line x1="${x(s.lowerWhisker).toFixed(1)}" y1="${(cy - 6).toFixed(1)}"
              x2="${x(s.lowerWhisker).toFixed(1)}" y2="${(cy + 6).toFixed(1)}" class="box__cap" />
        <line x1="${x(s.upperWhisker).toFixed(1)}" y1="${(cy - 6).toFixed(1)}"
              x2="${x(s.upperWhisker).toFixed(1)}" y2="${(cy + 6).toFixed(1)}" class="box__cap" />

        <rect x="${boxX.toFixed(1)}" y="${boxTop.toFixed(1)}"
              width="${boxW.toFixed(1)}" height="${boxH}" rx="2" class="box__box" />
        <line x1="${x(s.median).toFixed(1)}" y1="${boxTop.toFixed(1)}"
              x2="${x(s.median).toFixed(1)}" y2="${(boxTop + boxH).toFixed(1)}" class="box__median" />
        <circle cx="${x(s.mean).toFixed(1)}" cy="${cy.toFixed(1)}" r="2.5" class="box__mean" />
        ${outlierDots}
      </g>`;
    })
    .join('');

  // Explicit pixel dimensions, not just a viewBox: they are what hold the
  // scale factor at 1. CSS caps the width so an over-generous measurement
  // shrinks the drawing rather than overflowing the card.
  return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" class="box__svg" role="img"
               aria-label="Distribution of samples per measurement">
    ${ticks}
    ${body}
  </svg>`;
}

/** The shared key explaining what the marks mean. */
export const BOXPLOT_LEGEND = `
  <div class="box__legend">
    <span class="box__key box__key--box">25th–75th percentile</span>
    <span class="box__key box__key--median">Median</span>
    <span class="box__key box__key--mean">Mean</span>
    <span class="box__key box__key--whisker">Min–max within 1.5×IQR</span>
    <span class="box__key box__key--outlier">Outlier</span>
  </div>`;
