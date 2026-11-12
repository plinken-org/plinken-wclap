// Serving a plugin's UI files straight out of its `.wclap.tar.gz`, by plain
// HTTP: `/bundle-ui/<base64url(bundle url)>/<path inside the bundle>`.
//
// The page normally serves these files to the plugin iframe through a
// service worker (public/plugin-proxy-sw.js). Where no service worker can
// be registered — Safari on some iPads, private windows — the page points
// the iframe at this route instead. Relative imports inside the UI resolve
// under the same prefix, so nothing in the bundle has to be rewritten.
//
// Pure functions shared by the Cloudflare worker (worker/index.js) and the
// vite dev server (vite.config.ts).

export const BUNDLE_UI_PREFIX = '/bundle-ui/';

// The host page is COEP=require-corp: an iframe document without these is
// refused. Bundles change under a stable URL, so never serve a stale file.
export const BUNDLE_UI_HEADERS = {
  'Cross-Origin-Embedder-Policy': 'require-corp',
  'Cross-Origin-Resource-Policy': 'same-origin',
  'Cache-Control': 'no-cache'
};

/** base64url of a UTF-8 string (the bundle's URL as the page knows it). */
export function encodeBundleUrl(url) {
  const bytes = new TextEncoder().encode(url);
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** Split a request path into the bundle URL and the file wanted from it.
 *  Returns null for anything that is not a well-formed bundle-ui path. */
export function parseBundleUiPath(pathname) {
  if (!pathname.startsWith(BUNDLE_UI_PREFIX)) return null;
  const rest = pathname.slice(BUNDLE_UI_PREFIX.length);
  const cut = rest.indexOf('/');
  if (cut <= 0) return null;
  let bundleUrl;
  try {
    const b64 = rest.slice(0, cut).replace(/-/g, '+').replace(/_/g, '/');
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    bundleUrl = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
  let filePath;
  try {
    filePath = decodeURIComponent(rest.slice(cut + 1));
  } catch {
    return null;
  }
  // The browser has already resolved `..`; refuse anything that still
  // tries to climb, and empty names.
  if (!filePath || filePath.split('/').some((part) => part === '..' || part === '')) return null;
  return { bundleUrl, filePath };
}

export function isGzip(bytes) {
  return bytes.length > 2 && bytes[0] === 0x1f && bytes[1] === 0x8b;
}

function field(tar, start, length) {
  let end = start;
  while (end < start + length && tar[end] !== 0) end++;
  return new TextDecoder().decode(tar.subarray(start, end));
}

/** Contents of regular file `name` in an (uncompressed) tar archive, or
 *  null. `name` is relative to the archive root, without a leading `./`. */
export function findTarEntry(tar, name) {
  let off = 0;
  while (off + 512 <= tar.length) {
    const entryName = field(tar, off, 100);
    if (!entryName) break; // end-of-archive zero block
    const prefix = field(tar, off + 345, 155);
    const size = parseInt(field(tar, off + 124, 12).trim() || '0', 8) || 0;
    const type = tar[off + 156];
    const full = (prefix ? prefix + '/' : '') + entryName;
    const clean = full.startsWith('./') ? full.slice(2) : full;
    const isFile = type === 0 || type === 0x30; // '\0' or '0'
    if (isFile && clean === name) return tar.subarray(off + 512, off + 512 + size);
    off += 512 + Math.ceil(size / 512) * 512;
  }
  return null;
}

const MIME = {
  html: 'text/html; charset=utf-8',
  htm: 'text/html; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
  mjs: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8',
  json: 'application/json; charset=utf-8',
  svg: 'image/svg+xml',
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  woff: 'font/woff',
  woff2: 'font/woff2',
  wasm: 'application/wasm',
  txt: 'text/plain; charset=utf-8'
};

export function mimeForBundleFile(path) {
  const m = /\.([a-z0-9]+)$/i.exec(path);
  return MIME[(m ? m[1] : '').toLowerCase()] || 'application/octet-stream';
}
