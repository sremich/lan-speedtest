/**
 * The step-by-step progress strip.
 *
 * speed.cloudflare.com draws one chevron per *request* the profile will issue,
 * grouped and coloured by the stage it belongs to, filling in as the run
 * proceeds. It is a far better progress indicator than a bar, because it shows
 * the shape of the work up front: you can see that a run is two thirds
 * latency pings, or that the big downloads have not started yet.
 *
 * Confirmed against the engine's own default profile, which has 25 stages
 * whose fourth entry is `{ type: 'download', bytes: 1e5, count: 9 }` — the
 * "Payload: 100 kB, Requests: 9, Step: 4 of 25" the reference shows on hover.
 */

/** The subset of a measurement entry that decides how it is drawn. */
export interface StageSpec {
  type: string;
  bytes?: number;
  count?: number;
  numPackets?: number;
  /** Marks a warm-up round, whose samples the engine deliberately discards. */
  bypassMinDuration?: boolean;
}

export interface Stage {
  /** 1-based position in the profile, for "Step 4 of 25". */
  step: number;
  type: string;
  label: string;
  bytes?: number;
  /** How many chevrons this stage occupies. */
  requests: number;
  /** Index of this stage's first chevron in the whole strip. */
  offset: number;
  /** Alternating 0/1 so consecutive stages of one type stay distinguishable. */
  shade: number;
  /**
   * A warm-up round.
   *
   * The engine exempts these from its minimum-duration rule and then leaves
   * them out of the bandwidth points, so they contribute no measurement. The
   * strip says so rather than showing a step that mysteriously measured
   * nothing.
   */
  warmUp: boolean;
}

const LABELS: Record<string, string> = {
  latency: 'Latency',
  download: 'Download',
  upload: 'Upload',
  packetLoss: 'Packet loss',
  packetLossUnderLoad: 'Packet loss under load',
};

/**
 * Chevrons given to a packet-loss stage.
 *
 * It has no per-request progress we can observe — the engine reports one
 * figure at the end — but it takes seconds, so drawing it as a single chevron
 * would make the strip look stalled. A fixed block is honest about that: it
 * fills all at once when the stage completes.
 */
const PACKET_LOSS_CHEVRONS = 6;

/** How many chevrons a stage is worth. */
function requestsFor(spec: StageSpec): number {
  // Shipped profiles always state these explicitly; the fallback only matters
  // for a hand-edited profile that leans on an engine default, where the
  // strip may under-count rather than mislead about what is running.
  switch (spec.type) {
    case 'latency':
      return Math.max(1, spec.numPackets ?? 1);
    case 'download':
    case 'upload':
      return Math.max(1, spec.count ?? 1);
    case 'packetLoss':
    case 'packetLossUnderLoad':
      return PACKET_LOSS_CHEVRONS;
    default:
      return 1;
  }
}

/** Expands a profile's measurement list into positioned stages. */
export function planStages(specs: readonly StageSpec[]): Stage[] {
  const seenPerType = new Map<string, number>();
  let offset = 0;

  return specs.map((spec, i) => {
    const seen = seenPerType.get(spec.type) ?? 0;
    seenPerType.set(spec.type, seen + 1);

    const requests = requestsFor(spec);
    const warmUp = spec.bypassMinDuration === true;
    const base = LABELS[spec.type] ?? spec.type;
    const stage: Stage = {
      step: i + 1,
      type: spec.type,
      label: warmUp ? `${base} (warm-up)` : base,
      requests,
      offset,
      shade: seen % 2,
      warmUp,
      ...(spec.bytes !== undefined ? { bytes: spec.bytes } : {}),
    };
    offset += requests;
    return stage;
  });
}

/** Total chevrons across every stage. */
export function totalRequests(stages: readonly Stage[]): number {
  return stages.reduce((sum, s) => sum + s.requests, 0);
}

/** The stage a given chevron belongs to, or undefined if out of range. */
export function stageAt(stages: readonly Stage[], chevron: number): Stage | undefined {
  return stages.find((s) => chevron >= s.offset && chevron < s.offset + s.requests);
}

/**
 * Renders the strip.
 *
 * `done` is how many requests have completed; everything before it is filled.
 * Kept as markup rather than DOM mutation so the whole strip is one
 * assignment — there are only a few dozen elements and it re-renders at most
 * once per results change.
 */
export function renderChevrons(stages: readonly Stage[], done: number): string {
  if (stages.length === 0) return '';

  const total = totalRequests(stages);
  const cells: string[] = [];

  for (const stage of stages) {
    for (let i = 0; i < stage.requests; i += 1) {
      const index = stage.offset + i;
      const state = index < done ? ' is-done' : '';
      cells.push(
        `<span class="chev chev--${stage.type}${state}" data-step="${stage.step}" data-shade="${stage.shade}"></span>`,
      );
    }
  }

  return `<div class="chevrons" data-testid="chevrons" role="progressbar"
       aria-label="Test progress" aria-valuemin="0" aria-valuemax="${total}" aria-valuenow="${Math.min(
         done,
         total,
       )}">${cells.join('')}</div>`;
}

/** Formats a payload size for a tooltip, in the units the reference uses. */
export function formatPayload(bytes: number): string {
  if (bytes >= 1e6) {
    const mb = bytes / 1e6;
    return `${mb >= 10 ? mb.toFixed(0) : mb.toFixed(1)} MB`;
  }
  if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(0)} kB`;
  return `${bytes} B`;
}
