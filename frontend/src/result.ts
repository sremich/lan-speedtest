/**
 * The permalink: one stored run, drawn the way it was drawn when it ran.
 *
 * The history page could previously only start a *new* test, so a result you
 * navigated away from was gone. Every sample is stored with the run, and the
 * traces and distributions here come from `runview.ts` — the same code the
 * live page uses — so this is the run itself rather than a report about it.
 */

import './styles.css';
import { fetchResult, fetchStatus, setRunNote, type StoredRunDetail } from './api';
import { formatBandwidth, formatDuration, formatLatency, formatPacketLoss } from './format';
import {
  attachTraceHover,
  drawTrace,
  renderDetail,
  type RunSamples,
  type Sample,
} from './runview';

/** Human labels for the engine's AIM experience keys. */
const AIM_LABELS: Record<string, string> = {
  streaming: 'Streaming',
  gaming: 'Gaming',
  rtc: 'Video calls',
};

const el = <T extends HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

const ui = {
  title: el('page-title'),
  error: el('error'),
  note: el<HTMLButtonElement>('note'),
  download: el('download'),
  downloadUnit: el('download-unit'),
  upload: el('upload'),
  uploadUnit: el('upload-unit'),
  latency: el('latency'),
  jitter: el('jitter'),
  packetLoss: el('packet-loss'),
  downLoaded: el('down-loaded'),
  upLoaded: el('up-loaded'),
  downloadChart: el('download-chart'),
  uploadChart: el('upload-chart'),
  aim: el('aim'),
  detailBody: el('detail-body'),
  client: el('client'),
  measuredAt: el('measured-at'),
  profile: el('profile'),
  agent: el('agent'),
};

/** Escapes text destined for innerHTML. User agents are attacker-influenced. */
function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function setText(node: HTMLElement, value: string): void {
  node.textContent = value;
  node.dataset.pending = value === '—' ? 'true' : 'false';
}

/** The most recent run, so a resize can redraw without refetching. */
let lastSamples: RunSamples | undefined;

/** The run on screen, so the description can be edited against it. */
let current: StoredRunDetail | undefined;

/**
 * Reads the stored sample blob.
 *
 * Written by whatever version of the front end recorded the run, so nothing
 * here assumes a field is present: a run stored before 1.3.0 has no samples at
 * all, and should still render its headline rather than an error.
 */
function samplesOf(run: StoredRunDetail): RunSamples {
  const raw = (run.points ?? {}) as Record<string, unknown>;

  const points = (key: string): Sample[] => {
    const value = raw[key];
    if (!Array.isArray(value)) return [];
    return value.filter(
      (p): p is Sample =>
        typeof p === 'object' && p !== null && typeof (p as Sample).bps === 'number',
    );
  };

  const numbers = (key: string): number[] => {
    const value = raw[key];
    return Array.isArray(value) ? value.filter((n): n is number => typeof n === 'number') : [];
  };

  return {
    download: points('download'),
    upload: points('upload'),
    idleLatency: numbers('idleLatency'),
    downLoadedLatency: numbers('downLoadedLatency'),
    upLoadedLatency: numbers('upLoadedLatency'),
    ...(run.packetLoss !== null ? { packetLoss: run.packetLoss } : {}),
  };
}

/** Short, unambiguous local time. */
function formatWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });
}

/** Name, else resolved hostname, else the address. */
function clientLabel(run: StoredRunDetail): string {
  return run.clientName ?? run.hostname ?? run.clientIp;
}

function render(run: StoredRunDetail): void {
  current = run;
  const optional = (v: number | null): number | undefined => (v === null ? undefined : v);

  const down = formatBandwidth(optional(run.download));
  setText(ui.download, down.value);
  ui.downloadUnit.textContent = down.unit;

  const up = formatBandwidth(optional(run.upload));
  setText(ui.upload, up.value);
  ui.uploadUnit.textContent = up.unit;

  setText(ui.latency, formatLatency(run.latency));
  setText(ui.jitter, formatLatency(run.jitter));
  setText(ui.downLoaded, formatLatency(run.downLoadedLatency));
  setText(ui.upLoaded, formatLatency(run.upLoadedLatency));
  setText(ui.packetLoss, formatPacketLoss(optional(run.packetLoss)));

  const samples = samplesOf(run);
  lastSamples = samples;
  drawTrace('download', ui.downloadChart, samples.download, optional(run.download));
  drawTrace('upload', ui.uploadChart, samples.upload, optional(run.upload));
  renderDetail(ui.detailBody, samples);

  const cards = Object.entries(run.scores).map(
    ([key, rating]) => `<article class="aim__card rating--${esc(rating)}">
        <div class="aim__use">${esc(AIM_LABELS[key] ?? key)}</div>
        <div class="aim__rating">${esc(rating)}</div>
      </article>`,
  );
  ui.aim.innerHTML =
    cards.length > 0
      ? cards.join('')
      : '<p class="note">This run collected no loaded-latency samples, so the engine could not score it.</p>';

  // The label is a label: the address stays visible, because a friendly name
  // that quietly replaced it would make the history impossible to correlate
  // with anything else on the network.
  const label = clientLabel(run);
  ui.client.textContent = label === run.clientIp ? `client: ${label}` : `${label} · ${run.clientIp}`;
  ui.measuredAt.textContent =
    `Measured ${formatWhen(run.recordedAt)}` +
    (run.totalDurationMs !== null ? ` · took ${formatDuration(run.totalDurationMs)}` : '');
  ui.profile.textContent = `profile: ${run.profile}`;
  ui.agent.textContent = run.userAgent;

  renderNote(run);
  document.body.dataset.resultState = 'loaded';
}

/** The run's description, or an invitation to write one. */
function renderNote(run: StoredRunDetail): void {
  ui.note.dataset.id = String(run.id);
  ui.note.innerHTML = run.note
    ? esc(run.note)
    : '<span class="desc--empty">Add a description…</span>';
}

ui.note.addEventListener('click', () => {
  const id = Number(ui.note.dataset.id);
  if (!Number.isInteger(id)) return;

  const entered = window.prompt(
    'Description for this run — where you were, on what, what you were testing.',
    current?.note ?? '',
  );
  // Cancel is null and means leave it alone; an empty string clears it.
  if (entered === null) return;

  void (async () => {
    if (!(await setRunNote(id, entered))) {
      showError('Could not save that description.');
      return;
    }
    current = await fetchResult(id);
    renderNote(current);
  })().catch((e: unknown) => showError(e instanceof Error ? e.message : String(e)));
});

function showError(message: string): void {
  ui.error.textContent = message;
  ui.error.hidden = false;
  document.body.dataset.resultState = 'error';
}

/**
 * Names the page after the deployment.
 *
 * Best-effort and not awaited with the rest: a failure here is cosmetic and
 * should not stop the result itself from loading.
 */
async function applySiteName(): Promise<void> {
  try {
    const name = (await fetchStatus()).siteName.trim();
    if (!name) return;
    ui.title.textContent = `${name} — Result`;
    document.title = `Result — ${name}`;
  } catch {
    /* keep the shipped heading */
  }
}

/**
 * Redraw the pixel-sized drawings when the window changes.
 *
 * Debounced: a window drag fires a resize event per frame.
 */
let resizeTimer: ReturnType<typeof setTimeout> | undefined;
window.addEventListener('resize', () => {
  if (resizeTimer !== undefined) clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    if (lastSamples) renderDetail(ui.detailBody, lastSamples);
  }, 120);
});

attachTraceHover('download', ui.downloadChart);
attachTraceHover('upload', ui.uploadChart);

void (async () => {
  void applySiteName();

  const raw = new URLSearchParams(window.location.search).get('id');
  const id = Number(raw);
  if (!raw || !Number.isInteger(id) || id <= 0) {
    showError('No result id in the link.');
    return;
  }

  try {
    render(await fetchResult(id));
  } catch (e) {
    showError(e instanceof Error ? e.message : String(e));
  }
})();
