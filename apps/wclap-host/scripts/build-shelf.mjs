// Tier-2 aggregator. Walks `plugins/*/*/plugin.json`, validates each manifest,
// copies the built artifact into `apps/wclap-host/public/samples/`, and emits
// a single `shelf.json` the host fetches at runtime. External WebCLAP example
// bundles (Signalsmith, clack, as-clap) are appended unchanged — they don't
// live under `plugins/` because they're not authored in this repo.
//
// Run automatically via the wclap-host build script. Run on its own with:
//   node apps/wclap-host/scripts/build-shelf.mjs

import { readdir, readFile, copyFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const HOST_ROOT = resolve(here, '..');
const REPO_ROOT = resolve(HOST_ROOT, '../..');
const PLUGINS_ROOT = resolve(REPO_ROOT, 'plugins');
const SAMPLES_DIR = resolve(HOST_ROOT, 'public/samples');
const SHELF_PATH = resolve(HOST_ROOT, 'public/shelf.json');

// External WebCLAP examples already shipped under public/samples/. These are
// upstream bundles, not built from this repo — they pass through verbatim.
// `source` links back to where the original lives so the shelf can credit
// each plugin.
const EXTERNAL_EXAMPLES = [
  {
    id: 'signalsmith-basics',
    label: 'Signalsmith Basics',
    url: '/samples/signalsmith-basics.wclap.tar.gz',
    vendor: 'Signalsmith Audio',
    license: 'MIT',
    source:
      'https://github.com/WebCLAP/examples/tree/main/signalsmith-basics'
  },
  {
    id: 'signalsmith-clap-cpp',
    label: 'Signalsmith CLAP C++',
    url: '/samples/signalsmith-clap-cpp.wclap.tar.gz',
    vendor: 'Signalsmith Audio',
    license: 'MIT',
    source:
      'https://github.com/WebCLAP/examples/tree/main/signalsmith-clap-cpp'
  },
  {
    id: 'clack-gain',
    label: 'clack: gain',
    url: '/samples/clack-gain.wasm',
    vendor: 'clack',
    license: 'MIT OR Apache-2.0',
    source: 'https://github.com/WebCLAP/examples/tree/main/clack'
  },
  {
    id: 'clack-polysynth',
    label: 'clack: polysynth',
    url: '/samples/clack-polysynth.wasm',
    hint: 'needs MIDI',
    vendor: 'clack',
    license: 'MIT OR Apache-2.0',
    source: 'https://github.com/WebCLAP/examples/tree/main/clack'
  },
  {
    id: 'as-clap',
    label: 'as-clap: example',
    url: '/samples/as-clap-example.wclap.wasm',
    vendor: 'as-clap',
    license: 'BSL-1.0',
    source: 'https://github.com/WebCLAP/examples/tree/main/as-clap'
  }
];

const REPO_TREE_BASE =
  'https://github.com/taluvi-dev/plinken-org/tree/main/plugins';

const REQUIRED = ['id', 'name', 'vendor', 'version', 'artifact', 'format'];
const VALID_FORMATS = new Set(['wasm', 'tar.gz']);

async function main() {
  const items = [];
  let errors = 0;

  if (existsSync(PLUGINS_ROOT)) {
    const vendors = await readdir(PLUGINS_ROOT, { withFileTypes: true });
    for (const vendor of vendors) {
      if (!vendor.isDirectory()) continue;
      const vendorPath = join(PLUGINS_ROOT, vendor.name);
      const plugins = await readdir(vendorPath, { withFileTypes: true });
      for (const plugin of plugins) {
        if (!plugin.isDirectory()) continue;
        const pluginPath = join(vendorPath, plugin.name);
        const manifestPath = join(pluginPath, 'plugin.json');
        if (!existsSync(manifestPath)) continue;

        const slug = `${vendor.name}/${plugin.name}`;
        let manifest;
        try {
          manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
        } catch (err) {
          console.error(`  ✗ ${slug}: invalid JSON in plugin.json — ${err.message}`);
          errors += 1;
          continue;
        }

        const missing = REQUIRED.filter((k) => manifest[k] == null);
        if (missing.length) {
          console.error(`  ✗ ${slug}: missing manifest fields: ${missing.join(', ')}`);
          errors += 1;
          continue;
        }
        if (!VALID_FORMATS.has(manifest.format)) {
          console.error(`  ✗ ${slug}: format must be 'wasm' or 'tar.gz', got '${manifest.format}'`);
          errors += 1;
          continue;
        }

        const artifactPath = join(pluginPath, manifest.artifact);
        if (!existsSync(artifactPath)) {
          console.error(
            `  ✗ ${slug}: artifact ${manifest.artifact} not found — build the plugin first (pnpm --filter ${manifest.id} build, or run pnpm build in its folder)`
          );
          errors += 1;
          continue;
        }

        const ext = manifest.format === 'tar.gz' ? '.wclap.tar.gz' : '.wclap.wasm';
        const sampleFile = `${manifest.id}${ext}`;
        const sampleDest = join(SAMPLES_DIR, sampleFile);
        await copyFile(artifactPath, sampleDest);

        items.push({
          id: manifest.id,
          label: `${manifest.vendor}: ${manifest.name}`,
          url: `/samples/${sampleFile}`,
          vendor: manifest.vendor,
          version: manifest.version,
          description: manifest.description ?? null,
          features: manifest.features ?? [],
          license: manifest.license ?? null,
          homepage: manifest.homepage ?? null,
          source: `${REPO_TREE_BASE}/${vendor.name}/${plugin.name}`,
          ...(manifest.hint ? { hint: manifest.hint } : {})
        });
        console.log(`  ✓ ${slug} → /samples/${sampleFile}`);
      }
    }
  } else {
    console.warn(`No plugins/ directory at ${PLUGINS_ROOT}, skipping vendor scan`);
  }

  for (const ex of EXTERNAL_EXAMPLES) items.push(ex);

  const out = {
    generatedAt: new Date().toISOString(),
    items
  };
  await writeFile(SHELF_PATH, JSON.stringify(out, null, 2) + '\n');
  console.log(`\nWrote ${items.length} item${items.length === 1 ? '' : 's'} to ${SHELF_PATH}`);

  if (errors > 0) {
    console.error(`\n${errors} plugin manifest error${errors === 1 ? '' : 's'} — aborting build.`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
