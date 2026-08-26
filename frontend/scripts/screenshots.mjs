/**
 * Regenerates the screenshots used by the README and the wiki.
 *
 * Documentation screenshots rot silently: the page changes, the prose is
 * updated, and the picture beside it keeps showing last year's layout. This
 * script exists so that refreshing them is one command against a real backend
 * rather than a manual round of window-cropping nobody wants to repeat.
 *
 * Every figure in the output is a genuine measurement of whatever link the
 * script is pointed at. Nothing is mocked, seeded or drawn — a screenshot of
 * fabricated numbers would teach the reader to expect a page that does not
 * exist.
 *
 * Usage — see docs/wiki/Development.md for the backend invocation:
 *
 *   node scripts/screenshots.mjs
 *   SHOT_BASE=http://127.0.0.1:8080 SHOT_OUT=../../docs/wiki/images \
 *     node scripts/screenshots.mjs
 *
 * The backend must have history enabled and should have TURN configured;
 * without a relay the packet-loss figure is a dash in every shot.
 */

import { chromium } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));

const BASE = process.env.SHOT_BASE ?? 'http://127.0.0.1:8080';
// `||` not `??`: SHOT_OUT="" should fall back, not resolve OUT to HERE and
// scatter the images into this directory.
const OUT = path.resolve(HERE, process.env.SHOT_OUT || '../../docs/wiki/images');

/**
 * Extra clients for the history, obtained honestly.
 *
 * A history with a single client photographs badly — the per-client filter and
 * the client column both read as decoration. These addresses come from the
 * documented trusted-proxy path: the backend is started with
 * `SPEEDTEST_TRUSTED_PROXIES=127.0.0.1/32`, so it believes an
 * `X-Forwarded-For` from loopback and files the run under that address.
 *
 * The measurements are still real. Only the address a run is *filed under* is
 * arranged, through a feature that behaves exactly this way in production.
 * See docs/wiki/Client-Identity.md.
 */
const EXTRA_CLIENTS = [
  { ip: '192.168.1.24', runs: 2 },
  { ip: '192.168.1.51', runs: 1 },
];

/** Wide enough for the two-column layout, short enough to stay readable. */
const VIEWPORT = { width: 1280, height: 900 };
/** Retina: these are read scaled down on GitHub, and 1x looks muddy there. */
const SCALE = 2;

const RUN_TIMEOUT = 180_000;

let shotCount = 0;

async function shot(target, name, options = {}) {
  await target.screenshot({ path: path.join(OUT, `${name}.png`), ...options });
  shotCount += 1;
  console.log(`  → ${name}.png`);
}

/** Waits for a run to reach a terminal state, surfacing the UI's own error. */
async function waitForRun(page) {
  await page.waitForFunction(
    () => document.body.dataset.testState && document.body.dataset.testState !== 'running',
    null,
    { timeout: RUN_TIMEOUT },
  );
  const state = await page.evaluate(() => document.body.dataset.testState);
  if (state !== 'complete') {
    throw new Error(`run ended in state "${state}": ${await page.locator('#error').textContent()}`);
  }
}

/** Waits for the completed run to be persisted, so the history has it. */
async function waitForStored(page) {
  await page.waitForFunction(() => document.body.dataset.resultStored === 'yes', null, {
    timeout: 30_000,
  });
}

/** Reads the id of the run the page has just stored. */
async function storedId(page) {
  const href = await page.locator('#permalink').getAttribute('href');
  return href ? new URL(href, BASE).searchParams.get('id') : null;
}

/** Runs the test once in its own tab and returns the stored run's id. */
async function completeARun(context) {
  const page = await context.newPage();
  await page.goto(`${BASE}/`);
  await waitForRun(page);
  await waitForStored(page);
  const id = await storedId(page);
  await page.close();
  return id;
}

/** A fresh context, optionally filing its runs under another address. */
function makeContext(browser, { colorScheme = 'dark', clientIp } = {}) {
  return browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: SCALE,
    colorScheme,
    reducedMotion: 'reduce',
    ...(clientIp ? { extraHTTPHeaders: { 'X-Forwarded-For': clientIp } } : {}),
  });
}

/**
 * Moves the pointer onto an element and jiggles it.
 *
 * The tooltips are driven by `pointermove` rather than `:hover`, and a single
 * synthetic move can land before the chart has bound its listener. Two moves a
 * pixel apart is the cheap, reliable version.
 */
async function pointAt(page, locator, dx = 0, dy = 0) {
  const box = await locator.boundingBox();
  if (!box) throw new Error('element has no box to point at');
  const x = box.x + box.width / 2 + dx;
  const y = box.y + box.height / 2 + dy;
  await page.mouse.move(x, y);
  await page.mouse.move(x + 1, y);
  await page.waitForTimeout(300);
}

/** Opens a <details> section if it is closed. */
async function ensureOpen(page, selector) {
  const section = page.locator(selector);
  if (!(await section.evaluate((el) => el.open))) {
    await section.locator('summary').click();
    await page.waitForTimeout(200);
  }
}

async function main() {
  await mkdir(OUT, { recursive: true });
  console.log(`backend: ${BASE}`);
  console.log(`output:  ${OUT}\n`);

  const browser = await chromium.launch();
  const context = await makeContext(browser);

  // ---- a run in progress ---------------------------------------------------
  console.log('run in progress');
  const live = await context.newPage();
  await live.goto(`${BASE}/`);
  // Wait until a real stage is on screen — "Starting…" says nothing.
  await live.waitForFunction(
    () => /download|upload|latency|packet/i.test(document.querySelector('#phase')?.textContent ?? ''),
    null,
    { timeout: 60_000 },
  );
  await live.waitForTimeout(2000);
  await shot(live.locator('section.steps'), 'progress-strip');

  // The chevron tooltip: what a stage is, and what it measured. It has to
  // land on a chevron — the strip's `pointermove` handler ignores the gaps
  // between them, so aiming at the container's centre reliably misses.
  const chevs = live.locator('#chevrons .chev');
  const finished = await chevs.evaluateAll((els) =>
    els.reduce((last, el, i) => (el.className.includes('is-done') ? i : last), 0),
  );
  const target = chevs.nth(Math.max(0, Math.floor(finished / 2)));
  await target.scrollIntoViewIfNeeded();
  await pointAt(live, target);
  await shot(live.locator('section.steps'), 'progress-strip-tooltip');

  await waitForRun(live);
  await waitForStored(live);
  const firstId = await storedId(live);

  // ---- the finished run ----------------------------------------------------
  console.log('finished run');
  await live.evaluate(() => window.scrollTo(0, 0));
  await shot(live, 'overview');
  await shot(live.locator('section.band'), 'headline-figures');
  await shot(live.locator('section.aim'), 'quality-ratings');

  // One sample, named: speed, payload, round trip, request duration.
  await pointAt(live, live.locator('#download-chart'), 40, 0);
  await shot(live.locator('.trace').first(), 'trace-tooltip');
  await live.mouse.move(0, 0);

  // ---- the distribution view ----------------------------------------------
  console.log('distribution');
  await ensureOpen(live, '#detail');
  await live.locator('#detail').scrollIntoViewIfNeeded();
  await live.waitForTimeout(300);
  await shot(live.locator('#detail'), 'distribution');

  const row = live.locator('#detail-body .box__row').first();
  if (await row.count()) {
    await pointAt(live, row);
    await shot(live.locator('#detail'), 'distribution-tooltip');
    await live.mouse.move(0, 0);
  } else {
    console.warn('  ! no box-plot rows to hover');
  }

  // ---- raw throughput ------------------------------------------------------
  console.log('raw throughput');
  await ensureOpen(live, '#raw');
  await live.locator('#raw').scrollIntoViewIfNeeded();
  await live.locator('#raw-run').click();
  await live.waitForFunction(() => !document.querySelector('#raw-result')?.hidden, null, {
    timeout: 120_000,
  });
  await live.waitForTimeout(300);
  await shot(live.locator('#raw'), 'raw-throughput');
  await live.close();

  // ---- a history worth photographing --------------------------------------
  console.log('filling history');
  const ids = [firstId, await completeARun(context)];

  for (const { ip, runs } of EXTRA_CLIENTS) {
    const other = await makeContext(browser, { clientIp: ip });
    for (let i = 0; i < runs; i += 1) ids.push(await completeARun(other));
    await other.close();
  }

  // A typed name on one client, so the history shows a label and an address
  // side by side rather than a column of bare addresses.
  const named = await context.request.post(`${BASE}/api/clients/127.0.0.1/name`, {
    data: { name: 'workbench' },
  });
  if (!named.ok()) console.warn(`  ! naming the client failed: ${named.status()}`);

  // A description on one run: which *test*, as opposed to which machine.
  if (ids[1]) {
    await context.request.post(`${BASE}/api/results/${ids[1]}/note`, {
      data: { note: 'Wired, before moving the switch' },
    });
  }

  // ---- history -------------------------------------------------------------
  console.log('history');
  const hist = await context.newPage();
  await hist.goto(`${BASE}/history.html`);
  await hist.waitForFunction(() => document.body.dataset.historyState === 'loaded', null, {
    timeout: 30_000,
  });
  await hist.waitForTimeout(500);
  await shot(hist, 'history');
  await shot(hist.locator('#chart'), 'history-trend');
  await shot(hist.locator('.table-wrap'), 'history-table');

  // ---- compare -------------------------------------------------------------
  console.log('compare');
  const picks = hist.locator('#rows input[type="checkbox"]');
  const n = await picks.count();
  if (n >= 2) {
    await picks.nth(0).check();
    await picks.nth(Math.min(2, n - 1)).check();
    await hist.locator('#compare-go').click();
    await hist.waitForLoadState('load');
    await hist.waitForTimeout(1500);
    await shot(hist, 'compare');
  } else {
    console.warn('  ! not enough rows to compare');
  }
  await hist.close();

  // ---- a stored run's own page --------------------------------------------
  console.log('permalink');
  const res = await context.newPage();
  await res.goto(`${BASE}/result.html?id=${ids[1] ?? firstId}`);
  await res.waitForTimeout(1500);
  await shot(res, 'result-page');
  await res.close();
  await context.close();

  // ---- the light theme -----------------------------------------------------
  console.log('light theme');
  const lightCtx = await makeContext(browser, { colorScheme: 'light' });
  const light = await lightCtx.newPage();
  await light.goto(`${BASE}/result.html?id=${ids[1] ?? firstId}`);
  await light.waitForTimeout(1500);
  await shot(light, 'light-theme');
  await lightCtx.close();

  await browser.close();
  console.log(`\n${shotCount} screenshots written to ${OUT}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
