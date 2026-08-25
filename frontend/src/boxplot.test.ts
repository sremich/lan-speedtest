import { describe, expect, it } from 'vitest';

import { ariaSummary, boxTipMarkup, DEFAULT_W, renderBoxPlots } from './boxplot';
import { summarise, type Distribution } from './stats';

function rows(): Distribution[] {
  const summary = summarise([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
  if (!summary) throw new Error('fixture should summarise');
  return [{ label: 'a', detail: '10 samples', summary }];
}

/** The `width` in an `<svg width="...">` attribute. */
function widthAttr(svg: string): number {
  const m = /<svg[^>]*\swidth="(\d+)"/.exec(svg);
  if (!m) throw new Error(`no width attribute in: ${svg.slice(0, 120)}`);
  return Number(m[1]);
}

describe('renderBoxPlots sizing', () => {
  it('draws at the width it is given', () => {
    // Real pixel dimensions are what hold the scale factor at 1. A viewBox
    // stretched to fit would scale the 11px labels and the 34px rows along
    // with it — unreadably small in a narrow window, bloated on a wide one.
    const svg = renderBoxPlots(rows(), { format: String, width: 1180 });
    expect(widthAttr(svg)).toBe(1180);
    expect(svg).toContain('viewBox="0 0 1180');
  });

  it('falls back to a usable width when the container has not been measured', () => {
    const svg = renderBoxPlots(rows(), { format: String });
    expect(widthAttr(svg)).toBe(DEFAULT_W);
  });

  it('stops shrinking before the axis labels collide', () => {
    // Below the floor the drawing is scaled down by CSS instead, which keeps
    // the tick labels legible rather than letting them overlap.
    const svg = renderBoxPlots(rows(), { format: String, width: 120 });
    expect(widthAttr(svg)).toBeGreaterThanOrEqual(360);
  });

  it('draws nothing at all when there is nothing to draw', () => {
    expect(renderBoxPlots([], { format: String, width: 800 })).toBe('');
  });
});

describe('the distribution tooltip', () => {
  const data = {
    label: '25 MB (4)',
    detail: '4 requests',
    min: '852.5 Mbps',
    max: '926.8 Mbps',
    mean: '904.2 Mbps',
    median: '918.8 Mbps',
    p25: '897.1 Mbps',
    p75: '925.8 Mbps',
    samples: 4,
    outliers: 1,
  };

  it('explains what the marks mean before listing the figures', () => {
    // A box plot is only self-evident to people who already read box plots.
    // The sentence is the point of the redesign, so assert it is there.
    const html = boxTipMarkup(data);
    expect(html).toMatch(/25th to the 75th percentile/i);
    expect(html).toMatch(/median/i);
    expect(html).toMatch(/average/i);
    expect(html).toMatch(/outlier/i);
  });

  it('labels every figure in words rather than in statistics notation', () => {
    const html = boxTipMarkup(data);
    for (const label of ['Min', 'Max', 'Average', 'Median', '25th percentile', '75th percentile']) {
      expect(html, `missing label ${label}`).toContain(`>${label}<`);
    }
    // The old abbreviations should be gone: "p25" is not a human-readable label.
    expect(html).not.toMatch(/>p25</);
    expect(html).not.toMatch(/>p75</);
  });

  it('shows every value it was given', () => {
    const html = boxTipMarkup(data);
    for (const v of [data.min, data.max, data.mean, data.median, data.p25, data.p75]) {
      expect(html).toContain(v);
    }
    expect(html).toContain('4 samples');
    expect(html).toContain('1 outlier');
  });

  it('counts no outliers when there are none, and keeps the plural honest', () => {
    // The explainer sentence mentions outliers whatever the data, so this
    // looks only at the footer, which is where the count lives.
    const foot = (html: string) => /<div class="tip__foot">(.*?)<\/div>/.exec(html)?.[1] ?? '';

    expect(foot(boxTipMarkup({ ...data, outliers: 0, samples: 1 }))).toBe('1 sample');
    expect(foot(boxTipMarkup({ ...data, outliers: 1, samples: 4 }))).toBe('4 samples · 1 outlier');
    expect(foot(boxTipMarkup({ ...data, outliers: 3, samples: 9 }))).toBe('9 samples · 3 outliers');
  });

  it('escapes the label, which is built from measured values', () => {
    const html = boxTipMarkup({ ...data, label: '<img src=x onerror=alert(1)>' });
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img');
  });
});

describe('the row summary for assistive technology', () => {
  it('states the figures a hover would reveal, since it cannot hover', () => {
    const summary = ariaSummary({
      label: '25 MB (4)',
      detail: '4 requests',
      min: '852.5 Mbps',
      max: '926.8 Mbps',
      mean: '904.2 Mbps',
      median: '918.8 Mbps',
      p25: '897.1 Mbps',
      p75: '925.8 Mbps',
      samples: 4,
      outliers: 1,
    });
    expect(summary).toContain('25 MB (4)');
    expect(summary).toContain('918.8 Mbps');
    expect(summary).toContain('4 samples');
  });
});

describe('the rendered rows', () => {
  it('carry their figures as data, and no native title to compete with', () => {
    const svg = renderBoxPlots(rows(), { format: (v) => `${v} u`, width: 800 });
    expect(svg).toContain('data-median=');
    expect(svg).toContain('data-samples=');
    expect(svg).toContain('aria-label=');
    // A <title> would make the browser draw its own tooltip over ours.
    expect(svg).not.toContain('<title>');
  });
});
