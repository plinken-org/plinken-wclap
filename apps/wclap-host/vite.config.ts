import { defineConfig } from 'vite';
import { fileURLToPath, URL } from 'node:url';

// The upstream `wclap-host-js` repo ships ES module sources (no package.json),
// so we expose it through an alias rather than as a workspace dep.
const wclapJsRoot = fileURLToPath(
  new URL('../../vendor/wclap-host-js', import.meta.url)
);

// `.wclap.tar.gz` bundles are opaque binary payloads — the host fetches them
// and reads the gzip magic itself. Vite/sirv otherwise sets
// `Content-Encoding: gzip` because the file ends in `.gz`, which makes the
// browser transparently decompress and the fetched bytes start with the first
// tar entry name instead of `1f 8b`.
const stripGzipEncodingForTarGz = {
  name: 'wclap-strip-gzip-encoding-for-tar-gz',
  configureServer(server: import('vite').ViteDevServer) {
    server.middlewares.use((req, res, next) => {
      if (req.url?.endsWith('.tar.gz')) {
        const origSetHeader = res.setHeader.bind(res);
        res.setHeader = (name: string, value: number | string | readonly string[]) => {
          if (name.toLowerCase() === 'content-encoding') return res;
          return origSetHeader(name, value);
        };
      }
      next();
    });
  }
};

export default defineConfig({
  plugins: [stripGzipEncodingForTarGz],
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
      'Cross-Origin-Embedder-Policy': 'require-corp',
      'Cross-Origin-Resource-Policy': 'same-origin'
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
