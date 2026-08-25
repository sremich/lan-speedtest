import { describe, expect, it } from 'vitest';

import { renderAreaChart } from './areachart';

/** The `y1` of the marker line, in viewBox units. */
function markerLineY(html: string): number {
  const m = /class="trace__marker"[^>]*/.exec(html) ?? /<line[^>]*trace__marker[^>]*>/.exec(html);
  const line = /<line[^>]*class="trace__marker"[^>]*>/.exec(html)?.[0] ?? m?.[0] ?? '';
  const y = /y1="([\d.]+)"/.exec(line);
  if (!y) throw new Error(`no marker line in: ${html.slice(0, 200)}`);
  return Number(y[1]);
}

/** The `top` percentage the label was given. */
function labelTopPct(html: string): number {
  const m = /class="trace__marker-label"[^>]*style="top: ([\d.]+)%"/.exec(html);
  if (!m) throw new Error('no marker label');
  return Number(m[1]);
}

function labelSide(html: string): string {
  return /data-side="(above|below)"/.exec(html)?.[1] ?? '';
}

const CHART_H = 150;

describe('the marker label', () => {
  it('sits on its own line, whatever height that line is at', () => {
    // It used to clamp the label's percentage but not the line's, so the two
    // drifted apart by however much the clamp bit — which is why download and
    // upload showed the label at visibly different distances from the line.
    const nearTop = renderAreaChart([90, 95, 99, 100], {
      colour: 'red',
      id: 'a',
      marker: { value: 100, label: '90th percentile' },
    });
    const lower = renderAreaChart([10, 40, 100, 20], {
      colour: 'blue',
      id: 'b',
      marker: { value: 30, label: '90th percentile' },
    });

    for (const chart of [nearTop, lower]) {
      const linePct = (markerLineY(chart.html) / CHART_H) * 100;
      expect(labelTopPct(chart.html)).toBeCloseTo(linePct, 1);
    }
  });

  it('puts the two charts the same distance from their lines', () => {
    // The property the request was actually about: whatever each marker's
    // height, the offset between label and line is identical, because it is a
    // fixed CSS offset rather than anything computed here.
    const a = renderAreaChart([90, 95, 99, 100], {
      colour: 'red',
      id: 'a',
      marker: { value: 96, label: '90th percentile' },
    });
    const b = renderAreaChart([10, 40, 100, 20], {
      colour: 'blue',
      id: 'b',
      marker: { value: 44, label: '90th percentile' },
    });

    const offsetA = labelTopPct(a.html) - (markerLineY(a.html) / CHART_H) * 100;
    const offsetB = labelTopPct(b.html) - (markerLineY(b.html) / CHART_H) * 100;

    // Not exactly zero: the line rounds to one decimal and the label to two,
    // so up to 0.05% of 150px — under a tenth of a pixel — can survive. The
    // point is that neither offset is a computed adjustment.
    expect(Math.abs(offsetA - offsetB)).toBeLessThan(0.05);
    expect(Math.abs(offsetA)).toBeLessThan(0.05);
  });

  it('flips below the line only when there is no room above it', () => {
    // A marker at the very top would otherwise push its label out of the card.
    const high = renderAreaChart([1, 1, 1, 100], {
      colour: 'red',
      id: 'c',
      marker: { value: 100, label: '90th percentile' },
    });
    const middling = renderAreaChart([1, 50, 100, 40], {
      colour: 'red',
      id: 'd',
      marker: { value: 40, label: '90th percentile' },
    });

    expect(labelSide(high.html)).toBe('below');
    expect(labelSide(middling.html)).toBe('above');
  });

  it('draws no marker at all when there is nothing to mark', () => {
    const none = renderAreaChart([1, 2, 3], { colour: 'red', id: 'e' });
    expect(none.html).not.toContain('trace__marker');
  });
});
