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
  getElements,
  setAudioState,
  setCoi,
  setMeters,
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
}

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
    source: null
  };
}

const slots: Slot[] = Array.from({ length: NUM_SLOTS }, emptySlot);

interface BaseGraph {
  ctx: AudioContext;
  oscL: OscillatorNode;
  oscR: OscillatorNode;
  inGain: GainNode;
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
void registerPluginProxy();
void loadShelf();

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
    splitter,
    analyserL,
    analyserR,
    meterTimer
  };

  rewire();
  return baseGraph;
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
//   inGain → slot1.effect → slot2.effect → … → (splitter, destination)
// Plus route CLAP events (MIDI/note) along the same order, so a keyboard
// plugin upstream can drive a synth plugin downstream. Empty slots are
// skipped (pass-through). Idempotent — call after any slot add/remove/
// replace.
function rewire(): void {
  if (!baseGraph) return;
  const { inGain, splitter, ctx } = baseGraph;

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
  tail.connect(ctx.destination);

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
    const hasUi =
      typeof (effect as { openInterface?: unknown }).openInterface ===
      'function';

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
          : { kind: 'file', file: source }
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

  try {
    slot.effect.disconnect();
  } catch {
    // Already disconnected — ignore.
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
      label.addEventListener('click', () => openPluginUi(idx));
    }
    slotEl.appendChild(label);

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

function openPluginUi(idx: number): void {
  const slot = slots[idx];
  if (!slot || !slot.effect) return;
  const openInterface = (
    slot.effect as { openInterface?: (opts: unknown) => unknown }
  ).openInterface;
  if (typeof openInterface !== 'function') {
    setStatus(ui, `Slot ${idx + 1}: plugin has no UI to open.`);
    return;
  }

  const existing = openPanels.get(idx);
  if (existing) {
    bringPanelToFront(existing);
    return;
  }

  // Register this slot as the proxy source for any plugin-proxy paths it owns.
  proxyResolvers.set(idx, async (path) => {
    const getFile = (slot.effect as { getFile?: (p: string) => Promise<unknown> })
      .getFile;
    if (typeof getFile !== 'function') return null;

    // Upstream `getWclap()` mutates `pluginPath` on its second pass, appending
    // `-copy-<random>` — but the files map is keyed off the original prefix.
    // Try the literal path first, then strip the `-copy-<hex>` segment.
    let raw = await getFile(path);
    if (!raw) {
      const stripped = path.replace(/-copy-[0-9a-fA-F]+(?=\/)/, '');
      if (stripped !== path) raw = await getFile(stripped);
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

  const result = openInterface({ resourcePrefix: PROXY_PREFIX });
  if (!(result instanceof HTMLIFrameElement)) {
    setStatus(ui, `Slot ${idx + 1}: plugin UI didn't return an iframe.`);
    proxyResolvers.delete(idx);
    return;
  }

  const container = document.getElementById('pluginPanels');
  if (!container) return;

  const panel = buildPluginPanel(idx, slot.label, result);
  positionPanel(panel);
  container.appendChild(panel);
  openPanels.set(idx, panel);
  wirePanelDrag(panel);
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
  label: string,
  iframe: HTMLIFrameElement
): HTMLElement {
  const panel = document.createElement('div');
  panel.className = 'pluginPanel';
  panel.dataset.slotIndex = String(idx);

  const header = document.createElement('header');
  header.className = 'pluginPanelHead';

  const title = document.createElement('span');
  title.className = 'pluginPanelTitle';
  title.textContent = `${label} · slot ${idx + 1}`;
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
    await navigator.serviceWorker.register('/plugin-proxy-sw.js', {
      scope: '/plugin-proxy/'
    });
    await navigator.serviceWorker.ready;
  } catch (err) {
    console.warn('[wclap-host] plugin-proxy SW registration failed', err);
    return;
  }

  navigator.serviceWorker.addEventListener('message', (event) => {
    const data = event.data as
      | { type: string; id: number; path: string }
      | undefined;
    if (!data || data.type !== 'plugin-proxy-request') return;
    // The top-level page registered the SW but lives outside the SW scope,
    // so `navigator.serviceWorker.controller` is null. Reply via the source
    // of the message (the SW itself), which works regardless of control.
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
