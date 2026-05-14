// Service worker that serves plugin webview assets to iframes loaded under
// /plugin-proxy/. The page provides the bytes via postMessage; we just route
// requests to the page and translate the response into an HTTP Response.
//
// Background: WCLAP plugins bundle their UI as an HTML file plus relative
// asset scripts (CSS, JS, images) inside the .wclap.tar.gz. The host
// unpacks them into an in-memory files map keyed by `/plugin/<hash>/...`.
// An iframe needs a URL it can navigate to, not an in-memory buffer — so we
// expose the files map through this SW under a stable URL prefix.

const SCOPE_PREFIX = '/plugin-proxy/';
const REQUEST_TIMEOUT_MS = 5000;
const pending = new Map();
let nextId = 1;

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (url.pathname.startsWith(SCOPE_PREFIX)) {
    event.respondWith(handle(event, url.pathname.slice(SCOPE_PREFIX.length - 1)));
  }
});

async function handle(event, innerPath) {
  // The fetch was initiated from inside the iframe (which lives under
  // /plugin-proxy/), but the iframe has no idea where to find the file.
  // Always route the request to the top-level page, which holds the
  // proxyResolvers map. Fall back to any window if none is top-level.
  const windows = await self.clients.matchAll({
    includeUncontrolled: true,
    type: 'window'
  });
  const client =
    windows.find((c) => c.frameType === 'top-level') ?? windows[0];
  if (!client) {
    return new Response('plugin-proxy: no client', {
      status: 503,
      headers: PROXY_HEADERS
    });
  }

  const id = nextId++;
  const promise = new Promise((resolve) => pending.set(id, resolve));
  client.postMessage({ type: 'plugin-proxy-request', id, path: innerPath });

  const result = await Promise.race([
    promise,
    new Promise((resolve) =>
      setTimeout(() => resolve(null), REQUEST_TIMEOUT_MS)
    )
  ]);
  pending.delete(id);

  if (!result || !result.body) {
    return new Response('plugin-proxy: not found', {
      status: 404,
      headers: PROXY_HEADERS
    });
  }
  return new Response(result.body, {
    headers: {
      'Content-Type': result.mime || 'application/octet-stream',
      ...PROXY_HEADERS
    }
  });
}

// Inherit cross-origin isolation from the host page so the iframe can use
// SharedArrayBuffer if the plugin's JS needs it. Every response from this
// SW — 200, 404, 503 — must carry these or it'll fail COEP=require-corp.
const PROXY_HEADERS = {
  'Cross-Origin-Embedder-Policy': 'require-corp',
  'Cross-Origin-Resource-Policy': 'same-origin'
};

self.addEventListener('message', (event) => {
  const data = event.data;
  if (data?.type === 'plugin-proxy-response') {
    const resolver = pending.get(data.id);
    if (resolver) resolver(data);
  }
});
