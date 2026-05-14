// Tiny worker that serves the built site from the ASSETS binding and adds the
// cross-origin-isolation headers wclap-host-js needs for threaded plugins.
//
// Also exposes `/r2-proxy?u=<urlencoded>` — a same-origin CORS-rewriting
// pass-through so the page can load `.wclap.tar.gz` bundles from any HTTP(S)
// origin without the upstream host needing to configure CORS itself. The
// proxy adds `Access-Control-Allow-Origin: *` + `Cross-Origin-Resource-Policy:
// cross-origin` to the upstream response so it satisfies the page's COEP.

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

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
