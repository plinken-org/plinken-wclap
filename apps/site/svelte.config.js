import adapter from '@sveltejs/adapter-cloudflare';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({
      // Defaults are fine. Adapter emits `.svelte-kit/cloudflare/_worker.js`
      // plus a static assets directory, both referenced from wrangler.jsonc.
    })
  }
};

export default config;
