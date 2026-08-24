import { resolve } from 'node:path';

import { defineConfig } from 'vitest/config';

// The backend serves the built assets in production, so everything is
// same-origin and no CORS handling is needed anywhere. In dev, Vite proxies
// the measurement endpoints to a locally running backend to preserve that.
const BACKEND = process.env.SPEEDTEST_BACKEND ?? 'http://127.0.0.1:8080';

export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      // Two pages, each a plain static asset for the Rust server to serve.
      input: {
        index: resolve(__dirname, 'index.html'),
        history: resolve(__dirname, 'history.html'),
      },
      output: { manualChunks: undefined },
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
