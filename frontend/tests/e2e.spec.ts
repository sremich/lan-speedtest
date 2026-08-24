import { expect, test, type Page, type Request } from '@playwright/test';

/**
 * Tier 2 — drives a complete run of the real engine in a real browser.
 *
 * The suite is deliberately small and deliberately strict about one thing:
 * where packets go. A speed test that quietly reports to a third party is the
 * failure this project exists to avoid, and it is invisible in the UI.
 */

const PENDING = '—';

/**
 * True when a latency cell shows nothing usable.
 *
 * Three renderings mean the same thing — the browser could not resolve the
 * interval: `—` (no samples at all), `<0.1 ms` (below what we will print), and
 * `0.0 ms`. Anything that parses as a positive number is a real measurement.
 *
 * This exists because Firefox coarsens resource timing to ~1 ms (Chrome to
 * ~0.1 ms), so on a sub-millisecond path its readings collapse to zero or
 * vanish entirely. Which of the three you get varies run to run.
 */
function latencyIsUnmeasurable(text: string | null): boolean {
  if (text === null) return true;
  const t = text.trim();
  if (t === PENDING || t === '<0.1 ms') return true;
  const value = Number.parseFloat(t);
  return Number.isNaN(value) || value <= 0.05;
}

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

test('a full run completes and reports every headline metric', async ({
  page,
  browserName,
}) => {
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

  // Loaded latency is the metric LibreSpeed could not give us and the reason
  // this project exists — but over loopback it is often below what any browser
  // can resolve, Chrome included. So assert the cells render a legitimate
  // value rather than a specific magnitude; whether the value is usable is
  // what the AIM test below reasons about. The number itself only becomes
  // meaningful on a real path, which is a tier-3/4 question.
  for (const id of ['down-loaded', 'up-loaded']) {
    const value = await page.getByTestId(id).textContent();
    expect(value, `${id} should render something`).not.toBeNull();
    expect(
      value!.trim() === PENDING || /^(<0\.1|\d+\.\d+) ms$/.test(value!.trim()),
      `${browserName}: unexpected ${id} rendering: ${value}`,
    ).toBe(true);
  }
});

test('AIM ratings are derived, or the run explains why it could not score', async ({
  page,
  browserName,
}) => {
  // Both outcomes are legitimate and the suitability panel must never be blank.
  //
  // Every AIM experience needs `loadedLatencyIncrease`, which the engine only
  // computes when loaded latency is a *truthy* number. Firefox coarsens
  // resource timing to ~1 ms (Chrome to ~0.1 ms), so on a sub-millisecond path
  // every latency reading rounds to exactly 0 and the engine returns no scores
  // at all. That is the engine's behaviour, not ours — it cannot be fixed
  // without forking, which is a non-goal — so what we guarantee is that the UI
  // says so rather than showing an empty panel.
  await page.goto('/');
  await waitForCompletion(page);

  const cardCount = await page.locator('.aim__card').count();

  if (cardCount > 0) {
    expect(cardCount, 'a scored run must produce exactly three experiences').toBe(3);
    const ratings = await page.locator('.aim__rating').allTextContents();
    expect(ratings).toHaveLength(3);
    for (const rating of ratings) {
      expect(['bad', 'poor', 'average', 'good', 'great']).toContain(rating.trim());
    }
    return;
  }

  // No scores. The panel must explain itself...
  await expect(
    page.getByTestId('aim'),
    `${browserName}: no ratings and no explanation — the panel is simply empty`,
  ).toContainText('No rating available');

  // ...and the reason must be the one we understand. The engine drops all
  // scores when loaded latency is not a truthy number, so at least one of the
  // two must be unmeasurable. If both were real values and the scores are
  // still missing, something else is wrong and this must fail rather than wave
  // it through.
  const downLoaded = await page.getByTestId('down-loaded').textContent();
  const upLoaded = await page.getByTestId('up-loaded').textContent();
  expect(
    latencyIsUnmeasurable(downLoaded) || latencyIsUnmeasurable(upLoaded),
    `${browserName}: scores are missing but loaded latency was measurable ` +
      `(down ${downLoaded}, up ${upLoaded}) — the cause is not timing resolution`,
  ).toBe(true);
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

test('the detail view shows a distribution per measurement', async ({ page }) => {
  // The headline figures are single percentiles, which say nothing about
  // consistency. This is the view that does.
  await page.goto('/');
  await waitForCompletion(page);

  const detail = page.getByTestId('detail');
  await detail.locator('summary').click();

  const body = page.getByTestId('detail-body');
  await expect(body).toBeVisible();

  // At least download and latency should have produced samples.
  const groups = body.locator('.box__group');
  expect(await groups.count()).toBeGreaterThanOrEqual(2);

  const titles = await body.locator('.box__group-title').allTextContents();
  expect(titles.some((t) => /download/i.test(t))).toBe(true);
  expect(titles.some((t) => /latency/i.test(t))).toBe(true);

  // Each group draws a box, a median line and whiskers per row.
  expect(await body.locator('rect.box__box').count()).toBeGreaterThanOrEqual(2);
  expect(await body.locator('line.box__median').count()).toBeGreaterThanOrEqual(2);
  expect(await body.locator('line.box__whisker').count()).toBeGreaterThanOrEqual(2);
  expect(await body.locator('circle.box__mean').count()).toBeGreaterThanOrEqual(2);

  // And the legend explains the marks, so the plot is readable without docs.
  await expect(body.locator('.box__legend')).toBeVisible();

  // Every row carries a tooltip with the five-number summary.
  const firstTitle = await body.locator('g.box__row title').first().textContent();
  for (const key of ['min', 'p25', 'median', 'mean', 'p75', 'max']) {
    expect(firstTitle, `tooltip should include ${key}`).toContain(key);
  }
});

test('box geometry is ordered and inside the plot', async ({ page }) => {
  // A box drawn with p75 left of p25, or a median outside its own box, would
  // look plausible and be wrong. Check the geometry rather than trusting it.
  await page.goto('/');
  await waitForCompletion(page);
  await page.getByTestId('detail').locator('summary').click();

  const rows = page.locator('g.box__row');
  const count = await rows.count();
  expect(count).toBeGreaterThan(0);

  for (let i = 0; i < count; i += 1) {
    const row = rows.nth(i);
    const box = row.locator('rect.box__box');
    const median = row.locator('line.box__median');

    const bx = Number(await box.getAttribute('x'));
    const bw = Number(await box.getAttribute('width'));
    const mx = Number(await median.getAttribute('x1'));

    expect(bw, `row ${i}: box has no width`).toBeGreaterThan(0);
    expect(mx, `row ${i}: median ${mx} is left of the box at ${bx}`).toBeGreaterThanOrEqual(bx - 0.6);
    expect(mx, `row ${i}: median ${mx} is right of the box end ${bx + bw}`).toBeLessThanOrEqual(
      bx + bw + 0.6,
    );

    // Whiskers must span the box. The tolerance is the minimum drawn box
    // width: a perfectly consistent set has p25 === p75 === min === max, so
    // the box is widened to stay visible and legitimately overhangs a
    // zero-length whisker by half of that.
    const MIN_BOX_W = 2;
    const whisker = row.locator('line.box__whisker');
    const w1 = Number(await whisker.getAttribute('x1'));
    const w2 = Number(await whisker.getAttribute('x2'));
    expect(w1, `row ${i}: whisker starts right of the box`).toBeLessThanOrEqual(
      bx + MIN_BOX_W,
    );
    expect(w2, `row ${i}: whisker ends left of the box`).toBeGreaterThanOrEqual(
      bx + bw - MIN_BOX_W,
    );
  }
});

test('raw throughput is measured separately and labelled as a different number', async ({
  page,
}) => {
  await page.goto('/');
  await waitForCompletion(page);

  const raw = page.getByTestId('raw');
  await raw.locator('summary').click();
  // The explanation must be present: two numbers that are not comparable need
  // saying so, or someone will compare them.
  await expect(raw).toContainText(/different things/i);

  await page.getByTestId('raw-run').click();

  await expect
    .poll(async () => page.locator('body').getAttribute('data-raw-state'), { timeout: 90_000 })
    .toBe('done');

  const result = page.getByTestId('raw-result');
  await expect(result).toBeVisible();
  await expect(result).toContainText(/bps/);
  // It reports how it got there, not just a bare figure.
  await expect(result).toContainText(/streams/);
});

test('the raw harness stays on this origin', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  const foreign: string[] = [];
  page.on('request', (req) => {
    const url = req.url();
    if (url.startsWith('data:') || url.startsWith('blob:')) return;
    if (!url.startsWith(origin)) foreign.push(`${req.method()} ${url}`);
  });

  await page.goto('/');
  await waitForCompletion(page);
  await page.getByTestId('raw').locator('summary').click();
  await page.getByTestId('raw-run').click();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-raw-state'), { timeout: 90_000 })
    .toBe('done');

  expect(foreign, `requests left the origin:\n${foreign.join('\n')}`).toEqual([]);
});
