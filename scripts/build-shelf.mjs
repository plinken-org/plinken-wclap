// Aggregator: walks `plugins/*/*/plugin.json`, validates each manifest,
// copies the built artifact into BOTH consumers' static directories, and
// emits a `shelf.json` for each. Two paths because CF Workers on the same
// account can't sub-request each other, so the proxy approach doesn't
// work — we duplicate the bytes at build time instead.
//
//   apps/wclap-host/public/samples/<id>.<ext>   served from wclap.plinken.org
//   apps/wclap-host/public/shelf.json           URLs: /samples/<id>.<ext>
//
//   apps/site/static/wclap/<id>.<ext>           served from plinken.org (apps/site)
//   apps/site/static/shelf.json                 URLs: /wclap/<id>.<ext>
//
// External WebCLAP example bundles (Signalsmith, clack, as-clap) get
// appended verbatim; they don't live under `plugins/` because they're not
// authored in this repo.

import { readdir, readFile, copyFile, writeFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(here, '..');
const PLUGINS_ROOT = resolve(REPO_ROOT, 'plugins');

const HOST_SAMPLES = resolve(REPO_ROOT, 'apps/wclap-host/public/samples');
const HOST_SHELF = resolve(REPO_ROOT, 'apps/wclap-host/public/shelf.json');

const SITE_WCLAP = resolve(REPO_ROOT, 'apps/site/static/wclap');
const SITE_SHELF = resolve(REPO_ROOT, 'apps/site/static/shelf.json');

const REPO_TREE_BASE =
  'https://github.com/taluvi-dev/plinken-org/tree/main/plugins';

const EXTERNAL_EXAMPLES = [
  {
    id: 'signalsmith-basics',
    label: 'Signalsmith Basics',
    file: 'signalsmith-basics.wclap.tar.gz',
    vendor: 'Signalsmith Audio',
    license: 'MIT',
    source:
      'https://github.com/WebCLAP/examples/tree/main/signalsmith-basics'
  },
  {
    id: 'signalsmith-clap-cpp',
    label: 'Signalsmith CLAP C++',
    file: 'signalsmith-clap-cpp.wclap.tar.gz',
    vendor: 'Signalsmith Audio',
    license: 'MIT',
    source:
      'https://github.com/WebCLAP/examples/tree/main/signalsmith-clap-cpp'
  },
  {
    id: 'clack-gain',
    label: 'clack: gain',
    file: 'clack-gain.wasm',
    vendor: 'clack',
    license: 'MIT OR Apache-2.0',
    source: 'https://github.com/WebCLAP/examples/tree/main/clack'
  },
  {
    id: 'clack-polysynth',
    label: 'clack: polysynth',
    file: 'clack-polysynth.wasm',
    hint: 'needs MIDI',
    vendor: 'clack',
    license: 'MIT OR Apache-2.0',
    source: 'https://github.com/WebCLAP/examples/tree/main/clack'
  },
  {
    id: 'as-clap',
    label: 'as-clap: example',
    file: 'as-clap-example.wclap.wasm',
    vendor: 'as-clap',
    license: 'BSL-1.0',
    source: 'https://github.com/WebCLAP/examples/tree/main/as-clap'
  }
];

const REQUIRED = ['id', 'name', 'vendor', 'version', 'artifact', 'format'];
const VALID_FORMATS = new Set(['wasm', 'tar.gz']);

async function main() {
  // Each entry collected before being projected onto host- vs site-style URLs.
  // `file` is the bare filename inside the consumer's directory.
  const entries = [];
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
            `  ✗ ${slug}: artifact ${manifest.artifact} missing — build the plugin first (pnpm --filter ${manifest.id} build)`
          );
          errors += 1;
          continue;
        }

        const ext = manifest.format === 'tar.gz' ? '.wclap.tar.gz' : '.wclap.wasm';
        const file = `${manifest.id}${ext}`;

        entries.push({
          file,
          srcPath: artifactPath,
          id: manifest.id,
          label: `${manifest.vendor}: ${manifest.name}`,
          vendor: manifest.vendor,
          version: manifest.version,
          description: manifest.description ?? null,
          features: manifest.features ?? [],
          license: manifest.license ?? null,
          homepage: manifest.homepage ?? null,
          source: `${REPO_TREE_BASE}/${vendor.name}/${plugin.name}`,
          hint: manifest.hint ?? null
        });
      }
    }
  } else {
    console.warn(`No plugins/ directory at ${PLUGINS_ROOT}, skipping vendor scan`);
  }

  // Layer the external upstream examples on top. Their bytes are already in
  // apps/wclap-host/public/samples/ (committed); we just record where to find
  // them so we can also mirror to apps/site/static/wclap/.
  for (const ex of EXTERNAL_EXAMPLES) {
    entries.push({
      file: ex.file,
      srcPath: join(HOST_SAMPLES, ex.file),
      id: ex.id,
      label: ex.label,
      vendor: ex.vendor,
      version: null,
      description: null,
      features: [],
      license: ex.license,
      homepage: null,
      source: ex.source,
      hint: ex.hint ?? null
    });
  }

  // Make sure the destination directories exist before we start copying.
  await mkdir(HOST_SAMPLES, { recursive: true });
  await mkdir(SITE_WCLAP, { recursive: true });

  for (const e of entries) {
    if (!existsSync(e.srcPath)) {
      console.error(`  ✗ ${e.id}: missing source bytes at ${e.srcPath}`);
      errors += 1;
      continue;
    }
    await copyFile(e.srcPath, join(HOST_SAMPLES, e.file));
    await copyFile(e.srcPath, join(SITE_WCLAP, e.file));
    console.log(`  ✓ ${e.id} → ${e.file}`);
  }

  // Two views of the same catalog. URLs differ by host-side path convention.
  const hostItems = entries.map((e) => projectItem(e, `/samples/${e.file}`));
  const siteItems = entries.map((e) => projectItem(e, `/wclap/${e.file}`));

  const generatedAt = new Date().toISOString();
  await writeFile(
    HOST_SHELF,
    JSON.stringify({ generatedAt, items: hostItems }, null, 2) + '\n'
  );
  await writeFile(
    SITE_SHELF,
    JSON.stringify({ generatedAt, items: siteItems }, null, 2) + '\n'
  );

  console.log(`\nWrote ${entries.length} item${entries.length === 1 ? '' : 's'} to:`);
  console.log(`  ${HOST_SHELF}`);
  console.log(`  ${SITE_SHELF}`);

  if (errors > 0) {
    console.error(`\n${errors} error${errors === 1 ? '' : 's'} — aborting build.`);
    process.exit(1);
  }
}

function projectItem(entry, url) {
  const item = {
    id: entry.id,
    label: entry.label,
    url,
    vendor: entry.vendor,
    license: entry.license,
    source: entry.source
  };
  if (entry.version) item.version = entry.version;
  if (entry.description) item.description = entry.description;
  if (entry.features?.length) item.features = entry.features;
  if (entry.homepage) item.homepage = entry.homepage;
  if (entry.hint) item.hint = entry.hint;
  return item;
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
