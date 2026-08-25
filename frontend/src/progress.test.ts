import { describe, expect, it } from 'vitest';

import {
  formatPayload,
  planStages,
  renderChevrons,
  stageAt,
  totalRequests,
  type StageSpec,
} from './progress';

/**
 * The engine's own default profile, abridged to the shape that matters.
 *
 * Its fourth stage is what the reference site labels "Payload: 100 kB,
 * Requests: 9, Step: 4 of 25", which is what pins this model down.
 */
const ENGINE_DEFAULT: StageSpec[] = [
  { type: 'latency', numPackets: 2 },
  { type: 'download', bytes: 1e5, count: 1 },
  { type: 'latency', numPackets: 20 },
  { type: 'download', bytes: 1e5, count: 9 },
  { type: 'latency', numPackets: 2 },
];

describe('planStages', () => {
  it('numbers steps from one and places each stage after the last', () => {
    const stages = planStages(ENGINE_DEFAULT);
    expect(stages.map((s) => s.step)).toEqual([1, 2, 3, 4, 5]);
    expect(stages.map((s) => s.requests)).toEqual([2, 1, 20, 9, 2]);
    expect(stages.map((s) => s.offset)).toEqual([0, 2, 3, 23, 32]);
    expect(totalRequests(stages)).toBe(34);
  });

  it('reproduces the reference tooltip for the stage it was checked against', () => {
    const fourth = planStages(ENGINE_DEFAULT)[3]!;
    expect(fourth.label).toBe('Download');
    expect(formatPayload(fourth.bytes!)).toBe('100 kB');
    expect(fourth.requests).toBe(9);
    expect(fourth.step).toBe(4);
  });

  it('alternates shade between consecutive stages of the same type', () => {
    // Three downloads in a row would otherwise read as one long block.
    const stages = planStages([
      { type: 'download', count: 1 },
      { type: 'download', count: 1 },
      { type: 'download', count: 1 },
    ]);
    expect(stages.map((s) => s.shade)).toEqual([0, 1, 0]);
  });

  it('marks a warm-up round, which measures nothing by design', () => {
    // The engine exempts it from the minimum-duration rule and then leaves it
    // out of the bandwidth points, so a step that reported no figure looked
    // like a failure rather than a warm-up.
    const [warm, real] = planStages([
      { type: 'download', bytes: 1e6, count: 1, bypassMinDuration: true },
      { type: 'download', bytes: 25e6, count: 4 },
    ]);
    expect(warm!.warmUp).toBe(true);
    expect(warm!.label).toMatch(/warm-up/i);
    expect(real!.warmUp).toBe(false);
    expect(real!.label).toBe('Download');
  });

  it('gives packet loss a visible block rather than one chevron', () => {
    // It reports a single figure at the end but takes seconds; a lone chevron
    // would make the strip look stalled for the whole stage.
    const [stage] = planStages([{ type: 'packetLoss', numPackets: 1000 }]);
    expect(stage!.requests).toBeGreaterThan(1);
  });

  it('never gives a stage zero width', () => {
    // A stage drawn as nothing at all is worse than one drawn approximately.
    for (const spec of [
      { type: 'latency' },
      { type: 'download' },
      { type: 'upload', count: 0 },
      { type: 'somethingNew' },
    ] as StageSpec[]) {
      expect(planStages([spec])[0]!.requests).toBeGreaterThanOrEqual(1);
    }
  });
});

describe('stageAt', () => {
  it('maps a chevron back to the stage that owns it', () => {
    const stages = planStages(ENGINE_DEFAULT);
    expect(stageAt(stages, 0)!.step).toBe(1);
    expect(stageAt(stages, 2)!.step).toBe(2);
    expect(stageAt(stages, 23)!.step).toBe(4);
    expect(stageAt(stages, 31)!.step).toBe(4);
    expect(stageAt(stages, 32)!.step).toBe(5);
    expect(stageAt(stages, 99)).toBeUndefined();
  });
});

describe('renderChevrons', () => {
  it('draws one chevron per request and fills the completed ones', () => {
    const stages = planStages(ENGINE_DEFAULT);
    const html = renderChevrons(stages, 5);
    expect((html.match(/data-step=/g) ?? []).length).toBe(totalRequests(stages));
    expect((html.match(/is-done/g) ?? []).length).toBe(5);
  });

  it('reports progress to assistive technology, and never past the end', () => {
    const stages = planStages(ENGINE_DEFAULT);
    const html = renderChevrons(stages, 999);
    expect(html).toContain('aria-valuemax="34"');
    expect(html).toContain('aria-valuenow="34"');
  });

  it('draws nothing for an empty profile', () => {
    expect(renderChevrons([], 0)).toBe('');
  });
});

describe('formatPayload', () => {
  it('uses the units the reference uses', () => {
    expect(formatPayload(1e5)).toBe('100 kB');
    expect(formatPayload(1e6)).toBe('1.0 MB');
    expect(formatPayload(25e6)).toBe('25 MB');
    expect(formatPayload(512)).toBe('512 B');
  });
});
