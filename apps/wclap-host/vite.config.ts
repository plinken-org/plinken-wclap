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

// Mirrors the Cloudflare Worker's `/r2-proxy?u=<url>` route so URL-loaded
// shelf entries work in `pnpm dev` without deploying.
const r2ProxyDevMiddleware = {
  name: 'wclap-r2-proxy-dev',
  configureServer(server: import('vite').ViteDevServer) {
    server.middlewares.use('/r2-proxy', async (req, res) => {
      try {
        const url = new URL(req.url ?? '', 'http://localhost');
        const target = url.searchParams.get('u');
        if (!target) {
          res.statusCode = 400;
          res.end('missing ?u=<url>');
          return;
        }
        let targetUrl: URL;
        try {
          targetUrl = new URL(target);
        } catch {
          res.statusCode = 400;
          res.end('invalid target url');
          return;
        }
        if (targetUrl.protocol !== 'http:' && targetUrl.protocol !== 'https:') {
          res.statusCode = 400;
          res.end('only http(s) allowed');
          return;
        }
        const upstream = await fetch(targetUrl.href, {
          method: req.method as string,
          redirect: 'follow'
        });
        res.statusCode = upstream.status;
        for (const h of ['content-type', 'content-length', 'last-modified', 'etag']) {
          const v = upstream.headers.get(h);
          if (v) res.setHeader(h, v);
        }
        res.setHeader('Access-Control-Allow-Origin', '*');
        res.setHeader('Cross-Origin-Resource-Policy', 'cross-origin');
        if (req.method === 'HEAD' || !upstream.body) {
          res.end();
          return;
        }
        const reader = upstream.body.getReader();
        // eslint-disable-next-line no-constant-condition
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          res.write(value);
        }
        res.end();
      } catch (err) {
        res.statusCode = 502;
        res.end(`proxy fetch failed: ${err instanceof Error ? err.message : String(err)}`);
      }
    });
  }
};

export default defineConfig({
  plugins: [stripGzipEncodingForTarGz, r2ProxyDevMiddleware],
  resolve: {
    alias: {
      '@webclap/wclap-host-js': `${wclapJsRoot}/wclap.mjs`
    }
  },
  server: {
    port: 5174,
    strictPort: true,
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
