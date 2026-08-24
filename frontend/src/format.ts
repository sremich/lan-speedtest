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

export function formatLatency(ms: number | undefined | null): string {
  if (ms === undefined || ms === null || !Number.isFinite(ms)) return PENDING;
  if (ms < 0.05) return '<0.1 ms';
  return `${ms.toFixed(1)} ms`;
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
