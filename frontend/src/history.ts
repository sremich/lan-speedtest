/**
 * The history page: stored runs, and a trend chart.
 *
 * Charts are hand-drawn SVG rather than a charting library. The whole page is
 * one list and one line chart, and a library would be more bundle than the job
 * deserves — the front end's only dependency is the measurement engine itself.
 */

import './styles.css';
import { fetchStatus, setClientName } from './api';
import { formatBandwidth, formatLatency, formatPacketLoss } from './format';

interface StoredRun {
  id: number;
  recordedAt: string;
  clientIp: string;
  clientName: string | null;
  /** What reverse DNS found for the address, if anything. */
  hostname: string | null;
  userAgent: string;
  profile: string;
  download: number | null;
  upload: number | null;
  latency: number | null;
  jitter: number | null;
  downLoadedLatency: number | null;
  upLoadedLatency: number | null;
  packetLoss: number | null;
  totalDurationMs: number | null;
  scores: Record<string, string>;
}

interface ClientSummary {
  clientIp: string;
  clientName: string | null;
  hostname: string | null;
  runs: number;
  lastSeen: string;
}

const el = <T extends HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

/** The most recent load, kept so a resize can redraw without refetching. */
let lastRuns: StoredRun[] = [];

/** The clients the filter knows about, so renaming can find the selected one. */
let knownClients: ClientSummary[] = [];

const ui = {
  title: el('page-title'),
  chart: el('chart'),
  rows: el('rows'),
  clientFilter: el<HTMLSelectElement>('client-filter'),
  rename: el<HTMLButtonElement>('rename'),
  empty: el('empty'),
  error: el('error'),
  count: el('count'),
};

/** Escapes text destined for innerHTML. User agents are attacker-influenced. */
function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * What to call a client.
 *
 * A name you typed wins, then whatever reverse DNS found, then the address —
 * which is always shown somewhere as well, because a friendly label that
 * replaced the address outright would make history impossible to correlate
 * with anything else on the network.
 */
function clientLabel(run: {
  clientName: string | null;
  hostname?: string | null;
  clientIp: string;
}): string {
  return run.clientName ?? run.hostname ?? run.clientIp;
}

/** Short, unambiguous local time. */
function formatWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/**
 * A line chart of download and upload over time.
 *
 * Drawn oldest-left so it reads as a timeline. Points are positioned by index
 * rather than by timestamp: runs are irregular and sparse, and an index axis
 * keeps every point visible instead of bunching a week of tests into one pixel.
 */
/**
 * The drawing width, in CSS pixels.
 *
 * `clientWidth` is zero for a detached or hidden element, so fall back rather
 * than emitting a chart of width zero.
 */
function chartWidth(): number {
  const style = getComputedStyle(ui.chart);
  const inner =
    ui.chart.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  return Math.max(360, Math.round(Number.isFinite(inner) && inner > 0 ? inner : 900));
}

function renderChart(runs: StoredRun[]): void {
  const series = [...runs].reverse();
  const usable = series.filter((r) => r.download !== null || r.upload !== null);

  if (usable.length < 2) {
    ui.chart.innerHTML =
      '<p class="note">A trend needs at least two runs. Run the test again to start one.</p>';
    return;
  }

  // Drawn at the container's real pixel width rather than at a fixed viewBox
  // scaled to fit, which would scale the tick labels along with it — tiny in a
  // narrow window, oversized on a wide monitor. `chartWidth` is remeasured on
  // resize; see the listener at the foot of this file.
  const W = chartWidth();
  const H = Math.round(Math.min(320, Math.max(200, W * 0.3)));
  // The left gutter has to fit the widest tick label. At 56 it silently
  // clipped "250.0 Mbps" down to "50.0 Mbps", which is not a cosmetic bug —
  // the chart read as an order of magnitude slower than it was.
  const PAD = { top: 16, right: 16, bottom: 28, left: 78 };
  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;

  const values = usable.flatMap((r) => [r.download, r.upload].filter((v): v is number => v !== null));
  const max = Math.max(...values);
  // Round the axis up to something readable rather than to the exact maximum.
  const ceiling = niceCeiling(max);

  const x = (i: number) => PAD.left + (usable.length === 1 ? plotW / 2 : (i / (usable.length - 1)) * plotW);
  const y = (v: number) => PAD.top + plotH - (v / ceiling) * plotH;

  const path = (key: 'download' | 'upload') => {
    const pts = usable
      .map((r, i) => ({ i, v: r[key] }))
      .filter((p): p is { i: number; v: number } => p.v !== null);
    if (pts.length === 0) return '';
    return pts.map((p, n) => `${n === 0 ? 'M' : 'L'}${x(p.i).toFixed(1)},${y(p.v).toFixed(1)}`).join(' ');
  };

  const gridLines = [0, 0.25, 0.5, 0.75, 1]
    .map((f) => {
      const gy = PAD.top + plotH - f * plotH;
      return `<line x1="${PAD.left}" y1="${gy.toFixed(1)}" x2="${W - PAD.right}" y2="${gy.toFixed(1)}" class="chart__grid" />
              <text x="${PAD.left - 10}" y="${(gy + 4).toFixed(1)}" class="chart__tick" text-anchor="end">${
                axisTick(ceiling * f)
              }</text>`;
    })
    .join('');

  const dots = (key: 'download' | 'upload', cls: string) =>
    usable
      .map((r, i) => {
        const v = r[key];
        if (v === null) return '';
        const title = `${formatWhen(r.recordedAt)} — ${clientLabel(r)}\n${key}: ${
          formatBandwidth(v).value
        } ${formatBandwidth(v).unit}`;
        return `<circle cx="${x(i).toFixed(1)}" cy="${y(v).toFixed(1)}" r="3" class="${cls}"><title>${esc(
          title,
        )}</title></circle>`;
      })
      .join('');

  ui.chart.innerHTML = `
    <svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" class="chart__svg" role="img"
         aria-label="Download and upload bandwidth over the most recent runs">
      ${gridLines}
      <path d="${path('download')}" class="chart__line chart__line--down" />
      <path d="${path('upload')}" class="chart__line chart__line--up" />
      ${dots('download', 'chart__dot chart__dot--down')}
      ${dots('upload', 'chart__dot chart__dot--up')}
      <text x="${PAD.left}" y="${H - 8}" class="chart__tick">oldest</text>
      <text x="${W - PAD.right}" y="${H - 8}" class="chart__tick" text-anchor="end">newest</text>
    </svg>
    <div class="chart__legend">
      <span class="chart__key chart__key--down">Download</span>
      <span class="chart__key chart__key--up">Upload</span>
    </div>`;
}

/**
 * A short axis label.
 *
 * Deliberately more compact than the headline formatter: axis ticks are round
 * numbers by construction, so the decimal place is noise, and a long label
 * either widens the gutter or gets clipped.
 */
export function axisTick(bps: number): string {
  if (bps <= 0) return '0';
  if (bps >= 1e9) {
    const g = bps / 1e9;
    return `${Number.isInteger(g) ? g : g.toFixed(1)} Gbps`;
  }
  if (bps >= 1e6) return `${Math.round(bps / 1e6)} Mbps`;
  return `${Math.round(bps / 1e3)} kbps`;
}

/** Rounds an axis maximum up to 1, 2 or 5 times a power of ten. */
export function niceCeiling(max: number): number {
  if (max <= 0) return 1;
  const pow = 10 ** Math.floor(Math.log10(max));
  const scaled = max / pow;
  const step = scaled <= 1 ? 1 : scaled <= 2 ? 2 : scaled <= 5 ? 5 : 10;
  return step * pow;
}

function renderRows(runs: StoredRun[]): void {
  if (runs.length === 0) {
    ui.rows.innerHTML = '';
    ui.empty.hidden = false;
    return;
  }
  ui.empty.hidden = true;

  ui.rows.innerHTML = runs
    .map((r) => {
      const down = formatBandwidth(r.download ?? undefined);
      const up = formatBandwidth(r.upload ?? undefined);
      const ratings = Object.entries(r.scores)
        .map(([k, v]) => `<span class="pill rating--${esc(v)}" title="${esc(k)}">${esc(v)}</span>`)
        .join(' ');
      // The address goes in the tooltip alongside the user agent, so a named
      // client is still identifiable without widening the column.
      const who = clientLabel(r);
      const detail = who === r.clientIp ? r.userAgent : `${r.clientIp}
${r.userAgent}`;
      return `<tr>
        <td>${esc(formatWhen(r.recordedAt))}</td>
        <td class="cell--client" title="${esc(detail)}">${esc(who)}</td>
        <td class="cell--num">${down.value}<span class="cell--unit"> ${down.unit}</span></td>
        <td class="cell--num">${up.value}<span class="cell--unit"> ${up.unit}</span></td>
        <td class="cell--num">${esc(formatLatency(r.latency))}</td>
        <td class="cell--num">${esc(formatLatency(r.downLoadedLatency))}</td>
        <td class="cell--num">${esc(formatPacketLoss(r.packetLoss ?? undefined))}</td>
        <td class="cell--ratings">${ratings}</td>
        <td class="cell--profile">${esc(r.profile)}</td>
        <td class="cell--link">
          <a class="row-link" href="/result.html?id=${r.id}" data-testid="open-result">Open</a>
        </td>
      </tr>`;
    })
    .join('');
}

async function load(): Promise<void> {
  ui.error.hidden = true;

  const filter = ui.clientFilter.value || 'all';
  const params = new URLSearchParams({ limit: '200' });
  if (filter !== 'all') params.set('client', filter);

  const res = await fetch(`/api/history?${params}`, { cache: 'no-store' });
  if (!res.ok) throw new Error(`/api/history responded ${res.status}`);
  const runs = (await res.json()) as StoredRun[];

  ui.count.textContent = runs.length === 1 ? '1 run' : `${runs.length} runs`;
  lastRuns = runs;
  renderChart(runs);
  renderRows(runs);
  document.body.dataset.historyState = 'loaded';
}

async function loadClients(): Promise<void> {
  const res = await fetch('/api/clients', { cache: 'no-store' });
  if (!res.ok) return;
  const clients = (await res.json()) as ClientSummary[];

  knownClients = clients;
  const options = ['<option value="all">All clients</option>']
    .concat(
      clients.map(
        (c) =>
          `<option value="${esc(c.clientIp)}">${esc(clientLabel(c))} (${c.runs})</option>`,
      ),
    )
    .join('');
  ui.clientFilter.innerHTML = options;
  updateRenameControl();
}

/**
 * Renaming applies to whichever client is being filtered on.
 *
 * Deliberately not a per-row control: the name belongs to the client, not to
 * one of its runs, and a rename button on every row would suggest otherwise.
 */
function updateRenameControl(): void {
  const ip = ui.clientFilter.value;
  const selected = knownClients.find((c) => c.clientIp === ip);
  ui.rename.hidden = selected === undefined;
  if (selected) {
    ui.rename.textContent = selected.clientName ? 'Rename client' : 'Name this client';
  }
}

ui.rename.addEventListener('click', () => {
  const ip = ui.clientFilter.value;
  const selected = knownClients.find((c) => c.clientIp === ip);
  if (!selected) return;

  const suggested = selected.clientName ?? selected.hostname ?? '';
  const entered = window.prompt(
    `Name for ${ip}. Leave it empty to go back to the address.`,
    suggested,
  );
  // Cancel is null, and means leave it alone; an empty string is a decision.
  if (entered === null) return;

  void (async () => {
    if (!(await setClientName(ip, entered))) {
      showError('Could not save that name.');
      return;
    }
    await loadClients();
    ui.clientFilter.value = ip;
    updateRenameControl();
    await load();
  })().catch(showError);
});

ui.clientFilter.addEventListener('change', () => {
  updateRenameControl();
  void load().catch(showError);
});

/**
 * Redraw on resize.
 *
 * The chart is sized in real pixels, so it has to be re-rendered when the
 * window changes rather than being stretched. Debounced, because a drag
 * generates a resize event per frame.
 */
let resizeTimer: ReturnType<typeof setTimeout> | undefined;
window.addEventListener('resize', () => {
  if (resizeTimer !== undefined) clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    if (lastRuns.length > 0) renderChart(lastRuns);
  }, 120);
});

function showError(e: unknown): void {
  ui.error.textContent = e instanceof Error ? e.message : String(e);
  ui.error.hidden = false;
  document.body.dataset.historyState = 'error';
}

/**
 * Names the page after the deployment.
 *
 * Best-effort and deliberately not awaited with the rest: a failure here is
 * cosmetic, and should not stop the history itself from loading.
 */
async function applySiteName(): Promise<void> {
  try {
    const name = (await fetchStatus()).siteName.trim();
    if (!name) return;
    ui.title.textContent = `${name} — History`;
    document.title = `History — ${name}`;
  } catch {
    /* keep the shipped heading */
  }
}

void (async () => {
  void applySiteName();
  try {
    await loadClients();
    await load();
  } catch (e) {
    showError(e);
  }
})();
