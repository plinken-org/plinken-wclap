// Cap WebAssembly.Memory maxima before any wasm code runs (upstream getWclap
// otherwise reserves 2 GB of shared virtual memory per plugin load).
import './wclap-runtime/cap-wasm-memory';

import ClapAudioNode, {
  type ClapEffectAudioNode,
  type WclapPluginInfo
} from './wclap-runtime/clap-audionode.mjs';
import workletUrl from './wclap-runtime/clap-audioworkletprocessor.mjs?worker&url';
import {
  clearError,
  flashMidiLed,
  getElements,
  setAudioState,
  setCoi,
  setMeters,
  setMidiNotes,
  setMidiStatus,
  setPlugin,
  setSampleRate,
  setStatus,
  showError
} from './ui';

const NUM_SLOTS = 5;
const SHELF_DT_TYPE = 'application/x-plinken-shelf-id';

interface ShelfItem {
  id: string;
  label: string;
  url: string;
  vendor?: string;
  version?: string;
  description?: string | null;
  features?: string[];
  license?: string | null;
  homepage?: string | null;
  source?: string;
  hint?: string;
  ui?: {
    compact_size?: { width: number; height: number };
    expanded_size?: { width: number; height: number };
  };
}

type PanelSize = { width: number; height: number };
type PanelMode = 'compact' | 'expanded';

// Fetched from /shelf.json at boot. Aggregator script
// (apps/wclap-host/scripts/build-shelf.mjs) generates it from every
// `plugins/*/*/plugin.json` plus the external WebCLAP example list.
let SHELF: ShelfItem[] = [];

type SlotSource =
  | { kind: 'url'; url: string }
  | { kind: 'file'; file: File }
  | null;

interface Slot {
  effect: ClapEffectAudioNode | null;
  node: ClapAudioNode | null;
  blobUrl: string | null;
  label: string;
  hasUi: boolean;
  /** All plugins in the loaded bundle (enumerated at load). */
  plugins: WclapPluginInfo[];
  /** Which plugin in `plugins` is currently instantiated as `effect`. */
  pluginIndex: number;
  /** Original source — used to reload with a different pluginIndex on cycle. */
  source: SlotSource;
  /** Manifest-declared panel sizes, if any. Pixel values. */
  compactSize: PanelSize | null;
  expandedSize: PanelSize | null;
}

function emptySlot(): Slot {
  return {
    effect: null,
    node: null,
    blobUrl: null,
    label: '',
    hasUi: false,
    plugins: [],
    pluginIndex: 0,
    source: null,
    compactSize: null,
    expandedSize: null
  };
}

const slots: Slot[] = Array.from({ length: NUM_SLOTS }, emptySlot);

// Master-volume DOM refs — bound by `wireMasterVolume()` during bootstrap.
let masterVolumeSlider: HTMLInputElement | null = null;
let masterVolumeValueEl: HTMLElement | null = null;

interface BaseGraph {
  ctx: AudioContext;
  oscL: OscillatorNode;
  oscR: OscillatorNode;
  inGain: GainNode;
  outGain: GainNode;
  splitter: ChannelSplitterNode;
  analyserL: AnalyserNode;
  analyserR: AnalyserNode;
  meterTimer: number;
}

let baseGraph: BaseGraph | null = null;
let workletReady = false;

const ui = getElements();

setCoi(ui, globalThis.crossOriginIsolated === true);
setAudioState(ui, 'idle (no context)');
setSampleRate(ui, null);
setPlugin(ui, '—');
setStatus(
  ui,
  'Drop plugins into the rack, then press Play for the 440 Hz test tone.'
);

ui.playBtn.disabled = false;
ui.playBtn.addEventListener('click', () => void onPlay());
ui.stopBtn.addEventListener('click', () => void onStop());

renderRack();
wirePluginModal();
wireMasterVolume();
wireShelfUrlLoader();
const proxyReady = registerPluginProxy();
void loadShelf();
void setupWebMidi();

async function loadShelf(): Promise<void> {
  try {
    const res = await fetch('/shelf.json');
    if (!res.ok) {
      console.warn(`[wclap-host] /shelf.json fetch failed: ${res.status}`);
    } else {
      const data = (await res.json()) as { items?: ShelfItem[] };
      if (Array.isArray(data.items)) SHELF = data.items;
    }
  } catch (err) {
    console.warn('[wclap-host] /shelf.json fetch errored', err);
  }
  renderShelf();
}

// ---------------------------------------------------------------------------
// URL-driven shelf entries (ephemeral, not persisted)
// ---------------------------------------------------------------------------
//
// Paste a URL to a `.wclap.tar.gz` or `.wasm` and we add a temporary chip
// to the shelf. `loadIntoSlot` already accepts arbitrary URLs (it fetches
// + sniffs format from bytes), so dragging the chip just works. CORS on
// the remote bucket must allow this origin or the fetch will fail when
// you actually load the chip into a slot.

function wireShelfUrlLoader(): void {
  const input = document.getElementById('shelfUrlInput') as HTMLInputElement | null;
  const btn = document.getElementById('shelfUrlAdd') as HTMLButtonElement | null;
  if (!input || !btn) return;
  const submit = (): void => {
    // Empty field uses the placeholder as the URL — convenient for the
    // one-click "show me what this does" path. The placeholder is the
    // canonical demo bundle hosted in R2.
    const url = input.value.trim() || input.placeholder.trim();
    if (!url) return;
    void addUrlToShelf(url, input, btn);
  };
  btn.addEventListener('click', submit);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    }
  });
}

// Same-origin proxy on the wclap-host worker (and mirrored by a vite dev
// middleware). The page never reads cross-origin URLs directly — plugin
// authors don't have to configure CORS on their own bucket, and the proxy
// preserves COEP via `Cross-Origin-Resource-Policy: cross-origin`.
function proxiedUrl(remote: string): string {
  return `/r2-proxy?u=${encodeURIComponent(remote)}`;
}

async function addUrlToShelf(
  rawUrl: string,
  input: HTMLInputElement,
  btn: HTMLButtonElement
): Promise<void> {
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    showError(ui, new Error(`Invalid URL: ${rawUrl}`));
    return;
  }

  const itemId = `url:${parsed.href}`;
  if (SHELF.some((s) => s.id === itemId)) {
    setStatus(ui, 'That URL is already on the shelf.');
    return;
  }

  // Pre-flight through the proxy so the user finds out about a bad URL /
  // dead bucket / 404 here rather than 4 clicks later. Proxy adds CORS so
  // we always get a response back; failures here mean the remote is
  // genuinely unreachable.
  const probeUrl = proxiedUrl(parsed.href);
  btn.disabled = true;
  try {
    const res = await fetch(probeUrl, { method: 'HEAD' });
    if (!res.ok) {
      throw new Error(`HEAD ${parsed.href}: ${res.status} ${res.statusText}`);
    }
  } catch (err) {
    btn.disabled = false;
    showError(
      ui,
      new Error(
        `Couldn't reach ${parsed.href}: ${
          err instanceof Error ? err.message : String(err)
        }`
      )
    );
    return;
  }
  btn.disabled = false;

  // Synthesize a shelf item. `url` is the proxy URL so loadIntoSlot's
  // fetch() goes through it automatically. `hint` shows the original host
  // on the chip so the user knows where it came from.
  const fileName = parsed.pathname.split('/').pop() ?? parsed.host;
  const label = fileName.replace(/\.(wclap\.tar\.gz|wasm)$/i, '');
  const item: ShelfItem = {
    id: itemId,
    label,
    url: proxiedUrl(parsed.href),
    vendor: parsed.host,
    description: `Loaded from ${parsed.host}`,
    features: ['user-loaded'],
    hint: parsed.host
  };
  SHELF.push(item);
  renderShelf();
  input.value = '';
  setStatus(ui, `Added "${label}" to shelf.`);
}

async function onPlay(): Promise<void> {
  ui.playBtn.disabled = true;
  try {
    const graph = await ensureBaseGraph();
    await graph.ctx.resume();
    setStatus(ui, statusForRunning());
    ui.stopBtn.disabled = false;
  } catch (err) {
    showError(ui, err);
    setStatus(ui, 'Failed to start audio. See error below.');
    ui.playBtn.disabled = false;
  }
}

async function onStop(): Promise<void> {
  if (!baseGraph) return;
  await baseGraph.ctx.suspend();
  setStatus(ui, 'Stopped — press Play to resume.');
  ui.playBtn.disabled = false;
  ui.stopBtn.disabled = true;
}

function statusForRunning(): string {
  const loaded = slots.filter((s) => s.effect).length;
  if (loaded === 0) return 'Playing — 440 Hz tone (no plugin in chain).';
  return `Playing — 440 Hz tone through ${loaded} plugin${loaded === 1 ? '' : 's'}.`;
}

async function ensureBaseGraph(): Promise<BaseGraph> {
  if (baseGraph) return baseGraph;

  const ctx = new AudioContext();
  setSampleRate(ui, ctx.sampleRate);
  wireAudioState(ctx);

  const oscL = ctx.createOscillator();
  const oscR = ctx.createOscillator();
  oscL.frequency.value = 440;
  oscR.frequency.value = 440;
  oscL.type = 'sine';
  oscR.type = 'sine';

  const merger = ctx.createChannelMerger(2);
  oscL.connect(merger, 0, 0);
  oscR.connect(merger, 0, 1);

  const inGain = ctx.createGain();
  // Single attenuation point for the whole chain. -3 dBFS is loud enough to
  // hear without trouble; downstream plugins can push it higher if they have
  // internal makeup gain, but tests so far don't exceed safe levels.
  inGain.gain.value = 0.7;
  merger.connect(inGain);

  const splitter = ctx.createChannelSplitter(2);
  const analyserL = ctx.createAnalyser();
  const analyserR = ctx.createAnalyser();
  analyserL.fftSize = 1024;
  analyserR.fftSize = 1024;
  splitter.connect(analyserL, 0);
  splitter.connect(analyserR, 1);

  // Master output fader. Meters tap PRE-fader (`tail → splitter`) so they
  // show actual plugin output level; this gain stage attenuates only what
  // reaches the speakers. Default 0.5 ≈ -6 dB — safe loudness on first run.
  const outGain = ctx.createGain();
  outGain.gain.value = 0.5;
  outGain.connect(ctx.destination);

  oscL.start();
  oscR.start();
  await ctx.suspend();

  // Peak-meter ballistics: instant attack, ~300 ms exponential release.
  const METER_INTERVAL_MS = 50;
  const METER_RELEASE_MS = 300;
  const RELEASE_COEFF = Math.exp(-METER_INTERVAL_MS / METER_RELEASE_MS);
  let displayedL = 0;
  let displayedR = 0;
  const meterTimer = window.setInterval(() => {
    const live = ctx.state === 'running';
    const targetL = live ? rms(analyserL) : 0;
    const targetR = live ? rms(analyserR) : 0;
    displayedL = targetL >= displayedL ? targetL : displayedL * RELEASE_COEFF;
    displayedR = targetR >= displayedR ? targetR : displayedR * RELEASE_COEFF;
    setMeters(ui, displayedL, displayedR);
  }, METER_INTERVAL_MS);

  baseGraph = {
    ctx,
    oscL,
    oscR,
    inGain,
    outGain,
    splitter,
    analyserL,
    analyserR,
    meterTimer
  };

  // Slider was wired at page load (before any audio context existed);
  // re-apply now so its current dB value lands on the freshly-created node.
  applyMasterVolume();
  rewire();
  return baseGraph;
}

// Master output volume — drives the `outGain` node introduced post-meters.
// Slider value is direct dB (−60..+6). At the floor we mute completely
// (snap to 0 gain) so dragging all the way down behaves like a mute switch
// rather than leaving a barely-audible residual. Wired at page load (before
// any audio context exists) so it's responsive even before first Play; the
// gain assignment is a no-op until `ensureBaseGraph` populates `baseGraph`.
// (Bindings declared up top — `wireMasterVolume` is called during bootstrap,
// before these would otherwise be initialised in lexical order.)

function applyMasterVolume(): void {
  if (!masterVolumeSlider || !masterVolumeValueEl) return;
  const db = parseFloat(masterVolumeSlider.value);
  const min = parseFloat(masterVolumeSlider.min);
  if (db <= min + 0.05) {
    if (baseGraph) baseGraph.outGain.gain.value = 0;
    masterVolumeValueEl.textContent = '−∞ dB';
    return;
  }
  const gain = Math.pow(10, db / 20);
  if (baseGraph) baseGraph.outGain.gain.value = gain;
  masterVolumeValueEl.textContent = `${db >= 0 ? '+' : ''}${db.toFixed(1)} dB`;
}

function wireMasterVolume(): void {
  masterVolumeSlider = document.getElementById('masterVolume') as HTMLInputElement | null;
  masterVolumeValueEl = document.getElementById('masterVolumeValue');
  if (!masterVolumeSlider) return;
  masterVolumeSlider.addEventListener('input', applyMasterVolume);
  applyMasterVolume();
}

async function ensureWorklet(): Promise<void> {
  if (workletReady) return;
  const graph = await ensureBaseGraph();
  await graph.ctx.audioWorklet.addModule(workletUrl);
  workletReady = true;
}

function wireAudioState(ctx: AudioContext): void {
  const maxCh = ctx.destination.maxChannelCount;
  const baseLatencyMs = (ctx.baseLatency ?? 0) * 1000;
  const extra = `out=${maxCh}ch · base latency≈${baseLatencyMs.toFixed(1)}ms`;
  setAudioState(ui, ctx.state, extra);
  ctx.onstatechange = (): void => {
    setAudioState(ui, ctx.state, extra);
  };
}

// Disconnect everything in the active chain and reconnect:
//   inGain → slot1.effect → slot2.effect → … → (splitter, outGain) → destination
// The meter (`splitter`) and master-volume (`outGain`) tap the chain tail
// in parallel — meter shows pre-fader peak so plugin output level is
// visible regardless of monitoring volume; only `outGain` actually reaches
// the speakers. Empty slots are skipped (pass-through). Idempotent — call
// after any slot add/remove/replace.
function rewire(): void {
  if (!baseGraph) return;
  const { inGain, outGain, splitter } = baseGraph;

  try {
    inGain.disconnect();
  } catch {
    // No prior connections — ignore.
  }
  for (const slot of slots) {
    if (slot.effect) {
      try {
        slot.effect.disconnect();
      } catch {
        // Already disconnected — ignore.
      }
      try {
        slot.effect.disconnectEvents?.(null);
      } catch {
        // Plugin may not support events — ignore.
      }
    }
  }

  let tail: AudioNode = inGain;
  let prevEffect: ClapEffectAudioNode | null = null;
  for (const slot of slots) {
    if (!slot.effect) continue;
    tail.connect(slot.effect);
    tail = slot.effect;
    if (prevEffect) {
      try {
        prevEffect.connectEvents?.(slot.effect);
      } catch {
        // Best-effort — not all plugins expose event ports.
      }
    }
    prevEffect = slot.effect;
  }
  tail.connect(splitter);
  tail.connect(outGain);

  refreshPluginSummary();
}

// Switch the slot to the previous/next plugin within its loaded bundle.
// We do a full reload (fresh ClapAudioNode + new wasm memory) rather than
// reusing the existing host node: instantiating a second plugin on the
// same shared-memory host is unreliable — the previous worklet still owns
// the wclap instance and the new `createNode()` can hang or fail silently
// on some bundles.
async function cycleSlot(idx: number, dir: 1 | -1): Promise<void> {
  const slot = slots[idx];
  if (!slot.source || slot.plugins.length < 2) return;

  const len = slot.plugins.length;
  const newIndex = (slot.pluginIndex + dir + len) % len;
  const src =
    slot.source.kind === 'url' ? slot.source.url : slot.source.file;
  await loadIntoSlot(idx, src, undefined, newIndex);
}

function refreshPluginSummary(): void {
  const loaded = slots
    .map((s, i) => (s.effect ? `${i + 1}: ${s.label}` : null))
    .filter((x): x is string => x != null);
  setPlugin(ui, loaded.length === 0 ? '—' : loaded.join(' → '));
}

async function loadIntoSlot(
  idx: number,
  source: File | string,
  displayHint?: string,
  preferPluginIndex = 0
): Promise<void> {
  if (idx < 0 || idx >= slots.length) return;
  clearError(ui);

  const slotEl = ui.rack.querySelector<HTMLElement>(
    `[data-slot-index="${idx}"]`
  );
  slotEl?.classList.add('loading');

  // Track the blob URL outside the try so the catch can revoke it if loading
  // fails before the slot takes ownership.
  let pendingBlobUrl: string | null = null;

  try {
    await ensureWorklet();
    await unloadSlot(idx, { skipRewire: true });

    let buf: ArrayBuffer;
    let displayName: string;
    if (typeof source === 'string') {
      const res = await fetch(source);
      if (!res.ok) {
        throw new Error(`Fetch failed: ${res.status} ${res.statusText}`);
      }
      buf = await res.arrayBuffer();
      displayName = displayHint ?? source.split('/').pop() ?? 'plugin';
    } else {
      buf = await source.arrayBuffer();
      displayName = source.name;
    }

    setStatus(ui, `Loading ${displayName} into slot ${idx + 1}…`);

    const head = new Uint8Array(buf, 0, Math.min(4, buf.byteLength));
    const isWasm =
      head[0] === 0x00 &&
      head[1] === 0x61 &&
      head[2] === 0x73 &&
      head[3] === 0x6d;
    const isGzip = head[0] === 0x1f && head[1] === 0x8b;

    if (!isWasm && !isGzip) {
      const hex = Array.from(head, (b) =>
        b.toString(16).padStart(2, '0')
      ).join(' ');
      throw new Error(
        `Unrecognized bundle format (header: ${hex}). Expected bare \`.wasm\` (00 61 73 6d) or \`.tar.gz\` (1f 8b).`
      );
    }

    // Always route through a blob URL so upstream `getWclap` takes its
    // fetch-based path. The `{ module: ArrayBuffer }` branch in
    // wclap-plugin.mjs has a `guessMemorySize(buffer, module)` bug that
    // references an undefined identifier and explodes for bare wasm.
    const mime = isWasm ? 'application/wasm' : 'application/gzip';
    const blob = new Blob([buf], { type: mime });
    const blobUrl: string = URL.createObjectURL(blob);
    pendingBlobUrl = blobUrl;
    const node: ClapAudioNode = new ClapAudioNode({ url: blobUrl });

    const plugins = await node.plugins();
    if (plugins.length === 0) {
      throw new Error('No CLAP plugins found in bundle.');
    }
    const targetIdx = Math.min(
      Math.max(preferPluginIndex, 0),
      plugins.length - 1
    );
    const target = plugins[targetIdx];
    if (!target) {
      throw new Error('Plugin lookup failed after bounds clamp.');
    }

    const graph = baseGraph;
    if (!graph) throw new Error('Base audio graph missing.');

    const effect = await node.createNode(graph.ctx, target.id ?? null, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [2]
    });

    const descriptor = effect.descriptor;
    const label =
      descriptor.name +
      (descriptor.vendor ? ` · ${descriptor.vendor}` : '');

    // Plugins that follow CLAP semantics call `host.state.mark_dirty()` on
    // every parameter change; the runtime forwards that to main as a
    // `state_mark_dirty` event. We don't persist plugin state yet, so no-op
    // the handler — without it `clap-audionode` logs an "unhandled event"
    // for every pot tweak.
    const effectEvents = (effect as { events?: Record<string, unknown> }).events;
    if (effectEvents) effectEvents.state_mark_dirty = () => {};
    const hasUi =
      typeof (effect as { openInterface?: unknown }).openInterface ===
      'function';

    const shelfItem =
      typeof source === 'string'
        ? SHELF.find((s) => s.url === source)
        : undefined;

    slots[idx] = {
      effect,
      node,
      blobUrl,
      label,
      hasUi,
      plugins,
      pluginIndex: targetIdx,
      source:
        typeof source === 'string'
          ? { kind: 'url', url: source }
          : { kind: 'file', file: source },
      compactSize: shelfItem?.ui?.compact_size ?? null,
      expandedSize: shelfItem?.ui?.expanded_size ?? null
    };
    pendingBlobUrl = null; // ownership transferred to the slot
    rewire();

    setStatus(ui, `Slot ${idx + 1}: ${descriptor.name} ready.`);
    renderRack();
    if (graph.ctx.state === 'running') {
      setStatus(ui, statusForRunning());
    }
  } catch (err) {
    showError(ui, err);
    setStatus(ui, 'Failed to load plugin. See error below.');
  } finally {
    if (pendingBlobUrl) URL.revokeObjectURL(pendingBlobUrl);
    slotEl?.classList.remove('loading');
  }
}

async function unloadSlot(
  idx: number,
  opts: { skipRewire?: boolean } = {}
): Promise<void> {
  const slot = slots[idx];
  if (!slot || !slot.effect) return;

  closePluginUi(idx);
  closeAutoUi(idx);

  // Unroute FIRST so neither audio buffers nor CLAP events can land on
  // the plugin while we're tearing it down. `disconnectEvents(null)` drops
  // all routing entries this node was a source for; `disconnect()` pulls
  // the AudioNode out of the WebAudio graph so `process()` stops being
  // scheduled. After both, the AWP's message handler is the only thing
  // that can still reach the plugin — and the next message we send is
  // destroyPlugin itself.
  try {
    slot.effect.disconnectEvents?.(null);
  } catch {
    // Plugin may not expose event ports — ignore.
  }
  try {
    slot.effect.disconnect();
  } catch {
    // Already disconnected — ignore.
  }

  // Now-quiescent plugin gets the proper CLAP teardown:
  //   stop_processing → deactivate → destroy.
  // The remote method round-trips into the AWP, serialising with anything
  // already in the worklet's message queue.
  const destroyPlugin = (
    slot.effect as { destroyPlugin?: () => Promise<unknown> }
  ).destroyPlugin;
  if (typeof destroyPlugin === 'function') {
    try {
      await destroyPlugin.call(slot.effect);
    } catch (err) {
      console.warn('[wclap-host] destroyPlugin failed', err);
    }
  }

  if (slot.blobUrl) URL.revokeObjectURL(slot.blobUrl);

  slots[idx] = emptySlot();
  if (!opts.skipRewire) {
    rewire();
    if (baseGraph?.ctx.state === 'running') {
      setStatus(ui, statusForRunning());
    } else {
      setStatus(ui, `Slot ${idx + 1} cleared.`);
    }
  }
  renderRack();
}

function renderRack(): void {
  ui.rack.innerHTML = '';
  slots.forEach((slot, idx) => {
    const occupied = slot.effect != null;
    const cls = ['rackSlot', occupied ? 'occupied' : 'empty'];
    if (slot.hasUi) cls.push('hasUi');
    const slotEl = document.createElement('div');
    slotEl.className = cls.join(' ');
    slotEl.dataset.slotIndex = String(idx);

    const num = document.createElement('span');
    num.className = 'slotNum';
    num.textContent = String(idx + 1).padStart(2, '0');
    slotEl.appendChild(num);

    const label = document.createElement('span');
    label.className = 'slotLabel';
    label.textContent = occupied ? slot.label : 'drop a plugin here';
    if (occupied && slot.hasUi) {
      label.title = 'Click to open plugin UI';
      label.addEventListener('click', () => void openPluginUi(idx, 'expanded'));
    }
    slotEl.appendChild(label);

    if (occupied && slot.hasUi) {
      const stripBtn = document.createElement('button');
      stripBtn.type = 'button';
      stripBtn.className = 'slotStrip';
      stripBtn.textContent = 'strip';
      stripBtn.title = 'Open compact (strip) view';
      stripBtn.disabled = !slot.compactSize;
      stripBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        void openPluginUi(idx, 'compact');
      });
      slotEl.appendChild(stripBtn);
    }

    if (occupied) {
      const autoBtn = document.createElement('button');
      autoBtn.type = 'button';
      autoBtn.className = 'slotStrip slotAuto';
      autoBtn.textContent = 'auto';
      autoBtn.title = 'Auto-generated fader UI from the plugin’s params';
      autoBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        void toggleAutoUi(idx);
      });
      slotEl.appendChild(autoBtn);

      const saveBtn = document.createElement('button');
      saveBtn.type = 'button';
      saveBtn.className = 'slotStrip slotState';
      saveBtn.textContent = 'save';
      saveBtn.title = 'Copy plugin state to clipboard (base64)';
      saveBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        void copyStateToClipboard(idx);
      });
      slotEl.appendChild(saveBtn);

      const loadBtn = document.createElement('button');
      loadBtn.type = 'button';
      loadBtn.className = 'slotStrip slotState';
      loadBtn.textContent = 'load';
      loadBtn.title = 'Load plugin state from clipboard (base64)';
      loadBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        void loadStateFromClipboard(idx);
      });
      slotEl.appendChild(loadBtn);
    }

    if (occupied && slot.plugins.length > 1) {
      const cycle = document.createElement('span');
      cycle.className = 'slotCycle';
      const total = slot.plugins.length;

      const prev = document.createElement('button');
      prev.type = 'button';
      prev.className = 'slotCycleBtn';
      prev.textContent = '◀';
      prev.title = 'Previous plugin in bundle';
      prev.setAttribute('aria-label', 'Previous plugin in bundle');
      prev.addEventListener('click', (e) => {
        e.stopPropagation();
        void cycleSlot(idx, -1);
      });

      const count = document.createElement('span');
      count.className = 'slotCycleCount';
      count.textContent = `${slot.pluginIndex + 1}/${total}`;

      const next = document.createElement('button');
      next.type = 'button';
      next.className = 'slotCycleBtn';
      next.textContent = '▶';
      next.title = 'Next plugin in bundle';
      next.setAttribute('aria-label', 'Next plugin in bundle');
      next.addEventListener('click', (e) => {
        e.stopPropagation();
        void cycleSlot(idx, 1);
      });

      cycle.appendChild(prev);
      cycle.appendChild(count);
      cycle.appendChild(next);
      slotEl.appendChild(cycle);
    }

    if (slot.effect) {
      const del = document.createElement('button');
      del.className = 'slotDelete';
      del.type = 'button';
      del.textContent = '✕';
      del.setAttribute(
        'aria-label',
        `Remove plugin from slot ${idx + 1}`
      );
      del.title = 'Remove plugin';
      del.addEventListener('click', (e) => {
        e.stopPropagation();
        void unloadSlot(idx);
      });
      slotEl.appendChild(del);
    }

    slotEl.addEventListener('dragover', (e) => {
      e.preventDefault();
      if (e.dataTransfer) {
        e.dataTransfer.dropEffect = 'copy';
      }
      slotEl.classList.add('dragOver');
    });
    slotEl.addEventListener('dragleave', () => {
      slotEl.classList.remove('dragOver');
    });
    slotEl.addEventListener('drop', (e) => {
      e.preventDefault();
      slotEl.classList.remove('dragOver');

      const shelfId = e.dataTransfer?.getData(SHELF_DT_TYPE);
      if (shelfId) {
        const item = SHELF.find((s) => s.id === shelfId);
        if (item) void loadIntoSlot(idx, item.url, item.label);
        return;
      }

      const file = e.dataTransfer?.files?.[0];
      if (file) void loadIntoSlot(idx, file);
    });

    ui.rack.appendChild(slotEl);
  });
}

function renderShelf(): void {
  ui.shelf.innerHTML = '';
  SHELF.forEach((item) => {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'shelfChip';
    if (item.id.startsWith('url:')) chip.classList.add('shelfChip--remote');
    chip.draggable = true;
    chip.dataset.shelfId = item.id;

    const labelSpan = document.createElement('span');
    labelSpan.textContent = item.label;
    chip.appendChild(labelSpan);
    if (item.hint) {
      const hint = document.createElement('span');
      hint.className = 'shelfChipHint';
      hint.textContent = `· ${item.hint}`;
      chip.appendChild(hint);
    }

    // Mouse / desktop: HTML5 DnD.
    chip.addEventListener('dragstart', (e) => {
      if (!e.dataTransfer) return;
      e.dataTransfer.setData(SHELF_DT_TYPE, item.id);
      e.dataTransfer.effectAllowed = 'copy';
    });

    // Touch / pen: HTML5 DnD doesn't fire on iOS, so do it by hand with
    // Pointer Events. A ghost element follows the finger and on release we
    // hit-test for the slot under it.
    chip.addEventListener('pointerdown', (e) => {
      if (e.pointerType !== 'touch' && e.pointerType !== 'pen') return;
      startTouchDrag(item, e);
    });

    chip.addEventListener('click', () => {
      const idx = slots.findIndex((s) => !s.effect);
      if (idx < 0) {
        setStatus(ui, 'Rack is full — remove a plugin first.');
        return;
      }
      void loadIntoSlot(idx, item.url, item.label);
    });

    ui.shelf.appendChild(chip);
  });
}

const DRAG_THRESHOLD_PX = 6;

function startTouchDrag(item: ShelfItem, startEvent: PointerEvent): void {
  startEvent.preventDefault();

  const startX = startEvent.clientX;
  const startY = startEvent.clientY;
  let started = false;
  let ghost: HTMLElement | null = null;

  const positionGhost = (x: number, y: number): void => {
    if (!ghost) return;
    ghost.style.left = `${x}px`;
    ghost.style.top = `${y}px`;
  };

  const clearSlotHighlights = (): void => {
    document.querySelectorAll('.rackSlot.dragOver').forEach((el) => {
      el.classList.remove('dragOver');
    });
  };

  const highlightSlotUnder = (x: number, y: number): HTMLElement | null => {
    const target = document.elementFromPoint(x, y);
    const slot = target?.closest<HTMLElement>('.rackSlot') ?? null;
    document.querySelectorAll('.rackSlot.dragOver').forEach((el) => {
      if (el !== slot) el.classList.remove('dragOver');
    });
    if (slot) slot.classList.add('dragOver');
    return slot;
  };

  const onMove = (ev: PointerEvent): void => {
    if (!started) {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
      started = true;
      ghost = document.createElement('div');
      ghost.className = 'dragGhost';
      ghost.textContent = item.label;
      document.body.appendChild(ghost);
    }
    positionGhost(ev.clientX, ev.clientY);
    highlightSlotUnder(ev.clientX, ev.clientY);
  };

  const onEnd = (ev: PointerEvent): void => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onEnd);
    window.removeEventListener('pointercancel', onCancel);

    const slot = started ? highlightSlotUnder(ev.clientX, ev.clientY) : null;
    clearSlotHighlights();
    if (ghost) {
      ghost.remove();
      ghost = null;
    }

    if (slot?.dataset.slotIndex != null) {
      const idx = Number.parseInt(slot.dataset.slotIndex, 10);
      if (Number.isFinite(idx)) {
        void loadIntoSlot(idx, item.url, item.label);
      }
    }
  };

  const onCancel = (): void => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onEnd);
    window.removeEventListener('pointercancel', onCancel);
    clearSlotHighlights();
    if (ghost) {
      ghost.remove();
      ghost = null;
    }
  };

  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onEnd);
  window.addEventListener('pointercancel', onCancel);
}

// ---------------------------------------------------------------------------
// Plugin UI modal — top-of-screen overlay holding the plugin's iframe. The
// iframe's resources (HTML, JS, CSS, etc.) are served through a service
// worker that proxies into the slot's in-memory files map.
// ---------------------------------------------------------------------------

const PROXY_PREFIX = '/plugin-proxy';
type ProxyRequest = (path: string) => Promise<ArrayBuffer | null>;
const proxyResolvers = new Map<number, ProxyRequest>();
// One panel per slot. Switching mode (expanded ↔ compact) closes and reopens.
// The underlying runtime (`clap-audionode.mjs`) tracks only one iframe per
// plugin in its closure, so two simultaneous panels for the same slot would
// orphan the first one's message routing.
const openPanels = new Map<number, HTMLElement>();
let panelCascade = 0;

function wirePluginModal(): void {
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || openPanels.size === 0) return;
    const container = document.getElementById('pluginPanels');
    const last = container?.lastElementChild;
    if (last instanceof HTMLElement) {
      const idx = Number.parseInt(last.dataset.slotIndex ?? '', 10);
      if (Number.isFinite(idx)) closePluginUi(idx);
    }
  });
}

async function openPluginUi(
  idx: number,
  mode: PanelMode = 'expanded'
): Promise<void> {
  // The iframe URL falls under the plugin-proxy SW scope. If the iframe is
  // created before the SW activates, the navigation goes to the network and
  // hits the SPA fallback instead of the resolver. Block until the SW is up.
  await proxyReady;

  const slot = slots[idx];
  if (!slot || !slot.effect) return;
  const openInterface = (
    slot.effect as { openInterface?: (opts: unknown) => unknown }
  ).openInterface;
  if (typeof openInterface !== 'function') {
    setStatus(ui, `Slot ${idx + 1}: plugin has no UI to open.`);
    return;
  }

  // One panel per slot. If a panel is already open for this slot and it's
  // already in the requested mode, just bring it to front. If it's the other
  // mode, close it first so the runtime can recreate the iframe at the new size.
  const existing = openPanels.get(idx);
  if (existing) {
    if (existing.dataset.panelMode === mode) {
      bringPanelToFront(existing);
      return;
    }
    closePluginUi(idx);
  }

  // Upstream `getWclap()` rewrites `pluginPath` to `/plugin/<hash>-copy-<hex>`,
  // and keys the files map off that mutated path. The webview URL inside the
  // plugin info still references the original `/plugin/<hash>/...`, so the
  // iframe will ask for the un-mutated path. Compute the prefix mapping once
  // here so each request is an O(1) direct lookup.
  const effectAny = slot.effect as {
    getFile?: (p: string) => Promise<unknown>;
    files?: Record<string, unknown>;
  };
  const sampleKey = effectAny.files ? Object.keys(effectAny.files)[0] : undefined;
  const mutatedPrefix = sampleKey?.match(/^\/plugin\/[^/]+/)?.[0] ?? '';
  const originalPrefix = mutatedPrefix.replace(/-copy-[0-9a-fA-F]+$/, '');

  proxyResolvers.set(idx, async (path) => {
    const getFile = effectAny.getFile;
    if (typeof getFile !== 'function') return null;

    let raw = await getFile(path);
    if (
      !raw &&
      originalPrefix &&
      mutatedPrefix !== originalPrefix &&
      path.startsWith(originalPrefix + '/')
    ) {
      raw = await getFile(mutatedPrefix + path.slice(originalPrefix.length));
    }

    if (raw instanceof ArrayBuffer) return raw;
    if (ArrayBuffer.isView(raw)) {
      const view = raw as ArrayBufferView;
      const out = new ArrayBuffer(view.byteLength);
      new Uint8Array(out).set(
        new Uint8Array(
          view.buffer as ArrayBufferLike,
          view.byteOffset,
          view.byteLength
        )
      );
      return out;
    }
    return null;
  });

  const result = openInterface({
    resourcePrefix: PROXY_PREFIX,
    filePrefix: PROXY_PREFIX,
  });
  if (!(result instanceof HTMLIFrameElement)) {
    setStatus(ui, `Slot ${idx + 1}: plugin UI didn't return an iframe.`);
    proxyResolvers.delete(idx);
    return;
  }

  const container = document.getElementById('pluginPanels');
  if (!container) return;

  const panel = buildPluginPanel(idx, mode, slot.label, result);
  applyPanelSize(panel, modeSize(slot, mode));
  positionPanel(panel);
  container.appendChild(panel);
  openPanels.set(idx, panel);
  wirePanelDrag(panel);
}

// Plugins declare preferred panel sizes via `ui.compact_size` / `ui.expanded_size`
// in `plugin.json`. The host clamps so a runaway value can't escape the viewport,
// and adds the header chrome height on top of the plugin's body height.
const PANEL_MIN_W = 140;
const PANEL_MIN_H = 60;
const PANEL_HEADER_PX = 40;

function modeSize(slot: Slot, mode: PanelMode): PanelSize | null {
  return mode === 'compact' ? slot.compactSize : slot.expandedSize;
}

function applyPanelSize(panel: HTMLElement, size: PanelSize | null): void {
  if (!size) {
    panel.style.width = '';
    panel.style.height = '';
    return;
  }
  const maxW = Math.floor(window.innerWidth * 0.9);
  const maxH = Math.floor(window.innerHeight * 0.85);
  const w = Math.min(Math.max(size.width, PANEL_MIN_W), maxW);
  const h = Math.min(Math.max(size.height + PANEL_HEADER_PX, PANEL_MIN_H), maxH);
  panel.style.width = `${w}px`;
  panel.style.height = `${h}px`;
}

// ---------------------------------------------------------------------------
// Auto-generated fader UI (clap.params)
// ---------------------------------------------------------------------------
//
// Built from `effect.getParams()` — one slider per param. Lives in its own
// panel (separate Map) so it can coexist with the plugin's own iframe UI.

interface ParamInfo {
  id: number;
  name: string;
  min: number;
  max: number;
  default: number;
  flags: number;
}

const autoPanels = new Map<number, HTMLElement>();

async function toggleAutoUi(idx: number): Promise<void> {
  if (autoPanels.has(idx)) {
    closeAutoUi(idx);
    return;
  }
  const slot = slots[idx];
  if (!slot || !slot.effect) return;
  const effect = slot.effect as {
    getParams?: () => Promise<unknown>;
    getParam?: (id: number) => Promise<unknown>;
    setParam?: (id: number, value: number) => Promise<unknown>;
  };
  if (typeof effect.getParams !== 'function') {
    setStatus(ui, `Slot ${idx + 1}: plugin doesn't expose params.`);
    return;
  }

  let params: ParamInfo[] = [];
  try {
    // AWP's `getParams` returns the CBOR-decoded array directly (it also
    // attaches `param.value` to each entry from `getParam`).
    const raw = (await effect.getParams()) as ParamInfo[] | { params?: ParamInfo[] } | undefined;
    if (Array.isArray(raw)) params = raw;
    else if (raw && Array.isArray((raw as { params?: ParamInfo[] }).params)) {
      params = (raw as { params: ParamInfo[] }).params;
    }
  } catch (err) {
    showError(ui, err);
    return;
  }

  const panel = document.createElement('div');
  panel.className = 'pluginPanel pluginPanel--auto';
  panel.dataset.slotIndex = String(idx);
  panel.style.width = '320px';

  const head = document.createElement('div');
  head.className = 'pluginPanelHead';
  const title = document.createElement('span');
  title.className = 'pluginPanelTitle';
  title.textContent = `${slot.label} · params`;
  head.appendChild(title);
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'pluginPanelClose';
  closeBtn.textContent = '×';
  closeBtn.title = 'Close';
  closeBtn.addEventListener('click', () => closeAutoUi(idx));
  head.appendChild(closeBtn);
  panel.appendChild(head);

  const body = document.createElement('div');
  body.className = 'pluginPanelBody pluginPanelBody--auto';

  if (params.length === 0) {
    const note = document.createElement('p');
    note.className = 'autoEmpty';
    note.textContent = 'No parameters.';
    body.appendChild(note);
  } else {
    for (const p of params) {
      const row = buildParamRow(idx, p, effect);
      body.appendChild(row);
    }
  }
  panel.appendChild(body);

  const container = document.getElementById('pluginPanels');
  if (!container) return;
  container.appendChild(panel);
  positionPanel(panel);
  autoPanels.set(idx, panel);
  wirePanelDrag(panel);
}

function buildParamRow(
  _idx: number,
  p: ParamInfo,
  effect: { getParam?: (id: number) => Promise<unknown>; setParam?: (id: number, v: number) => Promise<unknown> }
): HTMLElement {
  const row = document.createElement('label');
  row.className = 'autoParam';

  const head = document.createElement('span');
  head.className = 'autoParamHead';
  const name = document.createElement('span');
  name.className = 'autoParamName';
  name.textContent = p.name || `#${p.id}`;
  const valueEl = document.createElement('span');
  valueEl.className = 'autoParamValue';
  valueEl.textContent = p.default.toFixed(3);
  head.appendChild(name);
  head.appendChild(valueEl);

  const slider = document.createElement('input');
  slider.type = 'range';
  slider.min = String(p.min);
  slider.max = String(p.max);
  // Reasonable default resolution; finer-grained for narrow ranges.
  const span = p.max - p.min;
  slider.step = span > 0 ? String(span / 1000) : '0.001';
  slider.value = String(p.default);

  // Pull the current value once on open so the slider matches plugin state.
  if (typeof effect.getParam === 'function') {
    void effect.getParam(p.id).then((v) => {
      if (typeof v === 'number') {
        slider.value = String(v);
        valueEl.textContent = v.toFixed(3);
      }
    }).catch(() => { /* keep default */ });
  }

  slider.addEventListener('input', () => {
    const v = parseFloat(slider.value);
    valueEl.textContent = v.toFixed(3);
    if (typeof effect.setParam === 'function') void effect.setParam(p.id, v);
  });

  row.appendChild(head);
  row.appendChild(slider);
  return row;
}

function closeAutoUi(idx: number): void {
  const panel = autoPanels.get(idx);
  if (!panel) return;
  panel.remove();
  autoPanels.delete(idx);
}

// ---------------------------------------------------------------------------
// State save / load — round-trip through base64 in the clipboard
// ---------------------------------------------------------------------------
//
// Plugin state is opaque bytes (the plugin's own format). We don't try to
// interpret it — base64 makes the bytes pasteable in any text field, the
// other side decodes and hands the raw buffer back to `effect.loadState`.

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

function base64ToBytes(str: string): Uint8Array {
  const trimmed = str.replace(/\s+/g, '');
  const binary = atob(trimmed);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

async function copyStateToClipboard(idx: number): Promise<void> {
  const slot = slots[idx];
  if (!slot?.effect) return;
  const saveState = (slot.effect as { saveState?: () => Promise<unknown> })
    .saveState;
  if (typeof saveState !== 'function') {
    setStatus(ui, `Slot ${idx + 1}: plugin doesn't support state save.`);
    return;
  }
  try {
    const raw = await saveState.call(slot.effect);
    if (!raw) {
      setStatus(ui, `Slot ${idx + 1}: plugin returned no state.`);
      return;
    }
    const bytes = raw instanceof Uint8Array ? raw : new Uint8Array(raw as ArrayBuffer);
    const b64 = bytesToBase64(bytes);
    await navigator.clipboard.writeText(b64);
    setStatus(ui, `Slot ${idx + 1}: copied ${bytes.byteLength} bytes of state to clipboard.`);
  } catch (err) {
    showError(ui, err);
  }
}

async function loadStateFromClipboard(idx: number): Promise<void> {
  const slot = slots[idx];
  if (!slot?.effect) return;
  const loadState = (
    slot.effect as { loadState?: (b: ArrayBuffer) => Promise<unknown> }
  ).loadState;
  if (typeof loadState !== 'function') {
    setStatus(ui, `Slot ${idx + 1}: plugin doesn't support state load.`);
    return;
  }
  let text: string;
  try {
    text = await navigator.clipboard.readText();
  } catch (err) {
    showError(ui, err);
    return;
  }
  if (!text.trim()) {
    setStatus(ui, `Slot ${idx + 1}: clipboard is empty.`);
    return;
  }
  let bytes: Uint8Array;
  try {
    bytes = base64ToBytes(text);
  } catch (err) {
    setStatus(ui, `Slot ${idx + 1}: clipboard contents aren't valid base64.`);
    return;
  }
  try {
    const ok = await loadState.call(slot.effect, bytes.buffer as ArrayBuffer);
    if (ok) {
      setStatus(ui, `Slot ${idx + 1}: loaded ${bytes.byteLength} bytes of state.`);
    } else {
      setStatus(ui, `Slot ${idx + 1}: plugin rejected the state.`);
    }
  } catch (err) {
    showError(ui, err);
  }
}

function closePluginUi(idx?: number): void {
  if (idx == null) {
    for (const i of Array.from(openPanels.keys())) closePluginUi(i);
    return;
  }
  const panel = openPanels.get(idx);
  if (!panel) return;
  const slot = slots[idx];
  const closeInterface = slot?.effect
    ? (slot.effect as { closeInterface?: () => void }).closeInterface
    : undefined;
  if (typeof closeInterface === 'function') {
    try {
      closeInterface();
    } catch {
      // Best-effort — plugin may already be torn down.
    }
  }
  panel.remove();
  openPanels.delete(idx);
  proxyResolvers.delete(idx);
}

function buildPluginPanel(
  idx: number,
  mode: PanelMode,
  label: string,
  iframe: HTMLIFrameElement
): HTMLElement {
  const panel = document.createElement('div');
  panel.className = `pluginPanel pluginPanel--${mode}`;
  panel.dataset.slotIndex = String(idx);
  panel.dataset.panelMode = mode;

  const header = document.createElement('header');
  header.className = 'pluginPanelHead';

  const title = document.createElement('span');
  title.className = 'pluginPanelTitle';
  title.textContent =
    mode === 'compact' ? label : `${label} · slot ${idx + 1}`;
  header.appendChild(title);

  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'pluginPanelClose';
  close.textContent = '×';
  close.setAttribute('aria-label', 'Close plugin UI');
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    closePluginUi(idx);
  });
  header.appendChild(close);

  const body = document.createElement('div');
  body.className = 'pluginPanelBody';
  body.appendChild(iframe);

  panel.appendChild(header);
  panel.appendChild(body);
  panel.addEventListener('pointerdown', () => bringPanelToFront(panel));

  return panel;
}

function positionPanel(panel: HTMLElement): void {
  const offset = (panelCascade % 6) * 28;
  panel.style.left = `${24 + offset}px`;
  panel.style.top = `${24 + offset}px`;
  panelCascade += 1;
}

function bringPanelToFront(panel: HTMLElement): void {
  const container = panel.parentElement;
  if (container && container.lastElementChild !== panel) {
    container.appendChild(panel);
  }
}

function wirePanelDrag(panel: HTMLElement): void {
  const header = panel.querySelector<HTMLElement>('.pluginPanelHead');
  if (!header) return;

  let dragging = false;
  let startX = 0;
  let startY = 0;
  let startLeft = 0;
  let startTop = 0;

  header.addEventListener('pointerdown', (e) => {
    if (
      e.target instanceof Element &&
      e.target.closest('.pluginPanelClose')
    ) {
      return;
    }
    dragging = true;
    startX = e.clientX;
    startY = e.clientY;
    const rect = panel.getBoundingClientRect();
    startLeft = rect.left;
    startTop = rect.top;
    try {
      header.setPointerCapture(e.pointerId);
    } catch {
      // Some browsers reject if the pointer isn't active — ignore.
    }
    bringPanelToFront(panel);
  });

  header.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    panel.style.left = `${Math.max(0, startLeft + dx)}px`;
    panel.style.top = `${Math.max(0, startTop + dy)}px`;
  });

  const end = (e: PointerEvent): void => {
    if (!dragging) return;
    dragging = false;
    try {
      header.releasePointerCapture(e.pointerId);
    } catch {
      // Pointer already released — ignore.
    }
  };
  header.addEventListener('pointerup', end);
  header.addEventListener('pointercancel', end);
}

async function registerPluginProxy(): Promise<void> {
  if (!('serviceWorker' in navigator)) return;
  try {
    const reg = await navigator.serviceWorker.register('/plugin-proxy-sw.js', {
      scope: '/plugin-proxy/'
    });
    // `navigator.serviceWorker.ready` only resolves for a SW that controls
    // *this* document. Our SW is scoped to /plugin-proxy/ but the page is at
    // /, so `.ready` never fires. Wait on the registration's own state.
    if (!reg.active) {
      const sw = reg.installing ?? reg.waiting;
      if (sw) {
        await new Promise<void>((resolve) => {
          const check = () => {
            if (sw.state === 'activated') resolve();
          };
          sw.addEventListener('statechange', check);
          check();
        });
      }
    }
  } catch (err) {
    console.warn('[wclap-host] plugin-proxy SW registration failed', err);
    return;
  }

  navigator.serviceWorker.addEventListener('message', (event) => {
    const data = event.data as
      | { type: string; id: number; path: string }
      | undefined;
    if (!data || data.type !== 'plugin-proxy-request') return;
    const source = event.source;
    if (!(source instanceof ServiceWorker)) return;
    void respondProxy(source, data.id, data.path);
  });
}

async function respondProxy(
  source: ServiceWorker,
  id: number,
  path: string
): Promise<void> {
  const body = await lookupProxyFile(path);
  const mime = body ? mimeForPath(path) : undefined;
  source.postMessage({ type: 'plugin-proxy-response', id, body, mime });
}

async function lookupProxyFile(path: string): Promise<ArrayBuffer | null> {
  // Any active slot can serve a path; whichever has it wins. In practice the
  // path's `/plugin/<hash>/...` prefix is unique per loaded plugin, so this
  // doesn't collide across slots.
  for (const [, resolver] of proxyResolvers) {
    try {
      const buf = await resolver(path);
      if (buf) return buf;
    } catch {
      // Try next slot.
    }
  }
  return null;
}

const MIME_TABLE: Record<string, string> = {
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

function mimeForPath(path: string): string {
  const m = /\.([a-z0-9]+)(?:[?#].*)?$/i.exec(path);
  const ext = m?.[1]?.toLowerCase() ?? '';
  return MIME_TABLE[ext] ?? 'application/octet-stream';
}

const rmsScratch = new Float32Array(1024);
function rms(analyser: AnalyserNode): number {
  const len = Math.min(rmsScratch.length, analyser.fftSize);
  analyser.getFloatTimeDomainData(rmsScratch);
  let sum = 0;
  for (let i = 0; i < len; i++) {
    const v = rmsScratch[i] ?? 0;
    sum += v * v;
  }
  return Math.sqrt(sum / len);
}

// ---------------------------------------------------------------------------
// Web MIDI → CLAP note events
// ---------------------------------------------------------------------------
//
// Translates incoming MIDI note on/off into `clap_event_note` byte buffers
// and pushes them into every occupied slot's `effect.acceptEvent` (added as
// a remoteMethod in `clap-audioworkletprocessor.mjs`). Each plugin decides
// what to do with the event; instruments produce sound, effects ignore or
// pass through.

const CLAP_EVENT_NOTE_ON = 0;
const CLAP_EVENT_NOTE_OFF = 1;
const CLAP_EVENT_MIDI = 10;
const CLAP_CORE_EVENT_SPACE_ID = 0;

// clap_event_note: header(16) + note_id(4) + port_index(2) + channel(2) +
// key(2) + pad(6, double 8-align) + velocity(8) = 40 bytes.
const CLAP_EVENT_NOTE_SIZE = 40;
// clap_event_midi: header(16) + port_index(2) + data[3] + pad(3) = 24.
const CLAP_EVENT_MIDI_SIZE = 24;

function encodeClapNoteEvent(
  isOn: boolean,
  channel: number,
  key: number,
  velocity: number
): ArrayBuffer {
  const buf = new ArrayBuffer(CLAP_EVENT_NOTE_SIZE);
  const dv = new DataView(buf);
  // header
  dv.setUint32(0, CLAP_EVENT_NOTE_SIZE, true); // size
  dv.setUint32(4, 0, true); // time (sample offset within block; 0 = block start)
  dv.setUint16(8, CLAP_CORE_EVENT_SPACE_ID, true); // space_id
  dv.setUint16(10, isOn ? CLAP_EVENT_NOTE_ON : CLAP_EVENT_NOTE_OFF, true); // type
  dv.setUint32(12, 1, true); // flags = CLAP_EVENT_IS_LIVE
  // body
  dv.setInt32(16, -1, true); // note_id (-1 = unspecified)
  dv.setInt16(20, 0, true); // port_index
  dv.setInt16(22, channel, true); // channel
  dv.setInt16(24, key, true); // key
  // bytes 26..32 are alignment padding for the double
  dv.setFloat64(32, velocity, true); // velocity (0.0..1.0)
  return buf;
}

function encodeClapMidiEvent(midiBytes: Uint8Array): ArrayBuffer {
  // Pass-through wrapper for any 1–3-byte channel-voice message (CC, pitch
  // bend, aftertouch). Plugins that consume MIDI dispatch on header.type =
  // CLAP_EVENT_MIDI and read the raw bytes from `data[3]`.
  const buf = new ArrayBuffer(CLAP_EVENT_MIDI_SIZE);
  const dv = new DataView(buf);
  dv.setUint32(0, CLAP_EVENT_MIDI_SIZE, true); // size
  dv.setUint32(4, 0, true); // time = block start
  dv.setUint16(8, CLAP_CORE_EVENT_SPACE_ID, true);
  dv.setUint16(10, CLAP_EVENT_MIDI, true);
  dv.setUint32(12, 1, true); // flags = CLAP_EVENT_IS_LIVE
  dv.setUint16(16, 0, true); // port_index
  dv.setUint8(18, midiBytes[0] ?? 0);
  dv.setUint8(19, midiBytes[1] ?? 0);
  dv.setUint8(20, midiBytes[2] ?? 0);
  return buf;
}

function fanoutEvent(buf: ArrayBuffer): void {
  for (const slot of slots) {
    const accept = (
      slot.effect as { acceptEvent?: (b: ArrayBuffer) => unknown } | null
    )?.acceptEvent;
    if (typeof accept === 'function') accept.call(slot.effect, buf);
  }
}

function panicAllNotesOff(): void {
  // Send a NOTE_OFF for every key on channel 0 so any stuck note inside
  // a plugin gets a release event. Then drop our held-state and refresh
  // the display. Channel 0 is enough for our M4 plugins — they don't
  // multi-channel route yet.
  for (let key = 0; key < 128; key++) {
    fanoutEvent(encodeClapNoteEvent(false, 0, key, 0));
  }
  heldNotes.clear();
  refreshMidiNotesUi();
  flashMidiLed(ui);
}
ui.midiPanic.addEventListener('click', panicAllNotesOff);

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
function midiNoteName(key: number): string {
  const name = NOTE_NAMES[key % 12] ?? '?';
  const octave = Math.floor(key / 12) - 1;
  return `${name}${octave}`;
}

// Held notes (currently down) keyed by `${channel}:${key}`. On note-off
// the entry is removed; the display reflects only what's actually held.
const heldNotes = new Map<string, { key: number; vel: number }>();

function refreshMidiNotesUi(): void {
  const held = [...heldNotes.values()]
    .map((n) => `${midiNoteName(n.key)}·${Math.round(n.vel * 127)}`)
    .join(' ');
  setMidiNotes(ui, held);
}

function refreshMidiInputsLabel(access: MIDIAccess): void {
  const names = [...access.inputs.values()].map((i) => i.name ?? 'input');
  setMidiStatus(ui, names.length ? names.join(', ') : 'no device');
}

async function setupWebMidi(): Promise<void> {
  if (typeof navigator === 'undefined' || !navigator.requestMIDIAccess) {
    setMidiStatus(ui, 'Web MIDI unsupported');
    return;
  }
  let access: MIDIAccess;
  try {
    access = await navigator.requestMIDIAccess({ sysex: false });
  } catch (err) {
    console.warn('[wclap-host] MIDI access denied', err);
    setMidiStatus(ui, 'permission denied');
    return;
  }

  const wire = (input: MIDIInput): void => {
    input.onmidimessage = (ev) => {
      flashMidiLed(ui); // any MIDI byte stream blinks the LED
      const data = ev.data;
      if (!data || data.length < 1) return;
      const status = data[0] ?? 0;
      const high = status & 0xf0;
      const channel = status & 0x0f;
      const key = data[1] ?? 0;
      const velRaw = data[2] ?? 0;
      // Note-on with velocity 0 is conventionally a note-off.
      if (high === 0x90 && velRaw > 0) {
        const velocity = velRaw / 127;
        heldNotes.set(`${channel}:${key}`, { key, vel: velocity });
        refreshMidiNotesUi();
        fanoutEvent(encodeClapNoteEvent(true, channel, key, velocity));
      } else if (high === 0x80 || (high === 0x90 && velRaw === 0)) {
        if (heldNotes.delete(`${channel}:${key}`)) refreshMidiNotesUi();
        fanoutEvent(encodeClapNoteEvent(false, channel, key, velRaw / 127));
      } else if (
        high === 0xb0 || // CC
        high === 0xe0 || // pitch bend
        high === 0xd0 || // channel aftertouch
        high === 0xa0 || // poly aftertouch
        high === 0xc0    // program change (rare but harmless)
      ) {
        fanoutEvent(encodeClapMidiEvent(data));
      }
      // SysEx (0xF0) is multi-byte; would need clap_event_midi_sysex —
      // skip for now.
    };
  };

  for (const input of access.inputs.values()) wire(input);
  refreshMidiInputsLabel(access);
  access.onstatechange = (ev) => {
    const port = (ev as MIDIConnectionEvent).port;
    if (port instanceof MIDIInput && port.state === 'connected') wire(port);
    refreshMidiInputsLabel(access);
  };

  const count = access.inputs.size;
  if (count > 0) {
    console.log(`[wclap-host] Web MIDI: ${count} input(s) listening`);
  }
}
