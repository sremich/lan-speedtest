import { describe, expect, it } from 'vitest';
import { changePct, formatChange, verdictFor } from './comparemath';

describe('changePct', () => {
  it('is the change from A to B as a share of A', () => {
    expect(changePct(100, 150)).toBeCloseTo(50);
    expect(changePct(100, 50)).toBeCloseTo(-50);
    expect(changePct(940e6, 470e6)).toBeCloseTo(-50);
  });

  it('refuses to divide by nothing', () => {
    // 0 -> 5 is not "infinitely better", it is a run that measured nothing
    // and a run that did. Rendering "+∞%" would be a confident-looking lie.
    expect(changePct(0, 5)).toBeNull();
  });

  it('has no answer when a run did not measure the thing', () => {
    expect(changePct(null, 100)).toBeNull();
    expect(changePct(100, null)).toBeNull();
    expect(changePct(undefined, undefined)).toBeNull();
    expect(changePct(Number.NaN, 100)).toBeNull();
    expect(changePct(100, Number.POSITIVE_INFINITY)).toBeNull();
  });

  it('handles a negative baseline without flipping the sign', () => {
    // Divided by |A|, so "went further from zero" stays negative either way.
    expect(changePct(-10, -20)).toBeCloseTo(-100);
  });
});

describe('verdictFor', () => {
  it('reads the same number as opposite news depending on the metric', () => {
    // This is the whole reason the function exists: +10% download is good,
    // +10% latency is bad, and colouring both green would be worse than
    // colouring neither.
    expect(verdictFor(10, true)).toBe('better');
    expect(verdictFor(10, false)).toBe('worse');
    expect(verdictFor(-10, true)).toBe('worse');
    expect(verdictFor(-10, false)).toBe('better');
  });

  it('calls a small difference no difference', () => {
    // Two runs of the same link differ by a percent or two every time. Calling
    // that an improvement would make the page cry wolf on every comparison.
    expect(verdictFor(1.9, true)).toBe('same');
    expect(verdictFor(-1.9, false)).toBe('same');
    expect(verdictFor(0, true)).toBe('same');
  });

  it('says nothing when there is nothing to say', () => {
    expect(verdictFor(null, true)).toBe('none');
    expect(verdictFor(null, false)).toBe('none');
  });
});

describe('formatChange', () => {
  it('signs the number so the direction is readable without the colour', () => {
    expect(formatChange(12.34)).toBe('+12.3%');
    expect(formatChange(-12.34)).toBe('-12.3%');
    expect(formatChange(0)).toBe('0.0%');
    expect(formatChange(null)).toBe('—');
  });
});
