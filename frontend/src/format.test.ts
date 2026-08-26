import { describe, expect, it } from 'vitest';
import { formatLatency, latencyPredatesNagleFix } from './format';

describe('formatLatency', () => {
  it('keeps two decimals, because a LAN round trip is now well under a millisecond', () => {
    // 1.3.1 removed a ~40 ms Nagle stall; the guest now answers in about
    // 0.6 ms, where one decimal throws away a sixth of the figure.
    expect(formatLatency(0.6)).toBe('0.60 ms');
    expect(formatLatency(0.35)).toBe('0.35 ms');
    expect(formatLatency(0.85)).toBe('0.85 ms');
    expect(formatLatency(41.9)).toBe('41.90 ms');
  });

  it('does not claim more precision than it has at the floor', () => {
    expect(formatLatency(0.004)).toBe('<0.01 ms');
    expect(formatLatency(0)).toBe('<0.01 ms');
  });

  it('reports nothing rather than zero when there is no measurement', () => {
    expect(formatLatency(undefined)).toBe('—');
    expect(formatLatency(null)).toBe('—');
    expect(formatLatency(Number.NaN)).toBe('—');
  });
});

describe('latencyPredatesNagleFix', () => {
  it('flags every release before 1.3.1', () => {
    expect(latencyPredatesNagleFix('1.3.0')).toBe(true);
    expect(latencyPredatesNagleFix('1.2.0')).toBe(true);
    expect(latencyPredatesNagleFix('0.6.1')).toBe(true);
  });

  it('clears 1.3.1 and everything after it', () => {
    expect(latencyPredatesNagleFix('1.3.1')).toBe(false);
    expect(latencyPredatesNagleFix('1.4.0')).toBe(false);
    expect(latencyPredatesNagleFix('2.0.0')).toBe(false);
    // Two-digit components must compare numerically, not as strings: "1.10.0"
    // sorts before "1.3.1" alphabetically and is in fact after it.
    expect(latencyPredatesNagleFix('1.10.0')).toBe(false);
  });

  it('treats an unknown version as suspect rather than assuming the best', () => {
    // A run with no version was stored before the version was tracked, so it
    // may or may not predate the fix. Saying "may be inflated" is honest;
    // saying nothing would quietly present a possibly wrong number as sound.
    expect(latencyPredatesNagleFix(null)).toBe(true);
    expect(latencyPredatesNagleFix(undefined)).toBe(true);
    expect(latencyPredatesNagleFix('')).toBe(true);
    expect(latencyPredatesNagleFix('nonsense')).toBe(true);
    expect(latencyPredatesNagleFix('1.3')).toBe(true);
  });
});
