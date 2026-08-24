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
  version: string;
  gitSha: string;
  profile: string;
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

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path, { cache: 'no-store' });
  if (!res.ok) throw new Error(`${path} responded ${res.status}`);
  return (await res.json()) as T;
}

export const fetchStatus = (): Promise<Status> => getJson<Status>('/api/status');
export const fetchProfile = (): Promise<ProfileResponse> =>
  getJson<ProfileResponse>('/api/profile');

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
