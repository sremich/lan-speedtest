/**
 * Two runs, side by side.
 *
 * The question this page exists to answer is "did that change help?" — a new
 * access point, SQM turned on, a cable replaced, the same laptop over the VPN
 * instead of the wire. A single run cannot answer it and two permalinks in two
 * tabs answer it badly, because the interesting part is the difference and the
 * eye is poor at differencing two columns of formatted numbers.
 *
 * So the difference is computed rather than displayed and left to the reader,
 * and it is signed by *improvement* rather than by arithmetic: less latency is
 * better, more bandwidth is better, and a table that coloured both the same way
 * would be actively misleading.
 */

import './styles.css';
import { setUpThemeToggle } from './theme';
import { fetchResult, fetchStatus, type StoredRunDetail } from './api';
import {
  formatBandwidth,
  formatLatency,
  formatPacketLoss,
  latencyPredatesNagleFix,
} from './format';
import { changePct, formatChange, verdictFor } from './comparemath';
import { drawTrace, samplesFromStored } from './runview';

const el = <T extends HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

const ui = {
  title: el('page-title'),
  error: el('error'),
  headA: el('head-a'),
  headB: el('head-b'),
  rows: el('rows'),
  downA: el('down-a'),
  downB: el('down-b'),
  upA: el('up-a'),
  upB: el('up-b'),
  caveat: el('caveat'),
};

function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * One comparable figure.
 *
 * `higherIsBetter` is the whole point of the type: it is what lets the same
 * code render a 12% bandwidth gain and a 12% latency rise as opposite things.
 */
interface Metric {
  label: string;
  of: (run: StoredRunDetail) => number | null;
  format: (v: number | null) => string;
  higherIsBetter: boolean;
  /** True when the Nagle bug could have moved this figure. */
  latencyDerived?: boolean;
}

const METRICS: Metric[] = [
  {
    label: 'Download',
    of: (r) => r.download,
    format: (v) => bandwidth(v),
    higherIsBetter: true,
  },
  {
    label: 'Upload',
    of: (r) => r.upload,
    format: (v) => bandwidth(v),
    higherIsBetter: true,
  },
  {
    label: 'Latency, idle',
    of: (r) => r.latency,
    format: (v) => formatLatency(v),
    higherIsBetter: false,
    latencyDerived: true,
  },
  {
    label: 'Latency, loaded ↓',
    of: (r) => r.downLoadedLatency,
    format: (v) => formatLatency(v),
    higherIsBetter: false,
    latencyDerived: true,
  },
  {
    label: 'Latency, loaded ↑',
    of: (r) => r.upLoadedLatency,
    format: (v) => formatLatency(v),
    higherIsBetter: false,
    latencyDerived: true,
  },
  {
    label: 'Jitter',
    of: (r) => r.jitter,
    format: (v) => formatLatency(v),
    higherIsBetter: false,
    latencyDerived: true,
  },
  {
    label: 'Packet loss',
    of: (r) => r.packetLoss,
    format: (v) => formatPacketLoss(v ?? undefined),
    higherIsBetter: false,
  },
];

function bandwidth(v: number | null): string {
  const f = formatBandwidth(v ?? undefined);
  return f.unit ? `${f.value} ${f.unit}` : f.value;
}

/** How a run wants to be referred to: its description, else who and when. */
function runLabel(run: StoredRunDetail): string {
  if (run.note) return run.note;
  const who = run.clientName ?? run.hostname ?? run.clientIp;
  return `${who} · ${formatWhen(run.recordedAt)}`;
}

function formatWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

function renderHead(host: HTMLElement, side: string, run: StoredRunDetail): void {
  const who = run.clientName ?? run.hostname ?? run.clientIp;
  host.innerHTML = `
    <div class="cmp__side">${esc(side)}</div>
    <div class="cmp__label">${esc(runLabel(run))}</div>
    <div class="cmp__meta">${esc(who)} · ${esc(run.profile)} · ${esc(formatWhen(run.recordedAt))}</div>
    <a class="row-link" href="/result.html?id=${run.id}">Open run ${run.id}</a>`;
}

function render(a: StoredRunDetail, b: StoredRunDetail): void {
  renderHead(ui.headA, 'A', a);
  renderHead(ui.headB, 'B', b);

  // If either run predates 1.3.1, its latency carries up to 40 ms of our own
  // Nagle stall — so those rows are measuring our history, not the network.
  // Marked per row as well as explained in the footer, because the row is
  // where the wrong conclusion actually gets drawn.
  const latencySuspect =
    latencyPredatesNagleFix(a.appVersion) || latencyPredatesNagleFix(b.appVersion);

  ui.rows.innerHTML = METRICS.map((m) => {
    const va = m.of(a);
    const vb = m.of(b);
    const pct = changePct(va, vb);

    const verdict = verdictFor(pct, m.higherIsBetter);

    const caveat = latencySuspect && m.latencyDerived === true;

    return `<tr data-caveat="${caveat}">
      <th scope="row">${esc(m.label)}${
        caveat ? '<span class="suspect__mark" title="Not comparable — see below">*</span>' : ''
      }</th>
      <td class="cell--num">${esc(m.format(va))}</td>
      <td class="cell--num">${esc(m.format(vb))}</td>
      <td class="cell--num cmp__delta" data-verdict="${verdict}">${esc(formatChange(pct))}</td>
    </tr>`;
  }).join('');

  const sa = samplesFromStored(a.points, a.packetLoss);
  const sb = samplesFromStored(b.points, b.packetLoss);
  drawTrace('download', ui.downA, sa.download, a.download ?? undefined);
  drawTrace('download', ui.downB, sb.download, b.download ?? undefined);
  drawTrace('upload', ui.upA, sa.upload, a.upload ?? undefined);
  drawTrace('upload', ui.upB, sb.upload, b.upload ?? undefined);

  renderCaveat(a, b);
  document.body.dataset.compareState = 'loaded';
}

/**
 * Says when the latency rows cannot be compared.
 *
 * If one run predates 1.3.1 and the other does not, every latency difference
 * on this page is dominated by our own `TCP_NODELAY` fix rather than by
 * anything about the network. Silence here would turn this page into a
 * machine for drawing exactly the wrong conclusion.
 */
function renderCaveat(a: StoredRunDetail, b: StoredRunDetail): void {
  const oldA = latencyPredatesNagleFix(a.appVersion);
  const oldB = latencyPredatesNagleFix(b.appVersion);

  if (!oldA && !oldB) {
    ui.caveat.hidden = true;
    return;
  }

  ui.caveat.hidden = false;
  ui.caveat.dataset.suspect = 'true';
  ui.caveat.textContent =
    oldA !== oldB
      ? 'These two runs were measured either side of the TCP_NODELAY fix in 1.3.1, ' +
        'which removed up to 40 ms from every latency reading. The latency rows above ' +
        'mostly show that fix, not a change in the network. Bandwidth is comparable.'
      : 'Both runs predate the TCP_NODELAY fix in 1.3.1, so their latency figures are ' +
        'each inflated by up to 40 ms. They are comparable with each other, but not ' +
        'with anything measured since. Bandwidth is unaffected.';
}

function showError(message: string): void {
  ui.error.textContent = message;
  ui.error.hidden = false;
  document.body.dataset.compareState = 'error';
}

async function applySiteName(): Promise<void> {
  try {
    const name = (await fetchStatus()).siteName.trim();
    if (!name) return;
    ui.title.textContent = `${name} — Compare`;
    document.title = `Compare — ${name}`;
  } catch {
    /* keep the shipped heading */
  }
}

/** Redraw the pixel-sized traces when the window changes. */
let resizeTimer: ReturnType<typeof setTimeout> | undefined;
let loaded: [StoredRunDetail, StoredRunDetail] | undefined;
window.addEventListener('resize', () => {
  if (resizeTimer !== undefined) clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    if (loaded) render(loaded[0], loaded[1]);
  }, 120);
});

void (async () => {
  void applySiteName();

  const params = new URLSearchParams(window.location.search);
  const ids = ['a', 'b'].map((k) => Number(params.get(k)));
  if (ids.some((n) => !Number.isInteger(n) || n <= 0)) {
    showError('Pick two runs to compare from the history.');
    return;
  }
  const [idA, idB] = ids as [number, number];
  if (idA === idB) {
    showError('Those are the same run. Pick two different ones.');
    return;
  }

  try {
    // Fetched together: one round trip each way rather than one after the
    // other, and a failure on either is the same failure to the reader.
    const [a, b] = await Promise.all([fetchResult(idA), fetchResult(idB)]);
    loaded = [a, b];
    render(a, b);
  } catch (e) {
    showError(e instanceof Error ? e.message : String(e));
  }
})();

// Light or dark, remembered, on every page.
setUpThemeToggle();
