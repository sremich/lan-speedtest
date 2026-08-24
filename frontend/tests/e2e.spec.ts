import { expect, test, type Page, type Request } from '@playwright/test';

/**
 * Tier 2 — drives a complete run of the real engine in a real browser.
 *
 * The suite is deliberately small and deliberately strict about one thing:
 * where packets go. A speed test that quietly reports to a third party is the
 * failure this project exists to avoid, and it is invisible in the UI.
 */

const PENDING = '—';

/** Waits for the run to reach a terminal state, surfacing the UI error if any. */
async function waitForCompletion(page: Page): Promise<void> {
  await expect
    .poll(async () => page.locator('body').getAttribute('data-test-state'), {
      timeout: 100_000,
      message: 'test run did not reach a terminal state',
    })
    .not.toBe('running');

  const error = page.getByTestId('error');
  if (await error.isVisible()) {
    throw new Error(`front end reported: ${await error.textContent()}`);
  }
  await expect(page.locator('body')).toHaveAttribute('data-test-state', 'complete');
}

test('a full run completes and reports every headline metric', async ({ page }) => {
  await page.goto('/');

  // The brief asks for auto-start: no interaction should be required.
  await expect(page.getByTestId('phase')).not.toHaveText('Starting…', { timeout: 30_000 });

  await waitForCompletion(page);

  await expect(page.getByTestId('phase')).toHaveText('Complete');

  for (const id of ['download', 'upload', 'latency', 'jitter']) {
    await expect(page.getByTestId(id), `${id} should have a value`).not.toHaveText(PENDING);
  }

  // Bandwidth should be a real number with a unit beside it.
  const download = await page.getByTestId('download').textContent();
  expect(Number(download)).toBeGreaterThan(0);
  await expect(page.locator('#download-unit')).toHaveText(/bps$/);

  // Loaded latency is the metric LibreSpeed could not give us; it is the
  // reason this project exists, so assert it actually arrived.
  await expect(page.getByTestId('down-loaded')).not.toHaveText(PENDING);
  await expect(page.getByTestId('up-loaded')).not.toHaveText(PENDING);
});

test('AIM ratings are derived and rendered', async ({ page }) => {
  await page.goto('/');
  await waitForCompletion(page);

  const cards = page.locator('.aim__card');
  await expect(cards).toHaveCount(3);

  const ratings = await page.locator('.aim__rating').allTextContents();
  expect(ratings).toHaveLength(3);
  for (const rating of ratings) {
    expect(['bad', 'poor', 'average', 'good', 'great']).toContain(rating.trim());
  }
});

test('the build version and profile are visible on the page', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByTestId('build')).toHaveText(/^v\d+\.\d+\.\d+ · \S+$/);
  await expect(page.getByTestId('profile')).toHaveText(/^profile: quick/);
});

test('nothing leaves this origin for the whole run', async ({ page, baseURL }) => {
  // The headline non-negotiable. The engine defaults to POSTing completed
  // results to speed.cloudflare.com and to fetching TURN credentials from
  // Cloudflare; both are disabled server-side, and this proves it end to end.
  const origin = new URL(baseURL!).origin;
  const foreign: string[] = [];

  const record = (req: Request) => {
    const url = req.url();
    if (url.startsWith('data:') || url.startsWith('blob:')) return;
    if (!url.startsWith(origin)) foreign.push(`${req.method()} ${url}`);
  };

  page.on('request', record);

  await page.goto('/');
  await waitForCompletion(page);

  // The results POST fires after onFinish, so give it room to misbehave.
  await page.waitForTimeout(3000);

  expect(foreign, `requests left the origin:\n${foreign.join('\n')}`).toEqual([]);
});

test('the measurement endpoints are same-origin and uncacheable', async ({ page, baseURL }) => {
  const measurementRequests: string[] = [];
  page.on('request', (req) => {
    const path = new URL(req.url()).pathname;
    if (path === '/__down' || path === '/__up') measurementRequests.push(path);
  });

  await page.goto('/');
  await waitForCompletion(page);

  expect(measurementRequests.length).toBeGreaterThan(5);
  expect(measurementRequests).toContain('/__down');
  expect(measurementRequests).toContain('/__up');

  // Re-check the header the engine relies on, from the browser's perspective.
  const res = await page.request.get(`${baseURL}/__down?bytes=1024`);
  expect(res.headers()['server-timing']).toMatch(/^cfRequestDuration;dur=[0-9.]+$/);
  expect(res.headers()['cache-control']).toContain('no-store');
});
