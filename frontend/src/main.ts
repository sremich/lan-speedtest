/**
 * Drives @cloudflare/speedtest against our own backend and renders the run.
 *
 * The engine does all the measuring; this file is configuration, live
 * rendering and the AIM ratings read out of the engine's own scoring.
 */

import SpeedTest from '@cloudflare/speedtest';
import type { MeasurementSummary, Results, Scores } from '@cloudflare/speedtest';

import './styles.css';
import { assertLanOnly, fetchProfile, fetchStatus, submitResult, type EngineConfig } from './api';
import {
  formatBandwidth,
  formatDuration,
  formatLatency,
  formatPacketLoss,
  latencyIsAtBrowserResolution,
} from './format';
import { BOXPLOT_LEGEND, renderBoxPlots } from './boxplot';
import { measureParallelThroughput, suggestedProfile } from './parallel';
import { bandwidthBySize, formatTransferSize, summarise, type Distribution } from './stats';

type Engine = InstanceType<typeof SpeedTest>;

/** The engine's most recent download figure, used to size the raw harness. */
let lastDownloadBps: number | undefined;

const el = <T extends HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

const ui = {
  phase: el('phase'),
  restart: el<HTMLButtonElement>('restart'),
  progress: el('progress'),
  error: el('error'),
  download: el('download'),
  downloadUnit: el('download-unit'),
  upload: el('upload'),
  uploadUnit: el('upload-unit'),
  latency: el('latency'),
  jitter: el('jitter'),
  downLoaded: el('down-loaded'),
  upLoaded: el('up-loaded'),
  packetLoss: el('packet-loss'),
  duration: el('duration'),
  aim: el('aim'),
  precisionNote: el('precision-note'),
  profile: el('profile'),
  build: el('build'),
  historyLink: el<HTMLAnchorElement>('history-link'),
  detailBody: el('detail-body'),
  rawRun: el<HTMLButtonElement>('raw-run'),
  rawStatus: el('raw-status'),
  rawResult: el('raw-result'),
};

/** Human labels for the engine's AIM experience keys. */
const AIM_LABELS: Record<string, string> = {
  streaming: 'Streaming',
  gaming: 'Gaming',
  rtc: 'Video calls',
};

const PHASE_LABELS: Record<string, string> = {
  latency: 'Measuring latency',
  latencyUnderLoad: 'Measuring latency under load',
  download: 'Measuring download',
  upload: 'Measuring upload',
  packetLoss: 'Measuring packet loss',
  packetLossUnderLoad: 'Measuring packet loss under load',
};

function setText(node: HTMLElement, value: string): void {
  node.textContent = value;
  node.dataset.pending = value === '—' ? 'true' : 'false';
}

function showError(message: string): void {
  ui.error.textContent = message;
  ui.error.hidden = false;
}

function clearError(): void {
  ui.error.hidden = true;
  ui.error.textContent = '';
}

function render(results: Results, final = false): void {
  const summary = results.getSummary();

  lastDownloadBps = summary.download;

  const down = formatBandwidth(summary.download);
  setText(ui.download, down.value);
  ui.downloadUnit.textContent = down.unit;

  const up = formatBandwidth(summary.upload);
  setText(ui.upload, up.value);
  ui.uploadUnit.textContent = up.unit;

  setText(ui.latency, formatLatency(summary.latency));
  setText(ui.jitter, formatLatency(summary.jitter));
  setText(ui.downLoaded, formatLatency(summary.downLoadedLatency));
  setText(ui.upLoaded, formatLatency(summary.upLoadedLatency));
  setText(ui.packetLoss, formatPacketLoss(summary.packetLoss));
  setText(ui.duration, formatDuration(summary.totalDurationMs));

  renderScores(results, final);
  maybeNotePrecision(summary);
  renderDetail(results);
}

/** Formats bits per second for an axis or tooltip. */
function bps(value: number): string {
  const f = formatBandwidth(value);
  return `${f.value} ${f.unit}`;
}

function ms(value: number): string {
  return `${value.toFixed(2)} ms`;
}

/**
 * The per-measurement distributions.
 *
 * The headline figures are single percentiles, which say nothing about how
 * consistent a run was — and consistency is exactly what exposes a failing
 * cable or a duplex mismatch. These show every sample the engine collected.
 */
function renderDetail(results: Results): void {
  const groups: Array<{ title: string; rows: Distribution[]; format: (v: number) => string }> = [];

  const download = bandwidthBySize(results.getDownloadBandwidthPoints(), formatTransferSize);
  if (download.length > 0) {
    groups.push({ title: 'Download, by transfer size', rows: download, format: bps });
  }

  const upload = bandwidthBySize(results.getUploadBandwidthPoints(), formatTransferSize);
  if (upload.length > 0) {
    groups.push({ title: 'Upload, by transfer size', rows: upload, format: bps });
  }

  const latencyRows: Distribution[] = [];
  const latencySeries: Array<[string, number[]]> = [
    ['Idle', results.getUnloadedLatencyPoints()],
    ['Loaded ↓', results.getDownLoadedLatencyPoints()],
    ['Loaded ↑', results.getUpLoadedLatencyPoints()],
  ];
  for (const [label, points] of latencySeries) {
    const summary = summarise(points);
    if (summary) {
      latencyRows.push({
        label,
        detail: `${summary.count} ping${summary.count === 1 ? '' : 's'}`,
        summary,
      });
    }
  }
  if (latencyRows.length > 0) {
    // Not zero-based: on a LAN every value sits near zero, and forcing the
    // axis to the origin would flatten the whole distribution into a smear.
    groups.push({ title: 'Latency', rows: latencyRows, format: ms });
  }

  if (groups.length === 0) {
    ui.detailBody.innerHTML = '<p class="note">No samples collected yet.</p>';
    return;
  }

  ui.detailBody.innerHTML =
    groups
      .map(
        (g) => `<div class="box__group">
          <h3 class="box__group-title">${g.title}</h3>
          ${renderBoxPlots(g.rows, {
            format: g.format,
            zeroBased: g.format === bps,
          })}
        </div>`,
      )
      .join('') + BOXPLOT_LEGEND;
}

function renderScores(results: Results, final: boolean): void {
  let scores: Scores;
  try {
    scores = results.getScores();
  } catch {
    // Scores need a complete-enough summary; before that they simply are not
    // available yet, which is not an error worth surfacing.
    return;
  }

  const cards = Object.entries(scores)
    .filter(([, score]) => score && typeof score.classificationName === 'string')
    .map(([key, score]) => {
      const label = AIM_LABELS[key] ?? key;
      const rating = score.classificationName;
      return `<article class="aim__card rating--${rating}">
          <div class="aim__use">${label}</div>
          <div class="aim__rating">${rating}</div>
        </article>`;
    });

  if (cards.length > 0) {
    ui.aim.innerHTML = cards.join('');
    return;
  }

  // Every AIM experience depends on `loadedLatencyIncrease`, which the engine
  // computes only when loaded latency is a truthy number. A run whose
  // transfers all finished faster than the profile's
  // `loadedRequestMinDuration` collects no loaded-latency samples at all, so
  // the value stays 0 and the engine returns no scores whatsoever. Say that
  // plainly instead of leaving an empty panel.
  if (final) {
    ui.aim.innerHTML =
      '<p class="note">No rating available: this run collected no ' +
      'loaded-latency samples, so the engine could not score it. The ' +
      'transfers finished faster than the profile’s ' +
      '<code>loaded_request_min_duration</code> — lower that value, or use ' +
      'a profile with larger transfers.</p>';
  }
}

function maybeNotePrecision(summary: MeasurementSummary): void {
  const quantised = latencyIsAtBrowserResolution(
    summary.latency,
    summary.jitter,
    summary.downLoadedLatency,
    summary.upLoadedLatency,
  );
  if (!quantised) {
    ui.precisionNote.hidden = true;
    return;
  }
  ui.precisionNote.textContent =
    'Latency figures are at the limit of the browser’s timing resolution ' +
    '(coarsened to ~0.1 ms in Chrome, ~1 ms in Firefox). On a LAN this means ' +
    'the true round trip is below what can be measured here — read the ' +
    'loaded-vs-idle difference rather than the absolute numbers.';
  ui.precisionNote.hidden = false;
}

async function run(): Promise<void> {
  clearError();
  ui.restart.disabled = true;
  ui.progress.style.width = '0%';
  setText(ui.phase, 'Starting…');

  const [status, profile] = await Promise.all([fetchStatus(), fetchProfile()]);

  ui.profile.textContent = `profile: ${profile.profile}${
    profile.description ? ` (${profile.description})` : ''
  }`;
  ui.build.textContent = `v${status.version} · ${status.gitSha}`;
  ui.historyLink.hidden = !status.historyEnabled;

  const config: EngineConfig = profile.engineConfig;
  // Fails loudly rather than letting a bad deploy phone home. See api.ts.
  assertLanOnly(config);

  const totalStages = config.measurements.length;
  let completedStages = 0;

  const engine: Engine = new SpeedTest({
    ...config,
    // Pinned here as well as server-side: this is the setting that decides
    // whether a completed run is posted to Cloudflare.
    logAimApiUrl: null,
    logMeasurementApiUrl: null,
    autoStart: false,
  });

  engine.onPhaseChange = ({ measurement }) => {
    completedStages += 1;
    const pct = totalStages > 0 ? Math.min(100, (completedStages / totalStages) * 100) : 0;
    ui.progress.style.width = `${pct}%`;
    ui.phase.textContent = PHASE_LABELS[measurement.type] ?? measurement.type;
  };

  engine.onResultsChange = () => render(engine.results);

  engine.onError = (message: string) => {
    showError(`Test failed: ${message}`);
    ui.restart.disabled = false;
    setText(ui.phase, 'Failed');
  };

  engine.onFinish = (results) => {
    render(results, true);
    ui.progress.style.width = '100%';
    setText(ui.phase, 'Complete');
    ui.restart.disabled = false;
    document.body.dataset.testState = 'complete';

    // Best-effort: a storage failure must not present as a failed test.
    if (status.historyEnabled) {
      let scores: Record<string, string> = {};
      try {
        scores = Object.fromEntries(
          Object.entries(results.getScores()).map(([k, v]) => [k, v.classificationName]),
        );
      } catch {
        scores = {};
      }
      void submitResult({
        summary: results.getSummary() as unknown as Record<string, unknown>,
        scores,
        profile: profile.profile,
      }).then((stored) => {
        document.body.dataset.resultStored = stored ? 'yes' : 'no';
      });
    }
  };

  document.body.dataset.testState = 'running';
  engine.play();
}

/**
 * The parallel-stream harness, run on demand.
 *
 * Deliberately a separate control and a separate number: it measures something
 * the engine does not, and merging the two would be misleading.
 */
ui.rawRun.addEventListener('click', () => {
  void (async () => {
    ui.rawRun.disabled = true;
    ui.rawResult.hidden = true;
    ui.rawStatus.textContent = 'Measuring…';

    try {
      // Size the transfer from what the engine just measured, so the harness
      // takes about a second on any link.
      const observed = lastDownloadBps ?? 1e9;
      const { streams, bytesPerStream } = suggestedProfile(observed);

      const result = await measureParallelThroughput({
        streams,
        bytesPerStream,
        timeoutMs: 60_000,
      });

      const f = formatBandwidth(result.bps);
      ui.rawResult.innerHTML = `${f.value} <span>${f.unit} · ${result.streams} streams × ${
        formatTransferSize(result.bytesPerStream)
      } in ${(result.elapsedMs / 1000).toFixed(2)} s</span>`;
      ui.rawResult.hidden = false;
      ui.rawStatus.textContent = '';
      document.body.dataset.rawState = 'done';
    } catch (e) {
      ui.rawStatus.textContent = e instanceof Error ? e.message : String(e);
      document.body.dataset.rawState = 'error';
    } finally {
      ui.rawRun.disabled = false;
    }
  })();
});

ui.restart.addEventListener('click', () => {
  void run().catch((e: unknown) => {
    showError(e instanceof Error ? e.message : String(e));
    ui.restart.disabled = false;
  });
});

// Auto-start on load, as the brief asks.
void run().catch((e: unknown) => {
  showError(e instanceof Error ? e.message : String(e));
  setText(ui.phase, 'Failed');
  ui.restart.disabled = false;
  document.body.dataset.testState = 'error';
});
