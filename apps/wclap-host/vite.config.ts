import { defineConfig } from 'vite';
import { fileURLToPath, URL } from 'node:url';

// The upstream `wclap-host-js` repo ships ES module sources (no package.json),
// so we expose it through an alias rather than as a workspace dep.
const wclapJsRoot = fileURLToPath(
  new URL('../../vendor/wclap-host-js', import.meta.url)
);

export default defineConfig({
  resolve: {
    alias: {
      '@webclap/wclap-host-js': `${wclapJsRoot}/wclap.mjs`
    }
  },
  server: {
    headers: {
      // Cross-origin isolation gates SharedArrayBuffer, which `wclap-host-js`
      // uses when threads are available. Safe to send unconditionally; if the
      // page loads cross-origin assets without CORP, the browser will refuse
      // them and `wclap-host-js` falls back to its non-threaded path.
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp'
    },
    fs: {
      // Vite needs to serve files from outside `apps/wclap-host/` (the alias
      // above resolves into `vendor/wclap-host-js/`).
      allow: ['../..']
    }
  },
  build: {
    target: 'es2022',
    sourcemap: true
  }
});
