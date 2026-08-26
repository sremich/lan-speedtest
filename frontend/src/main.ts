/**
 * Drives @cloudflare/speedtest against our own backend and renders the run.
 *
 * The engine does all the measuring; this file is configuration, live
 * rendering and the AIM ratings read out of the engine's own scoring.
 */

import SpeedTest from '@cloudflare/speedtest';
import type { MeasurementSummary, Results, Scores } from '@cloudflare/speedtest';

import './styles.css';
import { setUpThemeToggle } from './theme';
import {
  assertLanOnly,
  fetchProfile,
  fetchProfiles,
  fetchStatus,
  submitResult,
  type EngineConfig,
  type ProfileSummary,
} from './api';
import {
  formatBandwidth,
  formatDuration,
  formatLatency,
  formatPacketLoss,
  latencyIsAtBrowserResolution,
} from './format';
import { measureParallelThroughput, suggestedProfile } from './parallel';
import { formatPayload, planStages, renderChevrons, totalRequests, type Stage } from './progress';
import { formatTransferSize, summarise } from './stats';
import {
  attachTraceHover,
  drawTrace,
  placeTip,
  renderDetail,
  tipRow,
  type RunSamples,
} from './runview';

type Engine = InstanceType<typeof SpeedTest>;

/** The engine's most recent download figure, used to size the raw harness. */
let lastDownloadBps: number | undefined;

/** The running engine, so the pause control can reach it. */
let currentEngine: Engine | undefined;

/**
 * The most recent results, so a window resize can redraw the box plots.
 *
 * They are drawn at a real pixel width rather than being stretched by the
 * browser, which is what keeps their labels legible at any window size — but
 * it does mean a resize needs a redraw rather than doing nothing.
 */
let lastResults: Results | undefined;

/** The planned stages of the current run, and how far through them we are. */
let stages: Stage[] = [];
let completedRequests = 0;
/** Index into `stages` of the stage now running; -1 before the first. */
let currentStage = -1;
/** Sample counts when the current stage began, to measure progress inside it. */
let stageBaseline = { download: 0, upload: 0, latency: 0 };
/** What each finished stage measured, keyed by step number. */
const stageOutcome = new Map<number, string>();
/** The step under the pointer, so a redraw does not drop the highlight. */
let hoveredStep: number | undefined;

/** Profiles the picker can offer, and the server's own default. */
let availableProfiles: ProfileSummary[] = [];
let serverDefaultProfile = '';

/** Remembered across visits, so a chosen profile sticks to this browser. */
const PROFILE_KEY = 'speedtest.profile';
const AUTO = '__auto__';

/** Remembered per browser, overriding whatever the deployment defaults to. */
const AUTOSTART_KEY = 'speedtest.autostart';

const el = <T extends HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

const ui = {
  siteName: el('site-name'),
  phase: el('phase'),
  restart: el<HTMLButtonElement>('restart'),
  autostart: el<HTMLInputElement>('autostart'),
  idleNote: el('idle-note'),
  error: el('error'),
  download: el('download'),
  downloadUnit: el('download-unit'),
  upload: el('upload'),
  uploadUnit: el('upload-unit'),
  latency: el('latency'),
  jitter: el('jitter'),
  packetLoss: el('packet-loss'),
  aim: el('aim'),
  precisionNote: el('precision-note'),
  profile: el('profile'),
  build: el('build'),
  historyLink: el<HTMLAnchorElement>('history-link'),
  permalink: el<HTMLAnchorElement>('permalink'),
  downLoaded: el('down-loaded'),
  upLoaded: el('up-loaded'),
  downJitter: el('down-jitter'),
  upJitter: el('up-jitter'),
  downloadChart: el('download-chart'),
  uploadChart: el('upload-chart'),
  measuredAt: el('measured-at'),
  connection: el('connection'),
  pause: el<HTMLButtonElement>('pause'),
  detailBody: el('detail-body'),
  rawRun: el<HTMLButtonElement>('raw-run'),
  rawStatus: el('raw-status'),
  rawResult: el('raw-result'),
  profileSelect: el<HTMLSelectElement>('profile-select'),
  chevrons: el('chevrons'),
  stepsNow: el('steps-now'),
  phaseDetail: el('phase-detail'),
  stepTip: el('step-tip'),
};

/** Human labels for the engine's AIM experience keys. */
const AIM_LABELS: Record<string, string> = {
  streaming: 'Streaming',
  gaming: 'Gaming',
  rtc: 'Video calls',
};

/**
 * Why the address on screen is the address on screen.
 *
 * Asked repeatedly, and the honest answers differ: a private address behind a
 * subnet router cannot be recovered at all, because the translation happens at
 * layer 3 and leaves no header behind.
 */
const ADDRESS_NOTES: Record<string, string> = {
  loopback: 'The test is being served to the same machine that is running it.',
  lan:
    'A LAN address, seen directly from the connection. If a router between you and the server ' +
    'translates addresses, this is the last hop before the server rather than the client itself.',
  cgnat:
    'A shared-address range, used by carrier-grade NAT and by Tailscale. Traffic arriving over a ' +
    'subnet router is translated on the way, so the original client address is not recoverable.',
  'link-local': 'A link-local address, assigned without a DHCP server.',
  public: 'A public address, so this connection reached the server from outside the LAN.',
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

/**
 * Names the page after the deployment.
 *
 * Server-side rather than baked into the bundle, so two installations on one
 * LAN are tellable apart and a rename is a restart rather than a rebuild.
 */
function applySiteName(name: string): void {
  const trimmed = name.trim();
  if (!trimmed) return;
  ui.siteName.textContent = trimmed;
  document.title = trimmed;
}

/**
 * Empties every figure on the page.
 *
 * Used when the profile changes: the numbers on screen were measured under the
 * old one, and leaving them under the new label would misattribute them. They
 * are cleared rather than re-measured, so changing a setting never moves
 * hundreds of megabytes on its own.
 */
function clearResults(): void {
  lastResults = undefined;
  lastDownloadBps = undefined;

  for (const node of [
    ui.download,
    ui.upload,
    ui.latency,
    ui.jitter,
    ui.packetLoss,
    ui.downLoaded,
    ui.upLoaded,
    ui.downJitter,
    ui.upJitter,
  ]) {
    setText(node, '—');
  }
  ui.downloadUnit.textContent = '';
  ui.uploadUnit.textContent = '';
  ui.aim.innerHTML = '';
  ui.downloadChart.innerHTML = '';
  ui.uploadChart.innerHTML = '';
  ui.detailBody.innerHTML = '';
  ui.measuredAt.textContent = '';
  ui.permalink.hidden = true;
  ui.precisionNote.hidden = true;

  stages = [];
  stageOutcome.clear();
  completedRequests = 0;
  currentStage = -1;
  paintChevrons();
  clearError();
}

function render(results: Results, final = false): void {
  lastResults = results;
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
  setText(ui.downJitter, formatLatency(summary.downLoadedJitter));
  setText(ui.upJitter, formatLatency(summary.upLoadedJitter));
  setText(ui.packetLoss, formatPacketLoss(summary.packetLoss));

  renderTraces(results, summary);

  if (summary.totalDurationMs !== undefined) {
    ui.measuredAt.textContent = `Measured at ${new Date().toLocaleTimeString()} · took ${formatDuration(
      summary.totalDurationMs,
    )}`;
  }

  renderScores(results, final);
  maybeNotePrecision(summary);
  renderDetail(ui.detailBody, samplesOf(results));
}

/**
 * The live bandwidth traces.
 *
 * The engine reports bandwidth at the 90th percentile, so that is what the
 * marker line shows — it makes the headline number locatable within the shape
 * of the samples rather than an unexplained summary of them.
 */
function renderTraces(results: Results, summary: MeasurementSummary): void {
  drawTrace('download', ui.downloadChart, results.getDownloadBandwidthPoints(), summary.download);
  drawTrace('upload', ui.uploadChart, results.getUploadBandwidthPoints(), summary.upload);
}

/**
 * The engine's results as plain data.
 *
 * This is both what the detail panel draws from and what a completed run is
 * stored as, so the permalink renders exactly the same page from exactly the
 * same numbers rather than an approximation of them.
 */
function samplesOf(results: Results): RunSamples {
  const loss = results.getPacketLoss();
  return {
    download: results.getDownloadBandwidthPoints() as unknown as RunSamples['download'],
    upload: results.getUploadBandwidthPoints() as unknown as RunSamples['upload'],
    idleLatency: results.getUnloadedLatencyPoints(),
    downLoadedLatency: results.getDownLoadedLatencyPoints(),
    upLoadedLatency: results.getUpLoadedLatencyPoints(),
    ...(loss !== undefined && Number.isFinite(loss) ? { packetLoss: loss } : {}),
  };
}

/* --- the step strip ------------------------------------------------------ */

/** Redraws the strip, preserving the highlight if the pointer is still on it. */
function paintChevrons(): void {
  ui.chevrons.innerHTML = renderChevrons(stages, completedRequests);
  if (hoveredStep !== undefined) highlightStep(hoveredStep);
}

/** How many samples of a kind the engine has produced so far. */
function sampleCount(results: Results, type: string): number {
  switch (type) {
    case 'download':
      return results.getDownloadBandwidthPoints().length;
    case 'upload':
      return results.getUploadBandwidthPoints().length;
    case 'latency':
      return results.getUnloadedLatencyPoints().length;
    default:
      // Packet loss reports one figure at the end, so it has no partial
      // progress to read; its block fills when the stage completes.
      return 0;
  }
}

function baselineFor(type: string): number {
  if (type === 'download') return stageBaseline.download;
  if (type === 'upload') return stageBaseline.upload;
  if (type === 'latency') return stageBaseline.latency;
  return 0;
}

/**
 * Advances the strip.
 *
 * Sample counts accumulate across every stage of a kind, so progress within
 * the running stage is the growth since it started. Monotonic on purpose: a
 * count that went backwards would make the strip flicker.
 */
function updateProgress(results: Results): void {
  const stage = stages[currentStage];
  if (!stage) return;

  const grown = sampleCount(results, stage.type) - baselineFor(stage.type);
  const done = stage.offset + Math.min(stage.requests, Math.max(0, grown));
  if (done > completedRequests) {
    completedRequests = done;
    paintChevrons();
  }
}

/** Closes off the stage that just ended and opens the next one. */
function advanceStage(results: Results | undefined): void {
  const finished = stages[currentStage];
  if (finished) {
    completedRequests = Math.max(completedRequests, finished.offset + finished.requests);
    if (results) {
      const outcome = summariseStage(results, finished);
      if (outcome) stageOutcome.set(finished.step, outcome);
    }
  }

  currentStage += 1;
  if (results) {
    stageBaseline = {
      download: results.getDownloadBandwidthPoints().length,
      upload: results.getUploadBandwidthPoints().length,
      latency: results.getUnloadedLatencyPoints().length,
    };
  }
  paintChevrons();
}

/** What a finished stage measured, for its tooltip. */
function summariseStage(results: Results, stage: Stage): string | undefined {
  if (stage.type === 'download' || stage.type === 'upload') {
    const all =
      stage.type === 'download'
        ? results.getDownloadBandwidthPoints()
        : results.getUploadBandwidthPoints();
    const summary = summarise(all.slice(baselineFor(stage.type)).map((p) => p.bps));
    if (!summary) return undefined;
    const f = formatBandwidth(summary.median);
    return `${f.value} ${f.unit} median`;
  }

  if (stage.type === 'latency') {
    const summary = summarise(results.getUnloadedLatencyPoints().slice(stageBaseline.latency));
    return summary ? `${summary.median.toFixed(2)} ms median` : undefined;
  }

  if (stage.type.startsWith('packetLoss')) {
    const loss = results.getPacketLoss();
    return loss !== undefined && Number.isFinite(loss)
      ? `${formatPacketLoss(loss)} lost`
      : undefined;
  }

  return undefined;
}

function highlightStep(step: number): void {
  for (const chev of ui.chevrons.querySelectorAll<HTMLElement>('.chev')) {
    chev.classList.toggle('is-hovered', chev.dataset.step === String(step));
  }
}

function hideStepTip(): void {
  hoveredStep = undefined;
  ui.stepTip.hidden = true;
  for (const chev of ui.chevrons.querySelectorAll<HTMLElement>('.chev.is-hovered')) {
    chev.classList.remove('is-hovered');
  }
}

/**
 * Hover on the strip: what this stage is, and what it measured.
 *
 * The reference shows payload, request count and step number; we can add the
 * result too, because by the time you hover we have measured it.
 */
function showStepTip(step: number, chev: HTMLElement): void {
  const stage = stages.find((s) => s.step === step);
  if (!stage) return;

  hoveredStep = step;
  highlightStep(step);

  const rows: string[] = [];
  if (stage.bytes !== undefined && stage.bytes > 0) {
    rows.push(tipRow('Payload', formatPayload(stage.bytes)));
  }
  rows.push(tipRow('Requests', String(stage.requests)));
  rows.push(tipRow('Step', `${stage.step} of ${stages.length}`));
  const outcome = stageOutcome.get(step);
  if (outcome) {
    rows.push(tipRow('Measured', outcome));
  } else if (stage.warmUp) {
    rows.push('<div class="tip__key">Warm-up — not counted</div>');
  }

  ui.stepTip.innerHTML =
    `<div class="tip__head"><span class="tip__swatch" ` +
    `style="--swatch: ${stageColour(stage.type)}"></span>${stage.label}</div>${rows.join('')}`;
  ui.stepTip.hidden = false;

  // Positioned against the section, which is the tooltip's offset parent.
  const host = ui.chevrons.parentElement ?? ui.chevrons;
  const hostBox = host.getBoundingClientRect();
  const chevBox = chev.getBoundingClientRect();
  placeTip(
    ui.stepTip,
    host,
    chevBox.left - hostBox.left + chevBox.width / 2,
    chevBox.top - hostBox.top,
    chevBox.height,
  );
}

/**
 * The line above the strip: what is running, in that stage's colour, with the
 * payload size it is moving.
 *
 * `running` drives the spinner. It is deliberately tied to the engine's own
 * state rather than to a timer, so a paused run stops spinning instead of
 * pretending to still be working.
 */
function showNow(label: string, type: string | undefined, running: boolean): void {
  setText(ui.phase, label);
  ui.stepsNow.dataset.running = running ? 'true' : 'false';
  ui.stepsNow.style.setProperty('--stage', type ? stageColour(type) : 'var(--text-dim)');

  const stage = currentStage >= 0 ? stages[currentStage] : undefined;
  ui.phaseDetail.textContent =
    running && stage?.bytes !== undefined ? `· ${formatPayload(stage.bytes)}` : '';
}

function stageColour(type: string): string {
  switch (type) {
    case 'download':
      return 'var(--accent)';
    case 'upload':
      return 'var(--upload)';
    case 'latency':
      return '#3b82f6';
    default:
      return '#c2410c';
  }
}

ui.chevrons.addEventListener('pointermove', (event) => {
  const chev = (event.target as HTMLElement | null)?.closest<HTMLElement>('.chev');
  if (!chev) {
    hideStepTip();
    return;
  }
  const step = Number(chev.dataset.step);
  if (Number.isFinite(step)) showStepTip(step, chev);
});
ui.chevrons.addEventListener('pointerleave', hideStepTip);

/* --- the profile picker --------------------------------------------------- */

/**
 * Populates the picker.
 *
 * Best-effort: a deployment whose profile list cannot be read should still run
 * a test with whatever the server considers its default.
 */
async function setUpProfilePicker(): Promise<void> {
  try {
    const list = await fetchProfiles();
    availableProfiles = list.profiles;
    serverDefaultProfile = list.default;
  } catch {
    ui.profileSelect.hidden = true;
    return;
  }

  const canAuto = availableProfiles.some((p) => p.autoSelectable && p.nominalBps);
  const options = availableProfiles.map(
    (p) =>
      `<option value="${p.name}">${p.name}${p.description ? ` — ${p.description}` : ''}</option>`,
  );
  ui.profileSelect.innerHTML =
    (canAuto ? `<option value="${AUTO}">Auto</option>` : '') + options.join('');
  ui.profileSelect.value = storedProfileChoice();
}

/** The remembered choice, falling back to the server's default. */
function storedProfileChoice(): string {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(PROFILE_KEY);
  } catch {
    // Private browsing, or storage disabled. Not worth surfacing.
  }
  if (stored === AUTO && availableProfiles.some((p) => p.autoSelectable && p.nominalBps)) {
    return AUTO;
  }
  if (stored && availableProfiles.some((p) => p.name === stored)) return stored;
  return serverDefaultProfile;
}

/**
 * Picks a profile from a quick measurement of the link.
 *
 * The profiles differ in transfer size, and a size chosen for the wrong link
 * measures the wrong thing: 25 MB on 10 GbE finishes in 20 ms, which is mostly
 * request overhead. The rule is the largest profile the measured link can
 * justify, allowing a factor of two because a single stream reaches only part
 * of a fast link.
 */
async function pickAutoProfile(): Promise<string | undefined> {
  const candidates = availableProfiles
    .filter((p) => p.autoSelectable && p.nominalBps)
    .sort((a, b) => (a.nominalBps ?? 0) - (b.nominalBps ?? 0));
  if (candidates.length === 0) return undefined;

  showNow('Sizing the test to the link…', undefined, true);
  try {
    const probe = await measureParallelThroughput({
      streams: 1,
      bytesPerStream: 64_000_000,
      timeoutMs: 20_000,
    });
    const affordable = candidates.filter((p) => (p.nominalBps ?? 0) <= probe.bps * 2);
    return (affordable[affordable.length - 1] ?? candidates[0])!.name;
  } catch {
    // A failed probe is not a failed test; fall back to the server's default.
    return undefined;
  }
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
  ui.idleNote.hidden = true;
  ui.pause.disabled = false;
  ui.restart.disabled = true;
  ui.permalink.hidden = true;
  hideStepTip();
  stages = [];
  stageOutcome.clear();
  completedRequests = 0;
  currentStage = -1;
  paintChevrons();
  showNow('Starting…', undefined, true);

  const status = await fetchStatus();

  // Auto probes the link before it can name a profile, so it has to resolve
  // before the configuration is fetched.
  const choice = ui.profileSelect.value || serverDefaultProfile;
  const wanted = choice === AUTO ? await pickAutoProfile() : choice;
  const profile = await fetchProfile(wanted);

  applySiteName(status.siteName);
  ui.profile.textContent = `profile: ${profile.profile}${
    profile.description ? ` (${profile.description})` : ''
  }`;
  ui.build.textContent = `v${status.version} · ${status.gitSha}`;
  ui.historyLink.hidden = !status.historyEnabled;
  // Stands in for the server-location map on speed.cloudflare.com. That needs
  // external tiles, which would leave the LAN — the one thing this must not
  // do — and on a LAN the useful half is knowing which machine you are on.
  ui.connection.textContent = `client: ${status.clientIp} (${status.clientKindLabel})`;
  ui.connection.title = ADDRESS_NOTES[status.clientKind] ?? '';

  const config: EngineConfig = profile.engineConfig;
  // Fails loudly rather than letting a bad deploy phone home. See api.ts.
  assertLanOnly(config);

  // The strip is drawn from the profile before anything runs, so the shape of
  // the work is visible up front rather than only in hindsight.
  stages = planStages(config.measurements);
  document.body.dataset.profile = profile.profile;
  document.body.dataset.steps = String(totalRequests(stages));
  paintChevrons();

  const engine: Engine = new SpeedTest({
    ...config,
    // Pinned here as well as server-side: this is the setting that decides
    // whether a completed run is posted to Cloudflare.
    logAimApiUrl: null,
    logMeasurementApiUrl: null,
    autoStart: false,
  });

  engine.onPhaseChange = ({ measurement }) => {
    advanceStage(engine.results);
    showNow(PHASE_LABELS[measurement.type] ?? measurement.type, measurement.type, true);
  };

  engine.onResultsChange = () => {
    render(engine.results);
    updateProgress(engine.results);
  };

  engine.onError = (message: string) => {
    showError(`Test failed: ${message}`);
    ui.restart.disabled = false;
    showNow('Failed', undefined, false);
  };

  engine.onFinish = (results) => {
    render(results, true);
    // Close off the last stage and fill the strip: the final phase never gets
    // a phase-change event of its own to end it.
    advanceStage(results);
    completedRequests = totalRequests(stages);
    paintChevrons();
    showNow('Complete', undefined, false);
    ui.restart.disabled = false;
    ui.pause.disabled = true;
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
        // Stored so the run can be reopened and redrawn, rather than reduced
        // to the headline the moment you navigate away from it.
        points: samplesOf(results) as unknown as Record<string, unknown>,
      }).then((id) => {
        document.body.dataset.resultStored = id === undefined ? 'no' : 'yes';
        if (id !== undefined) showPermalink(id);
      });
    }
  };

  currentEngine = engine;
  ui.pause.textContent = 'Pause';
  ui.pause.disabled = false;

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

/**
 * Pause and resume.
 *
 * The engine supports this for the bandwidth and latency stages; packet loss
 * and reachability run to completion once started, so the button reflects what
 * the engine will actually honour rather than promising more.
 */
ui.pause.addEventListener('click', () => {
  const engine = currentEngine;
  if (!engine) return;
  if (engine.isRunning) {
    engine.pause();
    ui.pause.textContent = 'Resume';
    // Stop the spinner too: a paused run that still appears to be working is
    // the one thing the indicator must not do.
    ui.stepsNow.dataset.running = 'false';
  } else {
    engine.play();
    ui.pause.textContent = 'Pause';
    ui.stepsNow.dataset.running = 'true';
  }
});

/**
 * Redraw the pixel-sized drawings when the window changes.
 *
 * Debounced: a window drag fires a resize event per frame, and re-rendering
 * the detail panel that often is wasted work.
 */
let resizeTimer: ReturnType<typeof setTimeout> | undefined;
window.addEventListener('resize', () => {
  if (resizeTimer !== undefined) clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    if (lastResults) renderDetail(ui.detailBody, samplesOf(lastResults));
  }, 120);
});

ui.restart.addEventListener('click', () => {
  void start();
});

/**
 * Changing the profile clears the result and waits.
 *
 * It used to start a run immediately, to avoid the worst of both worlds:
 * yesterday's figures sitting under today's profile name, which is a lie about
 * what they are. Clearing solves that without the side effect — on `lan-10g`
 * an accidental brush of the dropdown was several gigabytes down the wire
 * before you could react to it.
 */
ui.profileSelect.addEventListener('change', () => {
  try {
    localStorage.setItem(PROFILE_KEY, ui.profileSelect.value);
  } catch {
    // Storage unavailable; the choice simply will not be remembered.
  }
  clearResults();
  showIdle('Ready — press Retest to measure with this profile.');
});

ui.autostart.addEventListener('change', () => {
  try {
    localStorage.setItem(AUTOSTART_KEY, ui.autostart.checked ? '1' : '0');
  } catch {
    // Storage unavailable; the choice lasts for this page only.
  }
});

attachTraceHover('download', ui.downloadChart);
attachTraceHover('upload', ui.uploadChart);

/**
 * Reveals the link back to this run.
 *
 * Shown only once the run is stored, because a link to a result that was never
 * written is worse than no link at all.
 */
function showPermalink(id: number): void {
  ui.permalink.href = `/result.html?id=${id}`;
  ui.permalink.hidden = false;
}

function start(): Promise<void> {
  return run().catch((e: unknown) => {
    showError(e instanceof Error ? e.message : String(e));
    showNow('Failed', undefined, false);
    ui.restart.disabled = false;
    document.body.dataset.testState = 'error';
  });
}

/**
 * Whether to measure on load.
 *
 * Three sources, most specific first: the URL, this browser's remembered
 * choice, then the deployment's default. The URL wins for one page load and is
 * deliberately not remembered — a link someone sent you should not silently
 * reconfigure your browser.
 */
function shouldAutostart(serverDefault: boolean): boolean {
  const asked = new URLSearchParams(window.location.search).get('autostart');
  if (asked !== null) return !/^(0|false|no|off)$/i.test(asked.trim());

  try {
    const stored = localStorage.getItem(AUTOSTART_KEY);
    if (stored !== null) return stored === '1';
  } catch {
    // Private browsing, or storage disabled.
  }
  return serverDefault;
}

/**
 * The page, ready but not measuring.
 *
 * A test moves hundreds of megabytes. Opening the page to read the history or
 * change a setting should not do that behind your back — least of all during
 * the video call that made you suspicious of the network in the first place.
 */
function showIdle(message: string): void {
  showNow('Ready', undefined, false);
  ui.restart.disabled = false;
  ui.pause.disabled = true;
  ui.phaseDetail.textContent = '';
  ui.idleNote.textContent = message;
  ui.idleNote.hidden = false;
  document.body.dataset.testState = 'idle';
}

async function bootstrap(): Promise<void> {
  await setUpProfilePicker();

  // A deployment that cannot be reached for its default gets the historical
  // behaviour, so a status endpoint failing never leaves the page inert.
  let serverDefault = true;
  try {
    serverDefault = (await fetchStatus()).autostart;
  } catch {
    // Keep the default; `run` will surface the real failure.
  }

  const on = shouldAutostart(serverDefault);
  ui.autostart.checked = on;
  if (on) {
    await start();
    return;
  }
  showIdle('Auto-start is off. Press Retest when you are ready to measure.');
}

void bootstrap();

// Light or dark, remembered, on every page.
setUpThemeToggle();
