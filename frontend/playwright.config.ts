import { defineConfig, devices } from '@playwright/test';

/**
 * Tier 2 — the real engine, in a real browser, against the real backend.
 *
 * The backend is started from the release binary and told to serve the built
 * front end, so this exercises the same static-serving path production uses.
 * Build both first:
 *
 *   cargo build --release --manifest-path backend/Cargo.toml
 *   npm --prefix frontend run build
 */

const PORT = Number(process.env.SPEEDTEST_E2E_PORT ?? 8099);
const BASE = `http://127.0.0.1:${PORT}`;
const BIN = process.env.SPEEDTEST_BIN ?? '../backend/target/release/lan-speedtest';

export default defineConfig({
  testDir: './tests',
  // A full run issues real transfers; these are not millisecond tests.
  timeout: 120_000,
  expect: { timeout: 30_000 },
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],

  use: {
    baseURL: BASE,
    trace: 'retain-on-failure',
  },

  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
  ],

  // Set SPEEDTEST_E2E_NO_SERVER=1 when the backend is already running and
  // Playwright is being driven from elsewhere — e.g. from the official
  // Playwright container against a backend on the host, which is how this
  // suite is run locally on a machine without the browser system libraries.
  ...(process.env.SPEEDTEST_E2E_NO_SERVER
    ? {}
    : {
        webServer: {
          command: BIN,
          url: `${BASE}/api/health`,
          reuseExistingServer: !process.env.CI,
          timeout: 30_000,
          stdout: 'pipe' as const,
          stderr: 'pipe' as const,
          env: {
            SPEEDTEST_BIND: `127.0.0.1:${PORT}`,
            SPEEDTEST_CONFIG: '../config/speedtest.toml',
            // Small transfers: this suite proves the wiring is right, not how
            // fast a runner's loopback is. Throughput is asserted in
            // backend/tests/throughput.rs.
            SPEEDTEST_PROFILE: 'quick',
            SPEEDTEST_STATIC_DIR: 'dist',
          },
        },
      }),
});
