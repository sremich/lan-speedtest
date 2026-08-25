import { describe, expect, it } from 'vitest';

import {
  bandwidthBySize,
  formatTransferSize,
  percentile,
  summarise,
  type Summary,
} from './stats';

describe('percentile', () => {
  // Worked against the type-7 definition, which is what NumPy and Excel use —
  // so a number shown in the UI matches what someone gets checking by hand.
  const sample = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

  it('interpolates between ranks', () => {
    expect(percentile(sample, 0)).toBe(1);
    expect(percentile(sample, 1)).toBe(10);
    expect(percentile(sample, 0.5)).toBeCloseTo(5.5, 10);
    expect(percentile(sample, 0.25)).toBeCloseTo(3.25, 10);
    expect(percentile(sample, 0.75)).toBeCloseTo(7.75, 10);
  });

  it('handles a single sample', () => {
    expect(percentile([42], 0)).toBe(42);
    expect(percentile([42], 0.5)).toBe(42);
    expect(percentile([42], 1)).toBe(42);
  });

  it('clamps out-of-range probabilities rather than reading past the array', () => {
    expect(percentile(sample, -1)).toBe(1);
    expect(percentile(sample, 2)).toBe(10);
  });

  it('refuses an empty set instead of inventing a number', () => {
    expect(() => percentile([], 0.5)).toThrow();
  });
});

describe('summarise', () => {
  it('returns null for no samples rather than a summary of zeros', () => {
    // Rendering zeros for "no data" would be a lie about the measurement.
    expect(summarise([])).toBeNull();
    expect(summarise([Number.NaN, Number.POSITIVE_INFINITY])).toBeNull();
  });

  it('computes the five-number summary and mean', () => {
    const s = summarise([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) as Summary;
    expect(s.count).toBe(10);
    expect(s.min).toBe(1);
    expect(s.max).toBe(10);
    expect(s.mean).toBeCloseTo(5.5, 10);
    expect(s.median).toBeCloseTo(5.5, 10);
    expect(s.p25).toBeCloseTo(3.25, 10);
    expect(s.p75).toBeCloseTo(7.75, 10);
    expect(s.iqr).toBeCloseTo(4.5, 10);
  });

  it('is not fooled by unsorted input', () => {
    const a = summarise([5, 1, 4, 2, 3]) as Summary;
    const b = summarise([1, 2, 3, 4, 5]) as Summary;
    expect(a).toEqual(b);
  });

  it('ignores non-finite samples but keeps the rest', () => {
    const s = summarise([1, Number.NaN, 2, Number.POSITIVE_INFINITY, 3]) as Summary;
    expect(s.count).toBe(3);
    expect(s.max).toBe(3);
  });

  it('flags outliers beyond the Tukey fences', () => {
    // 100 is far outside 1.5×IQR of a tight 1..9 cluster.
    const s = summarise([1, 2, 3, 4, 5, 6, 7, 8, 9, 100]) as Summary;
    expect(s.outliers).toContain(100);
    expect(s.max).toBe(100);
    // The whisker stops at the furthest real sample inside the fence, not at
    // the fence itself — drawing to the fence would invent a value.
    expect(s.upperWhisker).toBe(9);
    expect(s.outliers.every((o) => o > s.upperWhisker)).toBe(true);
  });

  it('flags low outliers too', () => {
    // A single slow request is exactly the symptom worth seeing.
    const s = summarise([1, 90, 92, 94, 96, 98, 100]) as Summary;
    expect(s.outliers).toContain(1);
    expect(s.lowerWhisker).toBeGreaterThan(1);
  });

  it('reports no outliers when every sample is identical', () => {
    // A zero IQR would otherwise make every sample away from the median an
    // outlier, which is technically true and completely useless.
    const s = summarise([5, 5, 5, 5]) as Summary;
    expect(s.iqr).toBe(0);
    expect(s.outliers).toEqual([]);
    expect(s.lowerWhisker).toBe(5);
    expect(s.upperWhisker).toBe(5);
  });

  it('handles a single sample without pretending to a spread', () => {
    const s = summarise([7]) as Summary;
    expect(s.count).toBe(1);
    expect(s.min).toBe(7);
    expect(s.max).toBe(7);
    expect(s.median).toBe(7);
    expect(s.iqr).toBe(0);
    expect(s.outliers).toEqual([]);
  });

  it('a whisker never ends inside its own box', () => {
    // [1, 2, 3, 100]: p75 is 27.25 but the largest non-outlier is 3, so an
    // unclamped upper whisker would sit inside the box and the plot would
    // render back-to-front. Caught by the box-geometry test in CI.
    const s = summarise([1, 2, 3, 100]) as Summary;
    expect(s.outliers).toContain(100);
    expect(s.upperWhisker).toBeGreaterThanOrEqual(s.p75);
    expect(s.lowerWhisker).toBeLessThanOrEqual(s.p25);
  });

  it('whiskers bracket the box for any sample set', () => {
    const sets = [
      [1, 2, 3, 100],
      [100, 3, 2, 1],
      [1, 1, 1, 1, 999],
      [5],
      [5, 5],
      [0.1, 0.2, 0.3, 50, 60],
      [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    ];
    for (const samples of sets) {
      const s = summarise(samples) as Summary;
      expect(s.lowerWhisker, `${samples}`).toBeLessThanOrEqual(s.p25);
      expect(s.upperWhisker, `${samples}`).toBeGreaterThanOrEqual(s.p75);
    }
  });

  it('whiskers always sit inside min and max', () => {
    for (const samples of [
      [1, 2, 3],
      [1, 1, 1, 50],
      [10, 20, 30, 40, 50, 60, 70, 80, 900],
      [0.1, 0.1, 0.2, 5],
    ]) {
      const s = summarise(samples) as Summary;
      expect(s.lowerWhisker).toBeGreaterThanOrEqual(s.min);
      expect(s.upperWhisker).toBeLessThanOrEqual(s.max);
      expect(s.p25).toBeLessThanOrEqual(s.median);
      expect(s.median).toBeLessThanOrEqual(s.p75);
    }
  });
});

describe('bandwidthBySize', () => {
  const points = [
    { bytes: 1e6, bps: 100 },
    { bytes: 1e6, bps: 200 },
    { bytes: 1e8, bps: 900 },
    { bytes: 1e8, bps: 950 },
    { bytes: 1e8, bps: 1000 },
  ];

  it('groups by transfer size, largest first', () => {
    // Largest first because that is the size that actually measured the link;
    // a small transfer measures round-trip overhead more than throughput.
    const rows = bandwidthBySize(points, formatTransferSize);
    expect(rows).toHaveLength(2);
    expect(rows[0]!.label).toBe('100 MB');
    expect(rows[0]!.summary.count).toBe(3);
    expect(rows[1]!.label).toBe('1 MB');
    expect(rows[1]!.summary.count).toBe(2);
  });

  it('never mixes sizes into one distribution', () => {
    const rows = bandwidthBySize(points, formatTransferSize);
    expect(rows[0]!.summary.min).toBe(900);
    expect(rows[1]!.summary.max).toBe(200);
  });

  it('describes how many requests produced each row', () => {
    const rows = bandwidthBySize(points, formatTransferSize);
    expect(rows[0]!.detail).toBe('3 requests');
    expect(bandwidthBySize([{ bytes: 1e6, bps: 1 }], formatTransferSize)[0]!.detail).toBe(
      '1 request',
    );
  });

  it('drops sizes whose samples are all unusable', () => {
    const rows = bandwidthBySize(
      [
        { bytes: 1e6, bps: Number.NaN },
        { bytes: 1e8, bps: 500 },
      ],
      formatTransferSize,
    );
    expect(rows).toHaveLength(1);
    expect(rows[0]!.label).toBe('100 MB');
  });

  it('returns nothing for no points', () => {
    expect(bandwidthBySize([], formatTransferSize)).toEqual([]);
  });
});

describe('formatTransferSize', () => {
  it('reads the way the profile is written', () => {
    expect(formatTransferSize(100_000)).toBe('100 kB');
    expect(formatTransferSize(1_000_000)).toBe('1 MB');
    expect(formatTransferSize(25_000_000)).toBe('25 MB');
    expect(formatTransferSize(67_108_864)).toBe('67.1 MB');
    expect(formatTransferSize(250_000_000)).toBe('250 MB');
    expect(formatTransferSize(512)).toBe('512 B');
  });
});
