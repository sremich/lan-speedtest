/**
 * Rendering a run — the traces, their hover detail, and the distributions.
 *
 * Shared by the live page and the permalink. The live page holds an engine
 * `Results` object; the permalink holds JSON that came back out of the
 * database. Neither shape belongs in a renderer, so both are adapted to the
 * plain `RunSamples` below and the drawing happens once.
 *
 * That sharing is the point: a stored run should look identical to the run you
 * just watched, and it only stays identical if there is one implementation.
 */

import { renderAreaChart, type SamplePosition } from './areachart';
import { BOXPLOT_LEGEND, boxTipData, boxTipMarkup, renderBoxPlots } from './boxplot';
import { formatBandwidth } from './format';
import { formatPayload } from './progress';
import { bandwidthBySize, formatTransferSize, summarise, type Distribution } from './stats';

/** One bandwidth sample, in the shape the engine reports and we store. */
export interface Sample {
  bps: number;
  bytes: number;
  ping?: number;
  duration?: number;
}

/** Everything a run collected, independent of where it came from. */
export interface RunSamples {
  download: Sample[];
  upload: Sample[];
  idleLatency: number[];
  downLoadedLatency: number[];
  upLoadedLatency: number[];
  packetLoss?: number;
}

export const EMPTY_SAMPLES: RunSamples = {
  download: [],
  upload: [],
  idleLatency: [],
  downLoadedLatency: [],
  upLoadedLatency: [],
};

export type TraceKey = 'download' | 'upload';

/**
 * What a trace needs to answer a hover: where it drew each sample, and what
 * that sample actually was. Kept together so the two cannot drift apart.
 */
interface TraceState {
  positions: readonly SamplePosition[];
  points: Sample[];
  colour: string;
  label: string;
}

const traces: Record<TraceKey, TraceState> = {
  download: { positions: [], points: [], colour: 'var(--accent)', label: 'Download' },
  upload: { positions: [], points: [], colour: 'var(--upload)', label: 'Upload' },
};

/**
 * Draws one trace.
 *
 * `reported` is the figure the headline shows, drawn as a marker line so the
 * number is locatable within the shape of the samples rather than an
 * unexplained summary of them.
 */
export function drawTrace(
  key: TraceKey,
  host: HTMLElement,
  raw: readonly Sample[],
  reported: number | undefined,
): void {
  const state = traces[key];
  // Filtered here rather than inside the renderer, so the positions it hands
  // back stay index-aligned with the samples the tooltip reads.
  const points = raw.filter((p) => Number.isFinite(p.bps) && p.bps >= 0);

  const chart = renderAreaChart(
    points.map((p) => p.bps),
    {
      colour: state.colour,
      id: `grad-${key}`,
      ...(reported !== undefined ? { marker: { value: reported, label: '90th percentile' } } : {}),
    },
  );

  host.innerHTML = chart.html;
  state.positions = chart.positions;
  state.points = points;
}

/**
 * Hover on a trace: a guide line, a dot on the curve, and what that sample
 * actually was.
 *
 * The engine records far more per sample than the headline uses — the payload
 * size, the round trip, how long the request took — and a single point on the
 * curve is where that detail belongs.
 */
export function attachTraceHover(key: TraceKey, host: HTMLElement): void {
  host.addEventListener('pointermove', (event) => showTraceTip(key, host, event));
  host.addEventListener('pointerleave', () => {
    for (const node of host.querySelectorAll<HTMLElement>('.trace__cursor, .trace__tip')) {
      node.hidden = true;
    }
  });
}

function showTraceTip(key: TraceKey, host: HTMLElement, event: PointerEvent): void {
  const state = traces[key];
  const plot = host.querySelector<HTMLElement>('.trace__plot');
  const cursor = host.querySelector<HTMLElement>('.trace__cursor');
  const tip = host.querySelector<HTMLElement>('.trace__tip');
  if (!plot || !cursor || !tip || state.positions.length === 0) return;

  const box = plot.getBoundingClientRect();
  if (box.width === 0) return;
  const fraction = (event.clientX - box.left) / box.width;

  let nearest = 0;
  let best = Infinity;
  state.positions.forEach((p, i) => {
    const distance = Math.abs(p.xPct / 100 - fraction);
    if (distance < best) {
      best = distance;
      nearest = i;
    }
  });

  const position = state.positions[nearest]!;
  const point = state.points[nearest];
  if (!point) return;

  const line = cursor.querySelector<HTMLElement>('.trace__cursor-line');
  const dot = cursor.querySelector<HTMLElement>('.trace__cursor-dot');
  if (line) line.style.left = `${position.xPct}%`;
  if (dot) {
    dot.style.left = `${position.xPct}%`;
    dot.style.top = `${position.yPct}%`;
  }
  cursor.hidden = false;

  tip.innerHTML = traceTipMarkup(state, point);
  tip.hidden = false;
  placeTip(tip, plot, (position.xPct / 100) * box.width, (position.yPct / 100) * box.height);
}

export function tipRow(key: string, value: string): string {
  return `<div><span class="tip__key">${key}:</span> <span class="tip__value">${value}</span></div>`;
}

function traceTipMarkup(state: TraceState, point: Sample): string {
  const speed = formatBandwidth(point.bps);
  const rows = [tipRow('Speed', `${speed.value} ${speed.unit}`)];
  if (Number.isFinite(point.bytes) && point.bytes > 0) {
    rows.push(tipRow('Payload', formatPayload(point.bytes)));
  }
  if (point.ping !== undefined && Number.isFinite(point.ping) && point.ping > 0) {
    rows.push(tipRow('Round trip', `${point.ping.toFixed(2)} ms`));
  }
  if (point.duration !== undefined && Number.isFinite(point.duration) && point.duration > 0) {
    rows.push(tipRow('Request took', `${point.duration.toFixed(0)} ms`));
  }
  return (
    `<div class="tip__head" style="--swatch: ${state.colour}">` +
    `<span class="tip__swatch"></span>${state.label}</div>${rows.join('')}`
  );
}

/**
 * Places a tooltip near a point without letting it escape its container.
 *
 * Measured after the content is set, because its size depends on the text.
 * Prefers to sit above the thing it describes and flips below when there is
 * no room — otherwise it clamps to the top edge and covers the controls.
 */
export function placeTip(
  tip: HTMLElement,
  within: HTMLElement,
  x: number,
  y: number,
  anchorHeight = 0,
): void {
  tip.style.left = '0px';
  tip.style.top = '0px';

  const maxX = Math.max(0, within.clientWidth - tip.offsetWidth);
  const maxY = Math.max(0, within.clientHeight - tip.offsetHeight);
  const above = y - tip.offsetHeight - 10;

  tip.style.left = `${Math.max(0, Math.min(maxX, x + 12))}px`;
  tip.style.top = `${Math.max(0, Math.min(maxY, above >= 0 ? above : y + anchorHeight + 10))}px`;
}

/** Formats bits per second for an axis or tooltip. */
export function bps(value: number): string {
  const f = formatBandwidth(value);
  return `${f.value} ${f.unit}`;
}

export function ms(value: number): string {
  return `${value.toFixed(2)} ms`;
}

/**
 * The width the box plots should be drawn at, in CSS pixels.
 *
 * A collapsed `<details>` reports zero, so fall back rather than emitting a
 * drawing of width zero.
 */
function detailWidth(host: HTMLElement): number {
  const style = getComputedStyle(host);
  const inner = host.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  return Number.isFinite(inner) && inner > 0 ? inner : 760;
}

/**
 * The per-measurement distributions.
 *
 * The headline figures are single percentiles, which say nothing about how
 * consistent a run was — and consistency is exactly what exposes a failing
 * cable or a duplex mismatch. These show every sample the run collected.
 */
export function renderDetail(host: HTMLElement, samples: RunSamples): void {
  const groups: Array<{ title: string; rows: Distribution[]; format: (v: number) => string }> = [];
  let packetLossBar = '';

  const download = bandwidthBySize(samples.download, formatTransferSize);
  if (download.length > 0) {
    groups.push({ title: 'Download, by transfer size', rows: download, format: bps });
  }

  const upload = bandwidthBySize(samples.upload, formatTransferSize);
  if (upload.length > 0) {
    groups.push({ title: 'Upload, by transfer size', rows: upload, format: bps });
  }

  const latencyRows: Distribution[] = [];
  const latencySeries: Array<[string, number[]]> = [
    ['Idle', samples.idleLatency],
    ['Loaded ↓', samples.downLoadedLatency],
    ['Loaded ↑', samples.upLoadedLatency],
  ];
  for (const [label, points] of latencySeries) {
    const summary = summarise(points);
    if (summary) {
      latencyRows.push({
        label: `${label} (${summary.count})`,
        detail: `${summary.count} ping${summary.count === 1 ? '' : 's'}`,
        summary,
      });
    }
  }

  // Packet loss as a received/lost bar, which reads at a glance in a way a
  // percentage does not — and shows the sample size, since 0% of 20 packets
  // and 0% of 1000 are different claims.
  const loss = samples.packetLoss;
  if (loss !== undefined && Number.isFinite(loss)) {
    const received = Math.max(0, Math.min(1, 1 - loss));
    packetLossBar = `<div class="box__group">
      <h3 class="box__group-title">Packet loss</h3>
      <div class="loss-bar">
        <div class="loss-bar__received" style="width: ${(received * 100).toFixed(2)}%">
          ${received >= 0.08 ? `Received ${(received * 100).toFixed(received === 1 ? 0 : 1)}%` : ''}
        </div>
      </div>
    </div>`;
  }

  if (latencyRows.length > 0) {
    // Not zero-based: on a LAN every value sits near zero, and forcing the
    // axis to the origin would flatten the whole distribution into a smear.
    groups.push({ title: 'Latency', rows: latencyRows, format: ms });
  }

  if (groups.length === 0 && packetLossBar === '') {
    host.innerHTML = '<p class="note">No samples collected yet.</p>';
    return;
  }

  const width = detailWidth(host);
  host.innerHTML =
    `<div class="box__plots">` +
    groups
      .map(
        (g) => `<div class="box__group">
          <h3 class="box__group-title">${g.title}</h3>
          ${renderBoxPlots(g.rows, {
            format: g.format,
            zeroBased: g.format === bps,
            width,
          })}
        </div>`,
      )
      .join('') +
    packetLossBar +
    BOXPLOT_LEGEND +
    `<div class="tip box__tip" data-testid="box-tip" hidden></div></div>`;

  attachBoxHover(host);
}

/**
 * Hover on a distribution row.
 *
 * Delegated from the panel rather than bound per row, because the panel is
 * re-rendered on every resize and on every results change — per-row listeners
 * would have to be torn down each time, and one missed teardown is a leak
 * that only shows up after a long run.
 */
function attachBoxHover(host: HTMLElement): void {
  const tip = host.querySelector<HTMLElement>('.box__tip');
  const within = host.querySelector<HTMLElement>('.box__plots');
  if (!tip || !within) return;

  host.addEventListener('pointermove', (event) => {
    const target = event.target as Element | null;
    const row = target?.closest<SVGGElement>('g.box__row');
    if (!row) {
      tip.hidden = true;
      highlightRow(host, null);
      return;
    }

    highlightRow(host, row);
    tip.innerHTML = boxTipMarkup(boxTipData(row as unknown as { dataset: DOMStringMap }));
    tip.hidden = false;

    const box = within.getBoundingClientRect();
    placeTip(tip, within, event.clientX - box.left, event.clientY - box.top);
  });

  host.addEventListener('pointerleave', () => {
    tip.hidden = true;
    highlightRow(host, null);
  });
}

/** Marks the row the tooltip is describing, so it is anchored to something. */
function highlightRow(host: HTMLElement, row: SVGGElement | null): void {
  for (const g of host.querySelectorAll<SVGGElement>('g.box__row.is-hovered')) {
    if (g !== row) g.classList.remove('is-hovered');
  }
  row?.classList.add('is-hovered');
}
