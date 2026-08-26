/**
 * Backend API types and fetchers.
 *
 * Everything the engine is configured with comes from the server, so the
 * settings that keep a test on the LAN are decided in one place rather than
 * being baked into this bundle. `assertLanOnly` re-checks them anyway — the
 * cost of a mistake here is silently reporting results to Cloudflare.
 */

import type { MeasurementConfig } from '@cloudflare/speedtest';

export interface Status {
  /** What this deployment calls itself: the heading and the tab title. */
  siteName: string;
  version: string;
  gitSha: string;
  profile: string;
  historyEnabled: boolean;
  /** The requesting client, as seen from the connection. */
  clientIp: string;
  /**
   * What sort of address that is: `private`, `cgnat`, `public`, `loopback` or
   * `linkLocal`.
   *
   * Worth saying out loud, because "why is it always 10.x?" has a different
   * answer depending on which of these it is — and the honest answer to some
   * of them is that the real address cannot be recovered. See
   * `https://github.com/sremich/lan-speedtest/wiki/Client-Identity`.
   */
  clientKind: string;
  clientKindLabel: string;
  serverProfileDescription: string;
  /**
   * Whether this deployment measures as soon as the page loads.
   *
   * The server's default for a first visit. A browser that has chosen
   * otherwise keeps its own answer.
   */
  autostart: boolean;
}

export interface EngineConfig {
  downloadApiUrl: string;
  uploadApiUrl: string;
  logAimApiUrl: null;
  logMeasurementApiUrl: null;
  autoStart: boolean;
  measurements: MeasurementConfig[];
  estimatedServerTime: number;
  measureDownloadLoadedLatency: boolean;
  measureUploadLoadedLatency: boolean;
  turnServerUri?: string;
  turnServerUser?: string;
  turnServerPass?: string;
}

export interface ProfileResponse {
  profile: string;
  description: string;
  packetLossEnabled: boolean;
  engineConfig: EngineConfig;
}

/** One entry the profile picker can offer. */
export interface ProfileSummary {
  name: string;
  description: string;
  /** The link speed this profile's transfer sizes were chosen for. */
  nominalBps: number | null;
  /** Whether automatic selection may choose it. */
  autoSelectable: boolean;
}

export interface ProfilesResponse {
  /** The server's configured profile, used when the client has no preference. */
  default: string;
  profiles: ProfileSummary[];
}

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path, { cache: 'no-store' });
  if (!res.ok) throw new Error(`${path} responded ${res.status}`);
  return (await res.json()) as T;
}

export const fetchStatus = (): Promise<Status> => getJson<Status>('/api/status');

/**
 * Fetches an engine configuration.
 *
 * A name asks for a specific profile; the server refuses one it does not know
 * rather than quietly serving its default, so a stale choice surfaces as an
 * error instead of measuring something other than what it claims to.
 */
export const fetchProfile = (name?: string): Promise<ProfileResponse> =>
  getJson<ProfileResponse>(
    name ? `/api/profile?name=${encodeURIComponent(name)}` : '/api/profile',
  );

export const fetchProfiles = (): Promise<ProfilesResponse> =>
  getJson<ProfilesResponse>('/api/profiles');

/**
 * Refuses to start a test that could reach off the LAN.
 *
 * The engine defaults `logAimApiUrl` to a Cloudflare endpoint and posts every
 * completed result to it, and falls back to fetching TURN credentials from
 * another one whenever `turnServerUser`/`turnServerPass` are not both set.
 * Both are server-controlled, but a misconfigured deploy should fail loudly
 * here rather than quietly phone home for months.
 */
export function assertLanOnly(cfg: EngineConfig): void {
  const problems: string[] = [];

  if (cfg.logAimApiUrl !== null) {
    problems.push('logAimApiUrl is set — results would be reported externally');
  }
  if (cfg.logMeasurementApiUrl !== null) {
    problems.push('logMeasurementApiUrl is set — measurements would be reported externally');
  }

  const absolute = (u: string) => /^[a-z]+:\/\//i.test(u);
  if (absolute(cfg.downloadApiUrl)) problems.push(`downloadApiUrl is absolute: ${cfg.downloadApiUrl}`);
  if (absolute(cfg.uploadApiUrl)) problems.push(`uploadApiUrl is absolute: ${cfg.uploadApiUrl}`);

  const wantsPacketLoss = cfg.measurements.some((m) => m.type.startsWith('packetLoss'));
  if (wantsPacketLoss && !(cfg.turnServerUri && cfg.turnServerUser && cfg.turnServerPass)) {
    problems.push(
      'packet-loss stage is enabled without a complete TURN uri/user/pass — ' +
        'the engine would fetch credentials from Cloudflare',
    );
  }

  // Reachability probes target external hosts by design; they must never be
  // in a profile we ship.
  const external = cfg.measurements
    .map((m) => m.type)
    .filter((t) => ['v4Reachability', 'v6Reachability', 'rpki', 'nxdomain'].includes(t));
  if (external.length > 0) {
    problems.push(`profile contains external probe stages: ${external.join(', ')}`);
  }

  if (problems.length > 0) {
    throw new Error(`Refusing to start — configuration would leave the LAN:\n- ${problems.join('\n- ')}`);
  }
}

/**
 * Stores a completed run.
 *
 * Deliberately best-effort: a failure here must never surface as a failed
 * speed test, because the measurement itself succeeded. The backend also
 * accepts and ignores submissions when history is turned off, so the front end
 * does not need to know whether this deployment keeps results.
 */
export async function submitResult(payload: {
  summary: Record<string, unknown>;
  scores: Record<string, string>;
  profile: string;
  /** Every sample, so the stored run can be redrawn rather than summarised. */
  points: Record<string, unknown>;
}): Promise<number | undefined> {
  try {
    const res = await fetch('/api/results', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) return undefined;
    const created = (await res.json()) as { id?: number };
    return typeof created.id === 'number' ? created.id : undefined;
  } catch {
    return undefined;
  }
}

/** A stored run, in full. */
export interface StoredRunDetail {
  id: number;
  recordedAt: string;
  clientIp: string;
  clientName: string | null;
  hostname: string | null;
  userAgent: string;
  profile: string;
  download: number | null;
  upload: number | null;
  latency: number | null;
  jitter: number | null;
  downLoadedLatency: number | null;
  upLoadedLatency: number | null;
  packetLoss: number | null;
  totalDurationMs: number | null;
  scores: Record<string, string>;
  /** A note written by hand after the run. */
  note: string | null;
  /** The build that measured this run. `null` for runs stored before 1.5.0. */
  appVersion: string | null;
  summary: Record<string, number | undefined>;
  points: Record<string, unknown>;
}

export const fetchResult = (id: number): Promise<StoredRunDetail> =>
  getJson<StoredRunDetail>(`/api/results/${encodeURIComponent(String(id))}`);

/**
 * Names a client, or clears the name when given an empty string.
 *
 * Best-effort in the same way results are: failing to rename a row is not a
 * reason to fail the page it is on.
 */
export async function setRunNote(id: number, note: string): Promise<boolean> {
  try {
    const res = await fetch(`/api/results/${encodeURIComponent(String(id))}/note`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ note }),
    });
    return res.ok;
  } catch {
    return false;
  }
}

export async function setClientName(ip: string, name: string): Promise<boolean> {
  try {
    const res = await fetch(`/api/clients/${encodeURIComponent(ip)}/name`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name }),
    });
    return res.ok;
  } catch {
    return false;
  }
}
