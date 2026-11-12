// Tiny worker that serves the built site from the ASSETS binding and adds the
// cross-origin-isolation headers wclap-host-js needs for threaded plugins.
//
// Also exposes `/r2-proxy?u=<urlencoded>` — a same-origin CORS-rewriting
// pass-through so the page can load `.wclap.tar.gz` bundles from any HTTP(S)
// origin without the upstream host needing to configure CORS itself. The
// proxy adds `Access-Control-Allow-Origin: *` + `Cross-Origin-Resource-Policy:
// cross-origin` to the upstream response so it satisfies the page's COEP.

import {
  BUNDLE_UI_PREFIX,
  BUNDLE_UI_HEADERS,
  parseBundleUiPath,
  findTarEntry,
  isGzip,
  mimeForBundleFile
} from './bundle-ui.js';

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname.startsWith(BUNDLE_UI_PREFIX)) {
      return handleBundleUi(request, env, url);
    }

    if (url.pathname === '/r2-proxy') {
      return handleProxy(request, url);
    }

    const response = await env.ASSETS.fetch(request);

    const headers = new Headers(response.headers);
    headers.set('Cross-Origin-Opener-Policy', 'same-origin');
    headers.set('Cross-Origin-Embedder-Policy', 'require-corp');

    return new Response(response.body, {
      status: response.status,
      statusText: response.statusText,
      headers
    });
  }
};

// One file out of a plugin bundle, for browsers where the page cannot use a
// service worker to serve the plugin's UI (see worker/bundle-ui.js).
async function handleBundleUi(request, env, url) {
  const fail = (status, why) =>
    new Response(`bundle-ui: ${why}`, { status, headers: { 'Content-Type': 'text/plain', ...BUNDLE_UI_HEADERS } });
  if (request.method !== 'GET' && request.method !== 'HEAD') return fail(405, 'method not allowed');
  const parsed = parseBundleUiPath(url.pathname);
  if (!parsed) return fail(400, 'bad path');

  // Only bundles this origin already serves: a shelf sample, or a remote
  // one through the existing /r2-proxy route.
  let bundle;
  try {
    bundle = new URL(parsed.bundleUrl, url.origin);
  } catch {
    return fail(400, 'bad bundle url');
  }
  if (bundle.origin !== url.origin) return fail(400, 'bundle must be on this origin');
  const upstream =
    bundle.pathname === '/r2-proxy'
      ? await handleProxy(new Request(bundle.href), bundle)
      : await env.ASSETS.fetch(new Request(bundle.href));
  if (!upstream.ok) return fail(404, 'bundle not found');

  let bytes = new Uint8Array(await upstream.arrayBuffer());
  if (isGzip(bytes)) {
    const stream = new Response(bytes).body.pipeThrough(new DecompressionStream('gzip'));
    bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  }
  const file = findTarEntry(bytes, parsed.filePath);
  if (!file) return fail(404, 'not in bundle');
  return new Response(request.method === 'HEAD' ? null : file, {
    headers: { 'Content-Type': mimeForBundleFile(parsed.filePath), ...BUNDLE_UI_HEADERS }
  });
}

async function handleProxy(request, requestUrl) {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return new Response('method not allowed', { status: 405 });
  }
  const target = requestUrl.searchParams.get('u');
  if (!target) return new Response('missing ?u=<url>', { status: 400 });

  let targetUrl;
  try {
    targetUrl = new URL(target);
  } catch {
    return new Response('invalid target url', { status: 400 });
  }
  if (targetUrl.protocol !== 'http:' && targetUrl.protocol !== 'https:') {
    return new Response('only http(s) allowed', { status: 400 });
  }

  const upstream = await fetch(targetUrl.href, {
    method: request.method,
    redirect: 'follow',
    cf: { cacheTtl: 300 }
  });

  // Pass through the response headers that actually matter for the page's
  // fetch + format-sniffing (content type, body length, caching), and add
  // CORS / CORP so the browser hands the bytes to the page.
  const headers = new Headers();
  for (const h of ['content-type', 'content-length', 'last-modified', 'etag']) {
    const v = upstream.headers.get(h);
    if (v) headers.set(h, v);
  }
  headers.set('Access-Control-Allow-Origin', '*');
  headers.set('Access-Control-Expose-Headers', 'Content-Length, Content-Type');
  headers.set('Cross-Origin-Resource-Policy', 'cross-origin');

  return new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers
  });
}
