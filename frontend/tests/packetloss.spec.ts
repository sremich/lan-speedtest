import { expect, test } from '@playwright/test';

/**
 * Tier 2 — the packet-loss stage against a real TURN relay.
 *
 * Split from the main suite because it needs coturn running
 * (`docker compose -f docker-compose.e2e.yml up -d`) and the backend started
 * with the `e2e-packetloss` profile and matching TURN credentials. The runner
 * script and the CI job set `SPEEDTEST_E2E_PACKETLOSS=1`; without it these
 * are skipped rather than failing on an absent relay.
 */

const enabled = !!process.env.SPEEDTEST_E2E_PACKETLOSS;

test.skip(!enabled, 'needs coturn — see docker-compose.e2e.yml');

test.describe('packet loss via TURN', () => {
  test('the relay is offered to the browser with complete credentials', async ({ request }) => {
    const res = await request.get('/api/profile');
    expect(res.ok()).toBeTruthy();
    const body = await res.json();

    expect(body.packetLossEnabled).toBe(true);

    const cfg = body.engineConfig;
    // The engine builds `turn:{uri}?transport=udp`, so the URI must be a bare
    // host:port with no scheme.
    expect(cfg.turnServerUri).toMatch(/^[^:/]+:\d+$/);
    // Both must be present. With either missing the engine silently falls back
    // to fetching credentials from Cloudflare.
    expect(cfg.turnServerUser).toBeTruthy();
    expect(cfg.turnServerPass).toBeTruthy();

    const kinds = cfg.measurements.map((m: { type: string }) => m.type);
    expect(kinds).toContain('packetLoss');
  });

  test('a relay candidate can be gathered from the configured server', async ({ page, request }) => {
    // The Trickle-ICE check, automated: if no `relay` candidate appears, the
    // packet-loss stage cannot work and the reported 0% would be meaningless.
    const cfg = (await (await request.get('/api/profile')).json()).engineConfig;
    await page.goto('/');

    const candidates: string[] = await page.evaluate(
      async ({ uri, username, credential }) => {
        const pc = new RTCPeerConnection({
          iceServers: [{ urls: `turn:${uri}?transport=udp`, username, credential }],
          iceTransportPolicy: 'relay',
        });
        pc.createDataChannel('probe');
        const found: string[] = [];
        const done = new Promise<void>((resolve) => {
          pc.onicecandidate = (e) => {
            if (!e.candidate) return resolve();
            found.push(e.candidate.candidate);
          };
          setTimeout(resolve, 12_000);
        });
        await pc.setLocalDescription(await pc.createOffer());
        await done;
        pc.close();
        return found;
      },
      {
        uri: cfg.turnServerUri as string,
        username: cfg.turnServerUser as string,
        credential: cfg.turnServerPass as string,
      },
    );

    const relayed = candidates.filter((c) => c.includes(' typ relay'));
    expect(
      relayed.length,
      `no relay candidate from ${cfg.turnServerUri}; gathered:\n${candidates.join('\n')}`,
    ).toBeGreaterThan(0);
  });

  test('a full run reports a packet-loss figure', async ({ page }) => {
    await page.goto('/');
    await expect
      .poll(async () => page.locator('body').getAttribute('data-test-state'), {
        timeout: 110_000,
      })
      .not.toBe('running');

    const error = page.getByTestId('error');
    if (await error.isVisible()) {
      throw new Error(`front end reported: ${await error.textContent()}`);
    }

    const loss = page.getByTestId('packet-loss');
    await expect(loss).not.toHaveText('—');
    // Loopback through a local relay should lose nothing; anything else means
    // the relay or the burst is misbehaving.
    await expect(loss).toHaveText(/^(0%|<0\.1%)$/);
  });

  test('the relay does not make the run reach off-origin', async ({ page, baseURL }) => {
    // TURN traffic is UDP and invisible to the request hook, but the HTTP side
    // must still stay put — in particular the engine must not have fetched
    // credentials from turnServerCredsApiUrl.
    const origin = new URL(baseURL!).origin;
    const foreign: string[] = [];
    page.on('request', (req) => {
      const url = req.url();
      if (url.startsWith('data:') || url.startsWith('blob:')) return;
      if (!url.startsWith(origin)) foreign.push(`${req.method()} ${url}`);
    });

    await page.goto('/');
    await expect
      .poll(async () => page.locator('body').getAttribute('data-test-state'), {
        timeout: 110_000,
      })
      .not.toBe('running');
    await page.waitForTimeout(3000);

    expect(foreign, `requests left the origin:\n${foreign.join('\n')}`).toEqual([]);
  });
});
