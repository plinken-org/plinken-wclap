// Main-thread side of plugin loading. Fetches a .wclap.tar.gz, parses it via
// wclap-host-js (`getWclap`), and forwards the result to the chain worklet
// over its MessagePort. The worklet does the wasm instantiation +
// host-registration; we only stage the bytes.

// @ts-expect-error — vendored JS module without types.
import { getWclap } from '@webclap/wclap-host-js';

export interface WclapPanelSize {
  width: number;
  height: number;
}

export interface WclapManifest {
  id: string;
  name?: string;
  vendor?: string;
  version?: string;
  has_ui?: boolean;
  category?: string;
  ui?: {
    compact_size?: WclapPanelSize;
    expanded_size?: WclapPanelSize;
  };
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

  // For raw `.wasm` bundles, wclap-host-js's `getWclap` calls
  // `WebAssembly.compileStreaming` and parks the compiled Module on
  // `wclapConfig.module`, leaving `files[<pluginPath>/module.wasm]` as an
  // empty ArrayBuffer placeholder. We strip `module` before postMessage
  // (Chromium refuses to transfer WebAssembly.Module to AudioWorklets),
  // so the worklet's recompile-from-bytes path needs the actual bytes.
  // Re-fetch them here when the placeholder is empty.
  const pluginPath = (wclapConfig as { pluginPath?: string }).pluginPath;
  if (pluginPath) {
    const wasmKey = `${pluginPath}/module.wasm`;
    const existing = files[wasmKey];
    if (!existing || existing.byteLength === 0) {
      const resp = await fetch(absUrl);
      const ctype = resp.headers.get('Content-Type') ?? '';
      // Tar.gz bundles already wrote bytes into files[wasmKey] inside
      // wclap-host-js; we only refetch when the URL itself is wasm.
      if (ctype.includes('wasm') || /\.wasm(\?|$)/.test(absUrl)) {
        files[wasmKey] = await resp.arrayBuffer();
      }
    }
  }

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
