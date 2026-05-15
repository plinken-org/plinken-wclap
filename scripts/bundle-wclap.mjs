// Bundle a built WCLAP plugin (module.wasm + ui assets) into the .wclap.tar.gz
// format the host expects.
//
// Usage:
//   node scripts/bundle-wclap.mjs <plugin-dir>
//
// Reads `<plugin-dir>/plugin.json`. The manifest's `artifact` field is the
// output path (must end in `.tar.gz`); the script expects to find the built
// wasm at `<plugin-dir>/dist/<artifact-basename>.wasm` (i.e. the same path
// you'd ship as a bare wasm artifact, but with `.wasm` extension). UI assets
// come from `<plugin-dir>/ui/`.
//
// The tarball uses POSIX ustar format. We hand-roll it rather than pulling
// in a tar dep so the build chain stays npm-free at this layer.
//
//   module.wasm                  ← required, becomes the host's entrypoint
//   <ui/...>                     ← copied verbatim under the same paths

import { readFile, writeFile, readdir, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, basename, resolve, relative, dirname } from 'node:path';
import { gzipSync } from 'node:zlib';

const BLOCK = 512;

/**
 * Encode one ustar header + content (padded). Returns a Buffer ready to
 * concatenate. Throws on names that don't fit ustar's 100-byte name field —
 * we don't need the prefix split for the small flat tree we produce.
 */
function tarEntry(path, content) {
  if (typeof content === 'string') content = Buffer.from(content);
  if (path.length > 100) {
    throw new Error(
      `tar entry path too long for ustar (${path.length} > 100): ${path}`
    );
  }

  const header = Buffer.alloc(BLOCK);
  header.write(path, 0, 100, 'utf8');
  header.write('0000644 ', 100, 8, 'ascii');   // mode (octal + space + NUL handled by write())
  header.write('0000000 ', 108, 8, 'ascii');   // uid
  header.write('0000000 ', 116, 8, 'ascii');   // gid
  // size: 11-digit octal + NUL = 12 bytes
  header.write(content.length.toString(8).padStart(11, '0') + '\0', 124, 12, 'ascii');
  // mtime: 0 epoch for reproducibility
  header.write('00000000000 ', 136, 12, 'ascii');
  // checksum placeholder (8 spaces — computed below)
  header.write('        ', 148, 8, 'ascii');
  header.write('0', 156, 1, 'ascii');          // type flag '0' = regular file
  // linkname stays blank
  header.write('ustar\0', 257, 6, 'binary');
  header.write('00', 263, 2, 'binary');        // version
  // uname/gname/devmajor/devminor/prefix all stay zero

  // Header checksum: sum of all header bytes treating the checksum field as
  // spaces (already done above). Encoded as 6-digit octal + NUL + space.
  let sum = 0;
  for (let i = 0; i < BLOCK; i++) sum += header[i];
  header.write(sum.toString(8).padStart(6, '0') + '\0 ', 148, 8, 'ascii');

  const padLen = (BLOCK - (content.length % BLOCK)) % BLOCK;
  const pad = Buffer.alloc(padLen);
  return Buffer.concat([header, content, pad]);
}

function tarEnd() {
  // Two zero blocks as end-of-archive marker.
  return Buffer.alloc(BLOCK * 2);
}

async function* walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      yield* walk(full);
    } else if (entry.isFile()) {
      yield full;
    }
  }
}

async function main() {
  const pluginDir = process.argv[2];
  if (!pluginDir) {
    console.error('usage: node scripts/bundle-wclap.mjs <plugin-dir>');
    process.exit(1);
  }
  const root = resolve(pluginDir);
  const manifest = JSON.parse(await readFile(join(root, 'plugin.json'), 'utf8'));

  if (manifest.format !== 'tar.gz') {
    console.error(
      `bundle-wclap: plugin.json format is '${manifest.format}' — this bundler only emits 'tar.gz'`
    );
    process.exit(1);
  }
  if (!manifest.artifact?.endsWith('.tar.gz')) {
    console.error(
      `bundle-wclap: artifact '${manifest.artifact}' must end in '.tar.gz'`
    );
    process.exit(1);
  }

  // Locate the built wasm. By convention the asc target writes it next to
  // where the tar.gz will land, with `.wasm` instead of `.tar.gz`.
  const wasmPath = join(
    root,
    manifest.artifact.replace(/\.tar\.gz$/, '.wasm')
  );
  if (!existsSync(wasmPath)) {
    console.error(
      `bundle-wclap: built wasm missing at ${wasmPath} — run \`pnpm --filter ${manifest.id} build:wasm\` first`
    );
    process.exit(1);
  }

  const entries = [];

  // module.wasm is required, always at the root of the bundle. Host code
  // (vendor/wclap-host-js/es6/wclap-plugin.mjs) expects this exact name.
  entries.push({ path: 'module.wasm', content: await readFile(wasmPath) });

  // plugin.json is shipped alongside so the host can read manifest metadata
  // (has_ui, ui.compact_size / expanded_size, etc.) at runtime. Without
  // this, the host-side `parseManifest` returns null and any UI affordance
  // gated on manifest fields (strip view, latency hints, ...) stays off.
  entries.push({ path: 'plugin.json', content: await readFile(join(root, 'plugin.json')) });

  // ui/ is optional. Walk it and add each file, preserving relative paths.
  const uiDir = join(root, 'ui');
  if (existsSync(uiDir)) {
    for await (const file of walk(uiDir)) {
      const rel = relative(root, file).replaceAll('\\', '/');
      entries.push({ path: rel, content: await readFile(file) });
    }

    // Shared widget library — `widgets/` at the repo root. Plugins that
    // have a UI get a copy bundled in at `widgets/...`, so their
    // ui/index.html can do `import { Meter } from '../widgets/meter.mjs'`.
    // Plugins without a UI skip this to keep their tarball tiny.
    //
    // Path traversal: bundle-wclap.mjs lives in <repo>/scripts/, so the
    // repo root is two `..` up from this file.
    const widgetsDir = resolve(dirname(new URL(import.meta.url).pathname), '..', 'widgets');
    if (existsSync(widgetsDir)) {
      for await (const file of walk(widgetsDir)) {
        const rel = ('widgets/' + relative(widgetsDir, file)).replaceAll('\\', '/');
        entries.push({ path: rel, content: await readFile(file) });
      }
    }
  }

  // Sort for deterministic output — useful for content-hashing and diffs.
  entries.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));

  const tarBuf = Buffer.concat(
    entries.map((e) => tarEntry(e.path, e.content)).concat([tarEnd()])
  );
  const gz = gzipSync(tarBuf, { level: 9 });

  const outPath = join(root, manifest.artifact);
  await writeFile(outPath, gz);

  const total = entries.reduce((n, e) => n + e.content.length, 0);
  console.log(
    `  ✓ ${manifest.id} → ${manifest.artifact} (${entries.length} files, ${total} B raw / ${gz.length} B gzipped)`
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
