import { describe, expect, it } from 'vitest';

import { suggestedProfile } from './parallel';

describe('suggestedProfile', () => {
  it('sizes a transfer to last roughly a second in total', () => {
    // 1 Gbps across 4 streams for ~1s => ~31 MB each.
    const p = suggestedProfile(1e9);
    expect(p.streams).toBe(4);
    const seconds = (p.bytesPerStream * 8 * p.streams) / 1e9;
    expect(seconds).toBeGreaterThan(0.5);
    expect(seconds).toBeLessThan(2);
  });

  it('does not ask a 10 GbE link for an absurd payload', () => {
    // Sizes are capped: the point is a measurement, not a bandwidth-hogging
    // stunt, and a browser holding 1 GB in flight helps nobody.
    const p = suggestedProfile(10e9);
    expect(p.bytesPerStream).toBeLessThanOrEqual(256 * 1024 * 1024);
  });

  it('keeps a floor so a slow link still transfers something measurable', () => {
    const p = suggestedProfile(1e6);
    expect(p.bytesPerStream).toBeGreaterThanOrEqual(4 * 1024 * 1024);
  });

  it('always returns whole bytes', () => {
    for (const bps of [1e6, 1e8, 9.37e8, 1e10]) {
      const p = suggestedProfile(bps);
      expect(Number.isInteger(p.bytesPerStream)).toBe(true);
      expect(p.bytesPerStream).toBeGreaterThan(0);
    }
  });

  it('scales with the expected rate between the bounds', () => {
    const slow = suggestedProfile(1e8);
    const fast = suggestedProfile(1e9);
    expect(fast.bytesPerStream).toBeGreaterThan(slow.bytesPerStream);
  });
});
