// Tiny worker that serves the built site from the ASSETS binding and adds the
// cross-origin-isolation headers wclap-host-js needs for threaded plugins.

export default {
  async fetch(request, env) {
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
