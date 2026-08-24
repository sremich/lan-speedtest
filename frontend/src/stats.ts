/**
 * Summary statistics for a set of samples.
 *
 * The engine reports a single percentile per metric (the 90th for bandwidth,
 * the median for latency), which hides how consistent a result was. A run that
 * averages 940 Mbps with every sample within 2% is a very different network
 * from one that averages 940 Mbps by alternating between 500 and 1300 — and
 * the second is what a failing cable or a duplex mismatch looks like.
 *
 * So these are computed from the engine's raw points and shown per transfer
 * size, in the spirit of speed.cloudflare.com's detail view.
 */

export interface Summary {
  count: number;
  min: number;
  max: number;
  mean: number;
  median: number;
  /** 25th percentile — the bottom of the box. */
  p25: number;
  /** 75th percentile — the top of the box. */
  p75: number;
  /** p75 − p25. */
  iqr: number;
  /** Whisker ends: the furthest samples still within 1.5×IQR of the box. */
  lowerWhisker: number;
  upperWhisker: number;
  /** Samples beyond the whiskers, ascending. */
  outliers: number[];
}

/**
 * Linear-interpolated percentile (the "type 7" definition).
 *
 * This is what NumPy, Excel and most spreadsheets do by default, so a figure
 * here matches what someone gets checking the numbers by hand.
 *
 * @param sorted ascending samples; must not be empty
 * @param p in [0, 1]
 */
export function percentile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) throw new Error('percentile of an empty set');
  if (sorted.length === 1) return sorted[0]!;

  const clamped = Math.min(1, Math.max(0, p));
  const rank = clamped * (sorted.length - 1);
  const lo = Math.floor(rank);
  const hi = Math.ceil(rank);
  if (lo === hi) return sorted[lo]!;
  return sorted[lo]! + (rank - lo) * (sorted[hi]! - sorted[lo]!);
}

/**
 * Summarises samples, ignoring anything non-finite.
 *
 * Returns `null` for an empty set rather than a summary of nothing — a caller
 * that renders zeros for "no data" is lying about the measurement.
 */
export function summarise(samples: readonly number[]): Summary | null {
  const clean = samples.filter((v) => Number.isFinite(v));
  if (clean.length === 0) return null;

  const sorted = [...clean].sort((a, b) => a - b);
  const p25 = percentile(sorted, 0.25);
  const p75 = percentile(sorted, 0.75);
  const iqr = p75 - p25;

  // Tukey fences. With a tight distribution the IQR can be 0, in which case
  // every sample away from the median reads as an outlier — which is correct
  // but useless, so a zero IQR yields no outliers at all.
  const lowerFence = iqr === 0 ? sorted[0]! : p25 - 1.5 * iqr;
  const upperFence = iqr === 0 ? sorted[sorted.length - 1]! : p75 + 1.5 * iqr;

  const inside = sorted.filter((v) => v >= lowerFence && v <= upperFence);
  const outliers = sorted.filter((v) => v < lowerFence || v > upperFence);

  return {
    count: sorted.length,
    min: sorted[0]!,
    max: sorted[sorted.length - 1]!,
    mean: sorted.reduce((a, b) => a + b, 0) / sorted.length,
    median: percentile(sorted, 0.5),
    p25,
    p75,
    iqr,
    // Whiskers reach the furthest *actual* sample inside the fence, not the
    // fence itself — drawing to the fence invents a value nothing measured.
    lowerWhisker: inside.length > 0 ? inside[0]! : sorted[0]!,
    upperWhisker: inside.length > 0 ? inside[inside.length - 1]! : sorted[sorted.length - 1]!,
    outliers,
  };
}

/** One row of the detail view: a labelled distribution. */
export interface Distribution {
  label: string;
  /** Extra context, e.g. how many requests of what size produced it. */
  detail: string;
  summary: Summary;
}

export interface BandwidthPointLike {
  bytes: number;
  bps: number;
}

/**
 * Groups bandwidth samples by transfer size, largest size first.
 *
 * Per-size is the useful split: a small transfer measures round-trip overhead
 * more than throughput, so mixing sizes into one distribution would make a
 * healthy link look wildly inconsistent.
 */
export function bandwidthBySize(
  points: readonly BandwidthPointLike[],
  formatBytes: (bytes: number) => string,
): Distribution[] {
  const bySize = new Map<number, number[]>();
  for (const p of points) {
    if (!Number.isFinite(p.bps)) continue;
    const list = bySize.get(p.bytes);
    if (list) list.push(p.bps);
    else bySize.set(p.bytes, [p.bps]);
  }

  return [...bySize.entries()]
    .sort((a, b) => b[0] - a[0])
    .map(([bytes, samples]) => {
      const summary = summarise(samples);
      return summary
        ? {
            label: formatBytes(bytes),
            detail: `${summary.count} request${summary.count === 1 ? '' : 's'}`,
            summary,
          }
        : null;
    })
    .filter((d): d is Distribution => d !== null);
}

/** Human byte size for a transfer-size label. */
export function formatTransferSize(bytes: number): string {
  if (bytes >= 1e6) {
    const mb = bytes / 1e6;
    return `${Number.isInteger(mb) ? mb : mb.toFixed(1)} MB`;
  }
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} kB`;
  return `${bytes} B`;
}
