import { expect, test, type Page } from '@playwright/test';

/**
 * Tier 2 — a completed run reaches storage and shows up on the history page.
 *
 * Runs only when the backend was started with history enabled; the main e2e
 * suite runs without it, so these skip rather than fail there.
 */

const enabled = !!process.env.SPEEDTEST_E2E_HISTORY;

test.skip(!enabled, 'needs a backend started with SPEEDTEST_HISTORY_DB set');

async function completeARun(page: Page): Promise<void> {
  await page.goto('/');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-test-state'), { timeout: 100_000 })
    .not.toBe('running');

  const error = page.getByTestId('error');
  if (await error.isVisible()) {
    throw new Error(`front end reported: ${await error.textContent()}`);
  }
  await expect(page.locator('body')).toHaveAttribute('data-test-state', 'complete');

  // The POST fires after onFinish; wait for the front end to report the outcome.
  await expect
    .poll(async () => page.locator('body').getAttribute('data-result-stored'), { timeout: 20_000 })
    .toBe('yes');
}

test('a finished run is offered a history link and is stored', async ({ page }) => {
  await page.goto('/');
  // The link only appears when the backend actually keeps results.
  await expect(page.getByTestId('history-link')).toBeVisible();

  await completeARun(page);
});

test('the history page lists stored runs and draws a trend', async ({ page, request }) => {
  // Two runs, so the chart has something to draw a line between.
  await completeARun(page);
  await completeARun(page);

  const stored = await (await request.get('/api/history')).json();
  expect(stored.length).toBeGreaterThanOrEqual(2);

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  await expect(page.getByTestId('error')).toBeHidden();
  await expect(page.getByTestId('empty')).toBeHidden();

  const rows = page.locator('#rows tr');
  expect(await rows.count()).toBeGreaterThanOrEqual(2);

  // Each row carries a real bandwidth figure, not a placeholder.
  const firstRow = rows.first();
  await expect(firstRow).toContainText(/bps/);

  // And the chart rendered as an actual plot rather than the "need more runs"
  // note, with one path per direction.
  const chart = page.getByTestId('chart');
  await expect(chart.locator('svg')).toBeVisible();
  expect(await chart.locator('path.chart__line').count()).toBe(2);
  expect(await chart.locator('circle.chart__dot').count()).toBeGreaterThanOrEqual(4);
});

test('the client filter narrows the list', async ({ page, request }) => {
  await completeARun(page);

  const clients = await (await request.get('/api/clients')).json();
  expect(clients.length).toBeGreaterThanOrEqual(1);

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  const filter = page.getByTestId('client-filter');
  // "All clients" plus one option per known client.
  expect(await filter.locator('option').count()).toBeGreaterThanOrEqual(2);

  await filter.selectOption(clients[0].clientIp);
  await expect
    .poll(async () => page.locator('#rows tr').count(), { timeout: 20_000 })
    .toBeGreaterThanOrEqual(1);

  await expect(page.getByTestId('error')).toBeHidden();
});

test('storing results does not send anything off-origin', async ({ page, baseURL }) => {
  // The result POST is new traffic, and it must stay local like everything else.
  const origin = new URL(baseURL!).origin;
  const foreign: string[] = [];
  page.on('request', (req) => {
    const url = req.url();
    if (url.startsWith('data:') || url.startsWith('blob:')) return;
    if (!url.startsWith(origin)) foreign.push(`${req.method()} ${url}`);
  });

  await completeARun(page);
  await page.goto('/history.html');
  await page.waitForTimeout(2000);

  expect(foreign, `requests left the origin:\n${foreign.join('\n')}`).toEqual([]);
});

test('chart axis labels are not clipped by the gutter', async ({ page }) => {
  // Regression guard. With a narrower gutter, "250 Mbps" rendered as
  // "50 Mbps" — the chart appeared an order of magnitude slower than it was,
  // silently and with no error anywhere.
  await completeARun(page);
  await completeARun(page);

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  const svg = page.locator('.chart__svg');
  await expect(svg).toBeVisible();

  const svgBox = await svg.boundingBox();
  expect(svgBox).not.toBeNull();

  const ticks = page.locator('.chart__tick');
  const count = await ticks.count();
  expect(count).toBeGreaterThan(0);

  for (let i = 0; i < count; i += 1) {
    const tick = ticks.nth(i);
    const text = (await tick.textContent())?.trim() ?? '';
    if (text === '' || text === 'oldest' || text === 'newest') continue;

    const box = await tick.boundingBox();
    expect(box, `tick "${text}" has no box`).not.toBeNull();
    // Every label must sit fully inside the drawing surface.
    expect(
      box!.x,
      `tick "${text}" starts at ${box!.x}, left of the chart edge ${svgBox!.x} — it is clipped`,
    ).toBeGreaterThanOrEqual(svgBox!.x - 0.5);
    expect(box!.x + box!.width).toBeLessThanOrEqual(svgBox!.x + svgBox!.width + 0.5);
  }
});

test('a finished run can be linked to, and the link redraws it', async ({ page }) => {
  // Item 6 of the request: the history page could start a new test but not
  // take you back to a result. A permalink is only worth having if it shows
  // the run rather than a summary of it, so this asserts the traces and the
  // distributions are there too.
  await completeARun(page);

  const link = page.getByTestId('permalink');
  await expect(link).toBeVisible();

  const headline = (await page.getByTestId('download').textContent())?.trim();
  const href = await link.getAttribute('href');
  expect(href).toMatch(/^\/result\.html\?id=\d+$/);

  await link.click();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-result-state'), { timeout: 20_000 })
    .toBe('loaded');

  await expect(page.getByTestId('error')).toBeHidden();
  expect(
    (await page.getByTestId('download').textContent())?.trim(),
    'the stored run should read as the run that was just watched',
  ).toBe(headline);

  // Every sample was stored, so the traces and the distributions redraw.
  await expect(page.locator('#download-chart svg')).toBeVisible();
  expect(await page.locator('#detail-body .box__group').count()).toBeGreaterThan(0);

  // And it says whose run it was and when, which a live page cannot.
  await expect(page.getByTestId('measured-at')).toContainText(/Measured/);
  await expect(page.getByTestId('profile')).toContainText('profile:');
});

test('a bad result link fails visibly rather than showing an empty run', async ({ page }) => {
  await page.goto('/result.html?id=999999');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-result-state'), { timeout: 20_000 })
    .toBe('error');
  await expect(page.getByTestId('error')).toBeVisible();
});

test('a history row opens the run it describes', async ({ page }) => {
  await completeARun(page);

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  const row = page.locator('#rows tr').first();
  const shown = (await row.locator('td').nth(2).textContent())?.trim();

  await row.getByTestId('open-result').click();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-result-state'), { timeout: 20_000 })
    .toBe('loaded');

  // The row and the page it opens are the same run, so the same figure.
  const value = (await page.getByTestId('download').textContent())?.trim();
  const unit = (await page.locator('#download-unit').textContent())?.trim();
  expect(shown?.replace(/\s+/g, ' ')).toBe(`${value} ${unit}`);
});

test('a client can be given a name, and the name sticks', async ({ page, request }) => {
  // "Is there a way we can resolve the hostname for the clients?" — reverse
  // DNS answers that when there is a PTR record, and a typed name answers it
  // when there is not. The typed name always wins.
  await completeARun(page);

  const clients = await (await request.get('/api/clients')).json();
  const ip = clients[0].clientIp as string;

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  // The control appears only once a specific client is selected: the name
  // belongs to a client, not to "all clients".
  await expect(page.getByTestId('rename')).toBeHidden();
  await page.getByTestId('client-filter').selectOption(ip);
  await expect(page.getByTestId('rename')).toBeVisible();

  page.once('dialog', (d) => void d.accept('Study desktop'));
  await page.getByTestId('rename').click();

  await expect(page.locator('#rows tr').first()).toContainText('Study desktop', {
    timeout: 20_000,
  });

  // The address is still recoverable, because a label that replaced it would
  // make the history impossible to correlate with anything else.
  await expect(page.locator('#rows tr').first().locator('.cell--client')).toHaveAttribute(
    'title',
    new RegExp(ip.replace(/\./g, '\.')),
  );

  // And it survives a reload, because it is stored rather than remembered.
  await page.reload();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');
  await expect(page.locator('#rows tr').first()).toContainText('Study desktop');

  // Clearing it falls back to the address rather than leaving a blank cell.
  await page.getByTestId('client-filter').selectOption(ip);
  page.once('dialog', (d) => void d.accept(''));
  await page.getByTestId('rename').click();
  await expect(page.locator('#rows tr').first()).toContainText(ip, { timeout: 20_000 });
});
