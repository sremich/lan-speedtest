import { resolve } from 'node:path';

import { defineConfig } from 'vitest/config';

// The backend serves the built assets in production, so everything is
// same-origin and no CORS handling is needed anywhere. In dev, Vite proxies
// the measurement endpoints to a locally running backend to preserve that.
const BACKEND = process.env.SPEEDTEST_BACKEND ?? 'http://127.0.0.1:8080';

// The engine is MIT and its licence requires the notice to travel with the
// code. It cannot travel on its own: `@cloudflare/speedtest` keeps its notice
// in a sibling LICENSE file rather than a banner comment, so there is nothing
// in the module for esbuild's `legalComments` to preserve — minification
// leaves the bundle with no attribution at all. Hence an explicit banner,
// applied to every chunk because `manualChunks` is free to move the engine
// into any of them. `THIRD-PARTY-NOTICES.md` carries the full text.
const LICENCE_BANNER = `/*!
 * Bundles @cloudflare/speedtest — MIT, Copyright (c) 2023 Cloudflare.
 * https://github.com/cloudflare/speedtest
 * Full third-party notices: /THIRD-PARTY-NOTICES.md
 */`;

export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      // Every page, each a plain static asset for the Rust server to serve.
      //
      // A page missing from this list is silently not built: the Dockerfile
      // copies `frontend/*.html` by glob, so the HTML ships and the script it
      // references does not. Add the page here at the same time as the file.
      input: {
        index: resolve(__dirname, 'index.html'),
        history: resolve(__dirname, 'history.html'),
        result: resolve(__dirname, 'result.html'),
        compare: resolve(__dirname, 'compare.html'),
      },
      output: { manualChunks: undefined, banner: LICENCE_BANNER },
    },
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
  server: {
    proxy: {
      '/__down': BACKEND,
      '/__up': BACKEND,
      '/api': BACKEND,
    },
  },
});
