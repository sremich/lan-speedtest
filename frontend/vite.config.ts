import { defineConfig } from 'vite';

// The backend serves the built assets in production, so everything is
// same-origin and no CORS handling is needed anywhere. In dev, Vite proxies
// the measurement endpoints to a locally running backend to preserve that.
const BACKEND = process.env.SPEEDTEST_BACKEND ?? 'http://127.0.0.1:8080';

export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // A single bundle keeps the page a plain static asset for the Rust
    // server: no code-splitting, no dynamic import paths to get wrong.
    rollupOptions: { output: { manualChunks: undefined } },
  },
  server: {
    proxy: {
      '/__down': BACKEND,
      '/__up': BACKEND,
      '/api': BACKEND,
    },
  },
});
