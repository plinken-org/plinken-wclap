// Main-thread side of plugin loading. Fetches a .wclap.tar.gz, parses it via
// wclap-host-js (`getWclap`), and forwards the result to the chain worklet
// over its MessagePort. The worklet does the wasm instantiation +
// host-registration; we only stage the bytes.

// @ts-expect-error — vendored JS module without types.
import { getWclap } from '@webclap/wclap-host-js';

export interface WclapManifest {
  id: string;
  name?: string;
  vendor?: string;
  version?: string;
  has_ui?: boolean;
  category?: string;
  [key: string]: unknown;
}

export interface LoadedWclap {
  /** Result of `getWclap(...)`. Contains `module` bytes + `files` map. */
  wclapConfig: any;
  /** Plugin id selected from the bundle (defaults to the first plugin). */
  pluginId: string;
  manifest: WclapManifest | null;
  files: Record<string, ArrayBuffer>;
}

export async function fetchWclap(url: string): Promise<LoadedWclap> {
  const absUrl = new URL(url, document.baseURI).href;
  const wclapConfig = await getWclap({ url: absUrl });

  const files: Record<string, ArrayBuffer> = wclapConfig.files || {};
  const manifest = parseManifest(files);
  const pluginId = manifest?.id ?? '';

  return { wclapConfig, pluginId, manifest, files };
}

function parseManifest(files: Record<string, ArrayBuffer>): WclapManifest | null {
  // Manifests live under /plugin/<hash>/plugin.json. Find any key ending in
  // /plugin.json — the bundle has exactly one.
  const key = Object.keys(files).find((k) => k.endsWith('/plugin.json'));
  if (!key) return null;
  try {
    const bytes = new Uint8Array(files[key]);
    const text = new TextDecoder().decode(bytes);
    return JSON.parse(text) as WclapManifest;
  } catch (e) {
    console.warn('[loader] failed to parse plugin.json:', e);
    return null;
  }
}
