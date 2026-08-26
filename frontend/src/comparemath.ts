/**
 * The arithmetic behind the compare page, kept apart so it can be tested.
 *
 * The page itself imports the stylesheet, which the unit runner cannot load —
 * and this is the part actually worth testing anyway. Everything here is pure.
 */

/** A change small enough to be two runs of the same link rather than a finding. */
export const NOISE_PCT = 2;

export type Verdict = 'better' | 'worse' | 'same' | 'none';

/**
 * The change from `a` to `b`, as a percentage of `a`.
 *
 * `null` rather than a number when either side is missing or when `a` is zero:
 * a percentage of nothing is undefined, not infinite, and "+∞%" is a
 * confident-looking lie about a run that simply did not measure that thing.
 */
export function changePct(a: number | null | undefined, b: number | null | undefined): number | null {
  if (a === null || a === undefined || b === null || b === undefined) return null;
  if (!Number.isFinite(a) || !Number.isFinite(b)) return null;
  if (a === 0) return null;
  return ((b - a) / Math.abs(a)) * 100;
}

/**
 * Whether a change is an improvement.
 *
 * Signed by improvement rather than by arithmetic, which is the whole point:
 * +10% download and +10% latency are opposite news, and a table that coloured
 * both green would be worse than one with no colour at all.
 */
export function verdictFor(pct: number | null, higherIsBetter: boolean): Verdict {
  if (pct === null) return 'none';
  if (Math.abs(pct) < NOISE_PCT) return 'same';
  return (higherIsBetter ? pct > 0 : pct < 0) ? 'better' : 'worse';
}

/** The change as it is written in the table. */
export function formatChange(pct: number | null): string {
  if (pct === null) return '—';
  return `${pct > 0 ? '+' : ''}${pct.toFixed(1)}%`;
}
