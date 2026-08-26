import { expect, test, type Page } from '@playwright/test';

/**
 * Tier 2 — a completed run reaches storage and shows up on the history page.
 *
 * Runs only when the backend was started with history enabled; the main e2e
 * suite runs without it, so these skip rather than fail there.
 */

const enabled = !!process.env.SPEEDTEST_E2E_HISTORY;

test.skip(!enabled, 'needs a backend started with SPEEDTEST_HISTORY_DB set');

/**
 * Waits for the run already under way to finish and reach storage.
 *
 * Separate from `completeARun` because a run that has to be configured first —
 * tagged with a location, say — is started by pressing the button rather than
 * by loading the page.
 */
async function awaitStoredRun(page: Page): Promise<void> {
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

async function completeARun(page: Page): Promise<void> {
  await page.goto('/');
  await awaitStoredRun(page);
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
  // By name, not by column index: this read `td` index 2 until a checkbox
  // column was added in front of it, at which point it silently compared the
  // client address with a bandwidth figure.
  const shown = (await row.getByTestId('row-download').textContent())?.trim();

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

test('a run can be given a description, from the history and the result page', async ({ page }) => {
  // Per-run, not per-client: "upstairs landing, laptop on battery" is exactly
  // what differs between two runs from the same machine.
  await completeARun(page);

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  const note = page.locator('#rows tr').first().getByTestId('note');
  await expect(note).toContainText(/add/i);

  page.once('dialog', (d) => void d.accept('upstairs landing, laptop on battery'));
  await note.click();
  await expect(note).toContainText('upstairs landing, laptop on battery', { timeout: 20_000 });

  // It is stored, not remembered, so it survives a reload.
  await page.reload();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');
  await expect(page.locator('#rows tr').first().getByTestId('note')).toContainText(
    'upstairs landing',
  );

  // And the same description shows on the run's own page, where it can also
  // be edited — that is the page a link lands on.
  await page.locator('#rows tr').first().getByTestId('open-result').click();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-result-state'), { timeout: 20_000 })
    .toBe('loaded');
  await expect(page.getByTestId('note')).toContainText('upstairs landing');

  page.once('dialog', (d) => void d.accept('rewritten from the result page'));
  await page.getByTestId('note').click();
  await expect(page.getByTestId('note')).toContainText('rewritten from the result page', {
    timeout: 20_000,
  });
});

test('a run can be tagged with a location, and the history filter narrows to it', async ({
  page,
  request,
}) => {
  // The tool is used by walking the house with a phone. A stack of runs that
  // does not say which room each was taken in cannot answer the only question
  // worth asking of it, so the room is chosen before the run rather than
  // reconstructed from memory afterwards.
  await page.goto('/?autostart=0');

  await expect(page.getByTestId('places')).toBeVisible();
  const chips = page.getByTestId('place-chips');

  // Untagged is the default: a run is never quietly attributed to wherever the
  // previous one happened to be taken.
  await expect(chips.locator('.chip[aria-pressed="true"]')).toHaveText('No location');

  // A field rather than a prompt — a modal dialog is a wall between you and
  // the one thing you came to the page to do.
  await page.getByTestId('place-add').click();
  const input = page.getByTestId('place-input');
  await expect(input).toBeFocused();
  await input.fill('Office');
  await input.press('Enter');

  await expect(chips.locator('.chip[aria-pressed="true"]')).toHaveText('Office');
  expect(
    await page.evaluate(() => localStorage.getItem('speedtest.location')),
    'the room should be remembered, so a walk is one tap per room',
  ).toBe('Office');

  await page.getByTestId('restart').click();
  await expect(page.locator('body')).toHaveAttribute('data-test-state', 'running');
  await awaitStoredRun(page);

  // And one from nowhere in particular, so the filter below has something to
  // exclude rather than only something to include.
  await chips.locator('.chip[data-place=""]').click();
  await page.getByTestId('restart').click();
  await expect(page.locator('body')).toHaveAttribute('data-test-state', 'running');
  await awaitStoredRun(page);

  const runs = await (await request.get('/api/history')).json();
  expect(runs.length).toBeGreaterThanOrEqual(2);
  expect(runs[0].location, 'the run just taken was deliberately untagged').toBeNull();
  expect(runs[1].location, 'the tag should have ridden along to storage').toBe('Office');

  // The filter is applied by the backend, so watch for it going out: narrowing
  // in the browser would leave the trend chart drawn from every run rather
  // than from the ones being asked about.
  const asked: string[] = [];
  page.on('request', (req) => {
    const url = new URL(req.url());
    if (url.pathname === '/api/history') asked.push(url.searchParams.get('location') ?? '');
  });

  await page.goto('/history.html');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-history-state'), { timeout: 20_000 })
    .toBe('loaded');

  // Every run wears its room, or an honest dash where it has none.
  await expect(page.locator('#rows tr').first().getByTestId('row-location')).toHaveText('—');

  const filter = page.getByTestId('location-filter');
  await expect(filter, 'the filter appears once there is anything to filter by').toBeVisible();
  await filter.selectOption('Office');

  await expect
    .poll(
      async () => {
        const rooms = await page.locator('#rows [data-testid="row-location"]').allTextContents();
        return rooms.length > 0 && rooms.every((t) => t.trim() === 'Office');
      },
      { timeout: 20_000, message: 'the table should narrow to the tagged runs' },
    )
    .toBe(true);

  expect(asked, 'the choice should reach the backend as ?location=').toContain('Office');
  await expect(page.getByTestId('error')).toBeHidden();

  // And the run's own page says where it was taken, next to the client and the
  // profile — the page a shared link lands on.
  await page.goto(`/result.html?id=${runs[1].id}`);
  await expect
    .poll(async () => page.locator('body').getAttribute('data-result-state'), { timeout: 20_000 })
    .toBe('loaded');
  await expect(page.getByTestId('result-location')).toHaveText('location: Office');
});

test('a location used before is offered again rather than retyped', async ({ page }) => {
  // The list is built from the runs themselves, so the second walk through the
  // house is a tap per room instead of a spelling exercise — and spelling is
  // the whole risk here, because the filter matches exactly.
  await page.goto('/?autostart=0');
  await expect(page.getByTestId('places')).toBeVisible();

  await page.getByTestId('place-add').click();
  await page.getByTestId('place-input').fill('Garage');
  await page.getByTestId('place-input').press('Enter');

  await page.getByTestId('restart').click();
  await expect(page.locator('body')).toHaveAttribute('data-test-state', 'running');
  await awaitStoredRun(page);

  await page.goto('/?autostart=0');
  const chips = page.getByTestId('place-chips');
  await expect(chips.locator('.chip', { hasText: 'Garage' })).toBeVisible();
  // Still the chosen one: the choice is remembered, not merely offered.
  await expect(chips.locator('.chip[aria-pressed="true"]')).toHaveText('Garage');

  // A near-miss joins the room it belongs to rather than starting a second
  // one. "garage" and "Garage" are one place to the person walking between
  // them and two places to an exact-match filter, which would quietly split a
  // room's history down the middle.
  await chips.locator('.chip[data-place=""]').click();
  await page.getByTestId('place-add').click();
  await page.getByTestId('place-input').fill('garage');
  await page.getByTestId('place-input').press('Enter');

  await expect(chips.locator('.chip[aria-pressed="true"]')).toHaveText('Garage');
  expect(
    await chips.locator('.chip', { hasText: /garage/i }).count(),
    'one room, one chip',
  ).toBe(1);
});

test('a stored run says which build measured it', async ({ page, request }) => {
  // A latency figure is only interpretable if you know what produced it:
  // everything recorded before 1.3.1 carries up to 40 ms of our own Nagle
  // stall. A run measured now must name its build and must NOT be marked
  // suspect — the marking has to be earned, or it means nothing.
  await completeARun(page);

  const runs = await (await request.get('/api/history')).json();
  expect(runs.length, 'the run just completed should be stored').toBeGreaterThan(0);
  const version = runs[0].appVersion;
  expect(version, 'every new run records its build').toMatch(/^\d+\.\d+\.\d+$/);

  await page.goto(`/result.html?id=${runs[0].id}`);
  const provenance = page.getByTestId('provenance');
  await expect(provenance).toBeVisible();
  await expect(provenance).toContainText(version);
  await expect(provenance).toHaveAttribute('data-suspect', 'false');
  await expect(provenance).not.toContainText('40 ms');

  // And the history table does not asterisk a sound run.
  await page.goto('/history.html');
  await expect(page.locator('#rows tr').first()).toBeVisible();
  expect(await page.locator('#rows tr').first().locator('.suspect').count()).toBe(0);
});

test('two runs can be selected and compared, with the difference computed', async ({
  page,
  request,
}) => {
  // The question the compare page exists to answer is "did that change help?".
  // A single run cannot answer it, and two tabs answer it badly.
  await completeARun(page);
  await completeARun(page);

  const runs = await (await request.get('/api/history')).json();
  expect(runs.length, 'need two runs to compare').toBeGreaterThanOrEqual(2);

  await page.goto('/history.html');
  const boxes = page.locator('input.pick');
  await expect(boxes.first()).toBeVisible();

  // Nothing selected: no bar.
  await expect(page.getByTestId('compare-bar')).toBeHidden();

  await boxes.nth(0).check();
  await expect(page.getByTestId('compare-bar')).toBeVisible();
  await expect(page.getByTestId('compare-count')).toContainText('pick one more');
  await expect(page.getByTestId('compare-go')).toBeHidden();

  await boxes.nth(1).check();
  await expect(page.getByTestId('compare-go')).toBeVisible();
  await page.getByTestId('compare-go').click();

  await expect(page.locator('body')).toHaveAttribute('data-compare-state', 'loaded');
  await expect(page.getByTestId('head-a')).toContainText('A');
  await expect(page.getByTestId('head-b')).toContainText('B');

  // A row per metric, each with a computed change rather than two numbers left
  // for the reader to difference.
  const rows = page.locator('#rows tr');
  expect(await rows.count()).toBeGreaterThanOrEqual(5);
  const deltas = page.locator('.cmp__delta');
  expect(await deltas.count()).toBe(await rows.count());

  // Every delta is either a signed percentage or an honest dash.
  for (const text of await deltas.allTextContents()) {
    expect(text.trim()).toMatch(/^([+-]?\d+\.\d%|—)$/);
  }

  // Both runs are current, so nothing should be marked as incomparable.
  await expect(page.getByTestId('caveat')).toBeHidden();
});

test('comparing a run with itself is refused rather than drawn as all zeroes', async ({
  page,
  request,
}) => {
  await completeARun(page);
  const runs = await (await request.get('/api/history')).json();
  const id = runs[0].id;

  await page.goto(`/compare.html?a=${id}&b=${id}`);
  await expect(page.locator('body')).toHaveAttribute('data-compare-state', 'error');
  await expect(page.getByTestId('error')).toContainText('same run');
});
