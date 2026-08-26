/**
 * Number formatting.
 *
 * The guiding rule is not to imply precision that does not exist. Browsers
 * coarsen `PerformanceResourceTiming` for privacy — roughly 0.1 ms in Chrome
 * and 1 ms in Firefox — so on a LAN with sub-millisecond round trips the
 * latency figures are quantised by the browser, not by the network. Latency
 * is therefore shown to one decimal at most, and anything under the browser's
 * own resolution is reported as such rather than as a confident small number.
 */

const PENDING = '—';

/** Chrome coarsens to ~100µs; Firefox to ~1ms. Take the coarser as the floor. */
export const LATENCY_RESOLUTION_MS = 1;

export function formatBandwidth(bps: number | undefined): { value: string; unit: string } {
  if (bps === undefined || !Number.isFinite(bps)) return { value: PENDING, unit: '' };
  if (bps >= 1e9) return { value: (bps / 1e9).toFixed(2), unit: 'Gbps' };
  if (bps >= 1e6) return { value: (bps / 1e6).toFixed(1), unit: 'Mbps' };
  if (bps >= 1e3) return { value: (bps / 1e3).toFixed(0), unit: 'kbps' };
  return { value: bps.toFixed(0), unit: 'bps' };
}

/**
 * Latency, to two decimals.
 *
 * One decimal was enough when a LAN round trip read tens of milliseconds. Since
 * 1.3.1 removed the Nagle stall it reads about 0.6 ms, where a tenth of a
 * millisecond is a sixth of the whole figure — and the distributions in the
 * detail view have always shown two decimals, so the headline was the odd one
 * out and looked less precise than the numbers underneath it.
 */
export function formatLatency(ms: number | undefined | null): string {
  if (ms === undefined || ms === null || !Number.isFinite(ms)) return PENDING;
  if (ms < 0.005) return '<0.01 ms';
  return `${ms.toFixed(2)} ms`;
}

/**
 * A three-part version as one comparable number, or `null` if it is not one.
 *
 * Compared numerically rather than as text, because "1.10.0" sorts before
 * "1.3.1" alphabetically and comes after it in fact. The bases cap minor and
 * patch at 999, which this project will not reach.
 */
function versionKey(text: string): number | null {
  const parts = /^(\d+)\.(\d+)\.(\d+)$/.exec(text);
  if (!parts) return null;
  const [, major = '0', minor = '0', patch = '0'] = parts;
  return Number(major) * 1_000_000 + Number(minor) * 1_000 + Number(patch);
}

/** The release that stopped `TCP_NODELAY` inflating every latency reading. */
const NAGLE_FIX = versionKey('1.3.1') ?? 0;

/**
 * Whether a run's latency figures predate the `TCP_NODELAY` fix.
 *
 * Runs recorded before 1.3.1 have their latency inflated by up to 40 ms, which
 * makes them incomparable with anything measured since. A run with no recorded
 * version was stored before the version was tracked at all, which is a weaker
 * claim than a version — it is treated as suspect because it might be, and
 * saying so is the honest reading.
 */
export function latencyPredatesNagleFix(appVersion: string | null | undefined): boolean {
  if (!appVersion) return true;
  const key = versionKey(appVersion);
  return key === null || key < NAGLE_FIX;
}

export function formatPacketLoss(ratio: number | undefined): string {
  if (ratio === undefined || !Number.isFinite(ratio)) return PENDING;
  const pct = ratio * 100;
  if (pct === 0) return '0%';
  if (pct < 0.1) return '<0.1%';
  return `${pct.toFixed(1)}%`;
}

export function formatDuration(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms)) return PENDING;
  return `${(ms / 1000).toFixed(1)} s`;
}

/**
 * True when every latency reading sits close enough to the browser's timing
 * resolution that the numbers are mostly quantisation. On a healthy LAN this
 * is the normal case, and the UI says so instead of pretending otherwise.
 */
export function latencyIsAtBrowserResolution(...values: Array<number | null | undefined>): boolean {
  const real = values.filter((v): v is number => typeof v === 'number' && Number.isFinite(v));
  return real.length > 0 && real.every((v) => v <= LATENCY_RESOLUTION_MS * 2);
}
