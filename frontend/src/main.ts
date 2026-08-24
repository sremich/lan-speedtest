/**
 * Drives @cloudflare/speedtest against our own backend and renders the run.
 *
 * The engine does all the measuring; this file is configuration, live
 * rendering and the AIM ratings read out of the engine's own scoring.
 */

import SpeedTest from '@cloudflare/speedtest';
import type { MeasurementSummary, Results, Scores } from '@cloudflare/speedtest';

import './styles.css';
import { assertLanOnly, fetchProfile, fetchStatus, type EngineConfig } from './api';
import {
  formatBandwidth,
  formatDuration,
  formatLatency,
  formatPacketLoss,
  latencyIsAtBrowserResolution,
} from './format';

type Engine = InstanceType<typeof SpeedTest>;

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
  };

  document.body.dataset.testState = 'running';
  engine.play();
}

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
