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

/**
 * Ensures a <details> section is open.
 *
 * Clicking the summary toggles, so a section that already starts open would be
 * closed by a click — which is how this test broke when the detail view was
 * made open by default.
 */
async function ensureOpen(page: Page, testId: string): Promise<void> {
  const section = page.getByTestId(testId);
  if (!(await section.evaluate((el) => (el as HTMLDetailsElement).open))) {
    await section.locator('summary').click();
  }
  await expect
    .poll(async () => section.evaluate((el) => (el as HTMLDetailsElement).open))
    .toBe(true);
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

  await ensureOpen(page, 'detail');

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
  await ensureOpen(page, 'detail');

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

  await ensureOpen(page, 'raw');
  const raw = page.getByTestId('raw');
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
  await ensureOpen(page, 'raw');
  await page.getByTestId('raw-run').click();
  await expect
    .poll(async () => page.locator('body').getAttribute('data-raw-state'), { timeout: 90_000 })
    .toBe('done');

  expect(foreign, `requests left the origin:\n${foreign.join('\n')}`).toEqual([]);
});

test('the live traces are drawn for both directions', async ({ page }) => {
  // The trace is what makes the headline number legible as the shape of a
  // measurement rather than an unexplained summary of it.
  await page.goto('/');
  await waitForCompletion(page);

  for (const id of ['download-chart', 'upload-chart']) {
    const chart = page.getByTestId(id);
    await expect(chart.locator('svg'), `${id} should render`).toBeVisible();
    // A filled area and a stroked line, not just an axis.
    expect(await chart.locator('path').count()).toBeGreaterThanOrEqual(2);
  }

  // The reported percentile is marked, so the headline can be located on it.
  expect(await page.locator('.trace__marker').count()).toBeGreaterThanOrEqual(1);
});

test('loaded latency and jitter are shown per direction', async ({ page }) => {
  await page.goto('/');
  await waitForCompletion(page);

  for (const id of ['down-loaded', 'up-loaded', 'down-jitter', 'up-jitter']) {
    const value = await page.getByTestId(id).textContent();
    expect(value, `${id} should render something`).not.toBeNull();
    expect(
      value!.trim() === PENDING || /^(<0\.1|\d+\.\d+) ms$/.test(value!.trim()),
      `unexpected ${id} rendering: ${value}`,
    ).toBe(true);
  }
});

test('the run reports when it was measured and from where', async ({ page }) => {
  await page.goto('/');
  await waitForCompletion(page);

  await expect(page.getByTestId('measured-at')).toContainText(/Measured at/);
  // Stands in for the server-location panel: which machine this ran from.
  await expect(page.getByTestId('connection')).toContainText(/client: \S+/);
});

test('the page is named by the server, so a deployment can be renamed', async ({ page }) => {
  // The name lives in server config rather than in this bundle: two of these
  // on one LAN need to be tellable apart, and renaming one should be a
  // restart rather than a rebuild.
  await page.goto('/');

  const status = await page.evaluate(async () => {
    const res = await fetch('/api/status', { cache: 'no-store' });
    return (await res.json()) as { siteName: string };
  });
  expect(status.siteName, 'the backend must name itself').not.toBe('');

  await expect(page.getByTestId('site-name')).toHaveText(status.siteName);
  await expect.poll(async () => page.title()).toBe(status.siteName);
});

test('the layout scales from a phone to a wide monitor', async ({ page }) => {
  // Two failures this catches: content wider than the window (a horizontal
  // scrollbar on the whole page), and drawings scaled to fit rather than drawn
  // to size, which shrinks and grows their labels with the window instead of
  // keeping them legible.
  await page.goto('/');
  await waitForCompletion(page);
  await ensureOpen(page, 'detail');

  /** How far the browser has had to scale the box plots to make them fit. */
  const boxPlotScale = () =>
    page.evaluate(() => {
      const svg = document.querySelector('svg.box__svg');
      if (!svg) return 0;
      const drawn = Number(svg.getAttribute('viewBox')?.split(' ')[2] ?? 0);
      return drawn > 0 ? svg.getBoundingClientRect().width / drawn : 0;
    });

  const labelHeights: number[] = [];

  for (const size of [
    { width: 390, height: 844 },
    { width: 820, height: 900 },
    { width: 1280, height: 800 },
    { width: 1920, height: 1080 },
  ]) {
    await page.setViewportSize(size);

    // Never stretched, and only shrunk by the narrow-window floor. The redraw
    // is debounced, so this poll is also what waits for it.
    await expect
      .poll(boxPlotScale, { message: `box plots are mis-scaled at ${size.width}px` })
      .toBeGreaterThan(0.85);
    expect(await boxPlotScale(), `box plots are stretched at ${size.width}px`).toBeLessThanOrEqual(
      1.01,
    );

    // Reports the culprit, not just the number: "23px too wide" tells you
    // nothing about which element did it.
    const overflow = await page.evaluate(() => {
      const doc = document.documentElement;
      const by = doc.scrollWidth - doc.clientWidth;
      if (by <= 1) return { by, culprit: '' };
      const worst = [...document.querySelectorAll<HTMLElement>('body *')]
        .map((el) => ({ el, right: el.getBoundingClientRect().right }))
        .sort((a, b) => b.right - a.right)[0];
      return {
        by,
        culprit: worst
          ? `${worst.el.tagName.toLowerCase()}.${worst.el.className} reaches ${Math.round(worst.right)}`
          : 'unknown',
      };
    });
    expect(
      overflow.by,
      `page overflows horizontally at ${size.width}px — ${overflow.culprit}`,
    ).toBeLessThanOrEqual(1);

    const label = await page.locator('.box__label').first().boundingBox();
    expect(label, `no box-plot label at ${size.width}px`).not.toBeNull();
    labelHeights.push(label!.height);

    // The two traces stay side by side until there is genuinely no room.
    const down = await page.getByTestId('download-chart').boundingBox();
    const up = await page.getByTestId('upload-chart').boundingBox();
    expect(down).not.toBeNull();
    expect(up).not.toBeNull();
    if (size.width >= 820) {
      expect(
        Math.abs(down!.y - up!.y),
        `download and upload should share a row at ${size.width}px`,
      ).toBeLessThan(4);
    }
  }

  // The point of drawing at a measured pixel width: one label is very nearly
  // the same size on a phone as on a 1920px monitor. Before this, the same
  // plot rendered at 6px in a narrow window and 17px in a wide one.
  const spread = Math.max(...labelHeights) - Math.min(...labelHeights);
  expect(
    spread,
    `box-plot labels rescale with the window: ${labelHeights.join(', ')}`,
  ).toBeLessThanOrEqual(2.5);
});

test('the step strip shows every request the profile will issue', async ({ page }) => {
  // A bar says how far through you are; this says what the run is made of —
  // that it is mostly latency pings, or that the big transfers have not
  // started. The count comes from the profile, so it is known before the run.
  await page.goto('/');

  const strip = page.getByTestId('chevron-strip');
  await expect(strip.locator('.chev').first()).toBeVisible();

  const planned = Number(await page.locator('body').getAttribute('data-steps'));
  expect(planned).toBeGreaterThan(0);
  expect(await strip.locator('.chev').count()).toBe(planned);

  await waitForCompletion(page);

  // A finished run fills the strip completely; a partly filled strip after
  // "Complete" would misreport what was measured.
  expect(await strip.locator('.chev.is-done').count()).toBe(planned);
  await expect(strip.locator('.chevrons')).toHaveAttribute('aria-valuenow', String(planned));
});

test('hovering a step explains it and reports what it measured', async ({ page }) => {
  await page.goto('/');
  await waitForCompletion(page);

  // The LAST download stage, not the first: the first is the engine's warm-up
  // round, which is exempt from the minimum-duration rule and contributes no
  // bandwidth sample at all, so it has nothing to report.
  await page.locator('.chev--download').last().hover();

  const tip = page.getByTestId('step-tip');
  await expect(tip).toBeVisible();
  const text = (await tip.textContent()) ?? '';
  expect(text).toMatch(/Download/);
  expect(text, 'the reference shows a request count').toMatch(/Requests:\s*\d+/);
  expect(text, 'and where in the run this is').toMatch(/Step:\s*\d+ of \d+/);
  expect(text, 'a payload size, as the reference does').toMatch(/Payload:\s*[\d.]+\s*[kM]?B/);
  expect(text, 'and what it actually measured').toMatch(/Measured:/);

  // The whole stage lights up, not just the one chevron under the pointer.
  expect(await page.locator('.chev.is-hovered').count()).toBeGreaterThanOrEqual(1);

  // The warm-up says why it has no figure, rather than leaving a gap that
  // reads as a measurement that failed.
  await page.locator('.chev--download').first().hover();
  await expect(tip).toContainText(/warm-up/i);
});

test('hovering a trace reports the sample under the pointer', async ({ page }) => {
  // The engine records the payload size, the round trip and the request
  // duration for every sample. The headline uses none of that; a point on the
  // curve is where it belongs.
  await page.goto('/');
  await waitForCompletion(page);

  const chart = page.getByTestId('download-chart');
  const box = await chart.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width * 0.6, box!.y + box!.height * 0.5);

  const tip = chart.locator('.trace__tip');
  await expect(tip).toBeVisible();
  const text = (await tip.textContent()) ?? '';
  expect(text).toMatch(/Download/);
  expect(text).toMatch(/Speed:\s*[\d.]+\s*[KMG]bps/);
  expect(text).toMatch(/Payload:/);

  // The cursor lands on the curve rather than floating anywhere.
  await expect(chart.locator('.trace__cursor')).toBeVisible();

  // And it goes away again.
  await page.mouse.move(box!.x + box!.width / 2, box!.y - 40);
  await expect(tip).toBeHidden();
});

test('the trace is drawn as a curve, not a polyline', async ({ page }) => {
  // Monotone cubic: smooth, but incapable of overshooting between samples. A
  // spline that overshoots would draw a bandwidth dip below zero on a ramp.
  await page.goto('/');
  await waitForCompletion(page);

  const d = await page.locator('#download-chart path').nth(1).getAttribute('d');
  expect(d, 'no path drawn').not.toBeNull();
  expect(d, 'the trace should be cubic segments').toMatch(/C-?[\d.]+,/);
});

test('the profile is selectable, and the run reports the one it used', async ({ page }) => {
  // Every stored run said "lan-1g" because the profile was fixed server-side.
  // It is a real choice now, and the transfer sizes differ between profiles,
  // so the label has to be the truth.
  await page.goto('/');

  const picker = page.getByTestId('profile-select');
  await expect(picker).toBeVisible();

  const values = await picker
    .locator('option')
    .evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value));
  expect(values, 'the shipped profiles should be offered').toContain('lan-1g');
  expect(values, 'and automatic selection').toContain('__auto__');

  await waitForCompletion(page);
  await expect(page.locator('body')).toHaveAttribute('data-profile', 'quick');

  // Choosing another profile re-runs with it rather than relabelling the old
  // numbers, which would be the worst of both.
  await picker.selectOption('lan-1g');
  await expect
    .poll(async () => page.locator('body').getAttribute('data-profile'), { timeout: 60_000 })
    .toBe('lan-1g');
  await waitForCompletion(page);
  await expect(page.getByTestId('profile')).toContainText('lan-1g');

  // And it is remembered, so the next visit does not silently revert.
  expect(await page.evaluate(() => localStorage.getItem('speedtest.profile'))).toBe('lan-1g');
});

test('a running test can be paused and resumed', async ({ page }) => {
  await page.goto('/');
  // Wait until it is genuinely under way before pausing.
  await expect(page.getByTestId('phase')).not.toHaveText('Starting…', { timeout: 30_000 });

  const pause = page.getByTestId('pause');
  await pause.click();
  await expect(pause).toHaveText('Resume');

  await pause.click();
  await expect(pause).toHaveText('Pause');

  // And it still finishes afterwards.
  await waitForCompletion(page);
});
