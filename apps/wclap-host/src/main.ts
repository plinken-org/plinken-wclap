// Plinken vocal-host V2 — main thread orchestrator.
//
// Owns:
//   - AudioContext + base graph (source → inGain → chainNode → meters → outGain)
//   - ONE chain AudioWorkletNode that runs every loaded plugin internally
//   - 5-slot rack UI with drag-drop + click-to-load
//   - Plugin iframe panels via the /plugin-proxy/ service worker
//
// Compare with apps/wclap-host/src/main.ts — that one wires one AWN per
// plugin. Here every plugin lives inside the single chain worklet, so
// `rewire()` is unnecessary: the audio graph topology is static.

// @ts-expect-error — vendored JS module without types.
import { getHost, startHost } from '@webclap/wclap-host-js';
import workletUrl from './wclap-runtime/chain-worklet.mjs?worker&url';
import { fetchWclap, type WclapManifest } from './plugin-loader';
import { IframeBridge } from './iframe-bridge';
import {
  getElements,
  setStatus,
  setPlugin,
  setSampleRate,
  setCoi,
  setAudioState,
  showError,
  clearError,
  setMeters,
  flashMidiLed,
  setMidiStatus,
  setMidiNotes
} from './ui';

// ---------------------------------------------------------------------------
// Types & state
// ---------------------------------------------------------------------------

const NUM_SLOTS = 5;
const SHELF_DT_TYPE = 'application/x-plinken-shelf-id';
const PROXY_PREFIX = '/plugin-proxy';
/** Tone-source gain when active. 10^(-10/20) ≈ -10 dBFS peak into the chain. */
const TONE_LEVEL_GAIN = 0.31623;

interface ShelfItem {
  id: string;
  /** Display label (build-shelf.mjs writes `label`; we also accept `name`). */
  label?: string;
  name?: string;
  vendor?: string;
  category?: string;
  url: string;
  hint?: string;
  manifest?: WclapManifest;
}

interface BundlePlugin {
  id: string;
  name?: string;
  vendor?: string;
}

interface Slot {
  index: number;
  url: string | null;
  pluginId: string | null;
  manifest: WclapManifest | null;
  files: Record<string, ArrayBuffer> | null;
  label: string;
  bypass: boolean;
  iframe: HTMLIFrameElement | null;
  panel: HTMLElement | null;
  /** All plugins in this slot's bundle — surfaced by the worklet at load. */
  plugins: BundlePlugin[];
  /** Index into `plugins` for the currently active plugin. */
  pluginIndex: number;
  /**
   * URI the plugin's `webview.get_uri` returned (file:/plugin/<hash>/...).
   * Null when the plugin doesn't advertise the `clap.webview/3` extension —
   * authoritative signal that the plugin has no UI.
   */
  webviewUri: string | null;
  /** pluginPath the loader gave wclap-host-js (e.g. /plugin/<hash>). */
  pluginPath: string | null;
  /** Latency reported by the plugin's clap.latency extension (samples). */
  latencySamples: number | null;
}

const slots: Slot[] = Array.from({ length: NUM_SLOTS }, (_, i) => ({
  index: i,
  url: null,
  pluginId: null,
  manifest: null,
  files: null,
  label: '',
  bypass: false,
  iframe: null,
  panel: null,
  plugins: [],
  pluginIndex: 0,
  webviewUri: null,
  pluginPath: null,
  latencySamples: null
}));

let SHELF: ShelfItem[] = [];
let baseGraph: BaseGraph | null = null;
let chainNode: AudioWorkletNode | null = null;
let workletReady = false;
let hostReadyResolve: (() => void) | null = null;
const hostReadyPromise = new Promise<void>((r) => (hostReadyResolve = r));
let masterVolumeSlider: HTMLInputElement | null = null;
let masterVolumeValueEl: HTMLElement | null = null;

type SourceMode = 'tone' | 'mic';
type MicChannelMode = 'L' | 'R' | 'MONO' | 'STEREO';

interface BaseGraph {
  ctx: AudioContext;
  oscL: OscillatorNode;
  oscR: OscillatorNode;
  toneGain: GainNode;
  micGain: GainNode;
  micSource: MediaStreamAudioSourceNode | null;
  micStream: MediaStream | null;
  micDeviceId: string | null;
  micSplitter: ChannelSplitterNode;
  micMerger: ChannelMergerNode;
  micGainLL: GainNode;
  micGainLR: GainNode;
  micGainRL: GainNode;
  micGainRR: GainNode;
  inGain: GainNode;
  outGain: GainNode;
  splitter: ChannelSplitterNode;
  analyserL: AnalyserNode;
  analyserR: AnalyserNode;
  meterTimer: number;
}

let sourceMode: SourceMode = 'tone';
let micChannelMode: MicChannelMode = 'L';

const ui = getElements();

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

setCoi(ui, globalThis.crossOriginIsolated === true);
wireMasterVolume();
wireShelfUrlLoader();
renderRack();
const proxyReady = registerPluginProxy();

ui.playBtn.addEventListener('click', () => void onPlay());
ui.stopBtn.addEventListener('click', () => void onStop());
ui.sourceToggle.addEventListener('click', () => void onSourceToggle());
ui.micDevice?.addEventListener('change', () => {
  void ensureMicSource(ui.micDevice.value || undefined).catch((e) => showError(ui, e));
});
for (const btn of ui.micChannelWrap.querySelectorAll<HTMLButtonElement>('.micChannelOpt')) {
  btn.addEventListener('click', () => {
    const mode = btn.dataset.mode as MicChannelMode | undefined;
    if (mode) void setMicChannelMode(mode);
  });
}
updateSourceUi();

void loadShelf();
ui.playBtn.disabled = false;

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    const last = [...openPanels.values()].at(-1);
    if (last) {
      const idx = Number.parseInt(last.dataset.slotIndex ?? '', 10);
      if (Number.isFinite(idx)) closePluginUi(idx);
    }
  }
});

// ---------------------------------------------------------------------------
// Audio graph
// ---------------------------------------------------------------------------

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

  // Two gated source paths feed `inGain`. Switching is a gain crossfade,
  // not a reconnect — keeps the graph topology stable and avoids zipper
  // noise during the switch.
  const toneGain = ctx.createGain();
  const micGain = ctx.createGain();
  toneGain.gain.value = sourceMode === 'tone' ? TONE_LEVEL_GAIN : 0;
  micGain.gain.value = sourceMode === 'mic' ? 1 : 0;
  merger.connect(toneGain);

  // Mic channel router. The MediaStreamSource is connected to `micSplitter`
  // lazily (in ensureMicSource); the rest of the topology is permanent.
  // Per-gain values (LL/LR/RL/RR) implement the L / R / MONO / STEREO mode.
  const micSplitter = ctx.createChannelSplitter(2);
  const micMerger = ctx.createChannelMerger(2);
  const micGainLL = ctx.createGain();
  const micGainLR = ctx.createGain();
  const micGainRL = ctx.createGain();
  const micGainRR = ctx.createGain();
  micSplitter.connect(micGainLL, 0);
  micSplitter.connect(micGainLR, 0);
  micSplitter.connect(micGainRL, 1);
  micSplitter.connect(micGainRR, 1);
  micGainLL.connect(micMerger, 0, 0);
  micGainRL.connect(micMerger, 0, 0);
  micGainLR.connect(micMerger, 0, 1);
  micGainRR.connect(micMerger, 0, 1);
  micMerger.connect(micGain);

  const inGain = ctx.createGain();
  inGain.gain.value = 0.7;
  toneGain.connect(inGain);
  micGain.connect(inGain);

  const splitter = ctx.createChannelSplitter(2);
  const analyserL = ctx.createAnalyser();
  const analyserR = ctx.createAnalyser();
  analyserL.fftSize = 1024;
  analyserR.fftSize = 1024;
  splitter.connect(analyserL, 0);
  splitter.connect(analyserR, 1);

  const outGain = ctx.createGain();
  outGain.gain.value = 0.5;
  outGain.connect(ctx.destination);

  oscL.start();
  oscR.start();
  await ctx.suspend();

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
    toneGain,
    micGain,
    micSource: null,
    micStream: null,
    micDeviceId: null,
    micSplitter,
    micMerger,
    micGainLL,
    micGainLR,
    micGainRL,
    micGainRR,
    inGain,
    outGain,
    splitter,
    analyserL,
    analyserR,
    meterTimer
  };
  applyMicChannelMode();
  applyMasterVolume();
  return baseGraph;
}

async function ensureChainNode(): Promise<AudioWorkletNode> {
  const graph = await ensureBaseGraph();
  if (chainNode) return chainNode;
  if (!workletReady) {
    await graph.ctx.audioWorklet.addModule(workletUrl);
    workletReady = true;
  }

  // The Rust host wasm is loaded once on the main thread (via getHost) and
  // its init object is passed into the worklet's processorOptions. The
  // worklet then calls startHost(hostInit, hostImports) inside the audio
  // realm, identical to V1's flow but for a SINGLE host instance that owns
  // every plugin.
  const hostWasmUrl = new URL('./wclap-runtime/host-rust.wasm', import.meta.url).href;
  const hostConfig = await getHost(hostWasmUrl);
  const host = await startHost(hostConfig, minimalHostImports());

  chainNode = new AudioWorkletNode(graph.ctx, 'vocal-chain', {
    numberOfInputs: 1,
    numberOfOutputs: 1,
    outputChannelCount: [2],
    processorOptions: { host: host.initObj() }
  });
  chainNode.port.onmessage = onWorkletMessage;

  iframeBridge = new IframeBridge({
    port: chainNode.port,
    getIframe: (slot) => slots[slot]?.iframe ?? null
  });

  graph.inGain.connect(chainNode);
  chainNode.connect(graph.splitter);
  chainNode.connect(graph.outGain);

  // Resume the AudioContext so the worklet's audio thread is alive. Without
  // this, port messages to the worklet are not dispatched (host-ready gets
  // through on initial construction, but subsequent `load` messages are
  // parked until the context resumes). We rely on the drop / shelf click
  // user gesture as the resume-permission. Audible output is still gated
  // by `outGain` (master volume slider) so this doesn't autoplay loudly.
  if (graph.ctx.state === 'suspended') {
    try {
      await graph.ctx.resume();
    } catch (e) {
      console.warn('[host] ctx.resume failed (no user gesture yet?)', e);
    }
  }

  // Wait until the worklet posts host-ready before any load happens.
  await hostReadyPromise;
  return chainNode;
}

// The main-thread host instance is created only to extract `initObj()` for
// transfer into the worklet; it never runs audio here. Trap import callbacks
// just in case wclap-host-js touches them during construction.
function minimalHostImports() {
  return {
    env: {
      webviewSend: () => {},
      eventsOutTryPush: () => {},
      stateMarkDirty: () => {},
      paramsRescan: () => {},
      log: () => {}
    }
  };
}

let iframeBridge: IframeBridge | null = null;

const pendingRequests = new Map<number, (data: unknown) => void>();
let nextRequestId = 1;

function workletRequest<T>(kind: string, payload: Record<string, unknown>): Promise<T> {
  if (!chainNode) return Promise.reject(new Error('chain worklet not ready'));
  const requestId = nextRequestId++;
  return new Promise<T>((resolve) => {
    pendingRequests.set(requestId, (data) => resolve(data as T));
    chainNode!.port.postMessage({ kind, requestId, ...payload });
  });
}

function onWorkletMessage(e: MessageEvent): void {
  const data = e.data;
  if (!data) return;
  if (data instanceof ArrayBuffer) {
    // Unrouted ArrayBuffer (shouldn't happen in V2; we use envelopes).
    return;
  }
  if (typeof data !== 'object') return;
  if (data.kind === 'msgerror') {
    showError(
      ui,
      new Error(
        `Worklet messageerror: a message from main couldn't be deserialized ` +
          `inside the AudioWorklet. This typically means the payload contains ` +
          `data that cross-realm structured clone rejects — most often a ` +
          `WebAssembly.Module, which Chrome refuses to transfer over an ` +
          `AudioWorklet MessagePort. (info: ${JSON.stringify(data.info)})`
      )
    );
    setStatus(ui, `Load failed — see error below.`);
    return;
  }
  switch (data.kind) {
    case 'host-ready':
      hostReadyResolve?.();
      break;
    case 'loaded':
      onLoadedFromWorklet(data);
      break;
    case 'unloaded':
      // Already cleaned up on main thread when we issued the unload.
      break;
    case 'webview':
      iframeBridge?.forwardToIframe(data.slot as number, data.buf as ArrayBuffer);
      break;
    case 'params': {
      const resolver = pendingRequests.get(data.requestId as number);
      if (resolver) {
        pendingRequests.delete(data.requestId as number);
        resolver(data.params);
      }
      break;
    }
    case 'state-saved': {
      const resolver = pendingRequests.get(data.requestId as number);
      if (resolver) {
        pendingRequests.delete(data.requestId as number);
        resolver({ ok: data.ok, buf: data.buf });
      }
      break;
    }
    case 'state-loaded': {
      const resolver = pendingRequests.get(data.requestId as number);
      if (resolver) {
        pendingRequests.delete(data.requestId as number);
        resolver({ ok: data.ok });
      }
      break;
    }
    case 'latency': {
      const resolver = pendingRequests.get(data.requestId as number);
      if (resolver) {
        pendingRequests.delete(data.requestId as number);
        resolver(data.latency);
      }
      break;
    }
    case 'crashed':
      showError(ui, new Error(`chain crashed: ${data.error}`));
      break;
    case 'error':
      showError(ui, new Error(`slot ${data.slot}: ${data.error}`));
      break;
  }
}

function onLoadedFromWorklet(data: {
  slot: number;
  pluginId: string;
  info?: { desc?: { name?: string; vendor?: string }; webview?: string | null };
  bundlePlugins?: BundlePlugin[];
}): void {
  const slot = slots[data.slot];
  if (!slot) return;
  slot.pluginId = data.pluginId;
  slot.plugins = Array.isArray(data.bundlePlugins) ? data.bundlePlugins : [];
  const foundIndex = slot.plugins.findIndex((p) => p.id === data.pluginId);
  slot.pluginIndex = foundIndex >= 0 ? foundIndex : 0;
  // `info.webview` is the authoritative URI the plugin returned from its
  // clap.webview/3 extension. Null means the plugin has no UI at all
  // (extension not advertised). This is what differentiates the keyboard
  // plugin from the chorus plugin inside the signalsmith-clap-cpp bundle.
  slot.webviewUri = data.info?.webview ?? null;
  const name = data.info?.desc?.name ?? slot.manifest?.name ?? data.pluginId;
  slot.label = name;
  refreshPluginSummary();
  renderRack();
  setStatus(ui, `${name} loaded in slot ${data.slot + 1}.`);
  // Latency may need a fresh read; the plugin reports it through
  // clap.latency.get and we render it in the rack strip.
  void refreshLatency(data.slot);
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

async function onPlay(): Promise<void> {
  try {
    const graph = await ensureBaseGraph();
    await ensureChainNode();
    await graph.ctx.resume();
    ui.playBtn.disabled = true;
    ui.stopBtn.disabled = false;
    setStatus(ui, 'Running.');
  } catch (e) {
    showError(ui, e);
  }
}

async function onStop(): Promise<void> {
  if (!baseGraph) return;
  await baseGraph.ctx.suspend();
  ui.playBtn.disabled = false;
  ui.stopBtn.disabled = true;
  setStatus(ui, 'Stopped.');
}

function statusForRunning(): string {
  const loaded = slots.filter((s) => s.pluginId).length;
  const src = sourceMode === 'mic' ? `mic (${micChannelMode})` : '440 Hz tone';
  if (loaded === 0) return `Playing — ${src} (no plugin in chain).`;
  return `Playing — ${src} through ${loaded} plugin${loaded === 1 ? '' : 's'}.`;
}

// ---------------------------------------------------------------------------
// Source toggle (tone ↔ mic) + mic channel router
// ---------------------------------------------------------------------------

async function onSourceToggle(): Promise<void> {
  const next: SourceMode = sourceMode === 'tone' ? 'mic' : 'tone';
  try {
    await setSourceMode(next);
  } catch (err) {
    showError(ui, err);
    // Roll back UI on failure (e.g. user denied mic permission).
    updateSourceUi();
  }
}

async function setSourceMode(mode: SourceMode): Promise<void> {
  if (mode === sourceMode) return;

  if (mode === 'mic') {
    if (!baseGraph?.micSource) {
      ui.sourceStatus.textContent = 'Mic · awaiting permission…';
    }
    await ensureMicSource();
  }

  sourceMode = mode;
  if (baseGraph) {
    baseGraph.toneGain.gain.value = mode === 'tone' ? TONE_LEVEL_GAIN : 0;
    baseGraph.micGain.gain.value = mode === 'mic' ? 1 : 0;
  }
  updateSourceUi();
  if (baseGraph && baseGraph.ctx.state === 'running') {
    setStatus(ui, statusForRunning());
  }
}

async function ensureMicSource(deviceId?: string): Promise<void> {
  const graph = await ensureBaseGraph();
  if (
    graph.micSource &&
    graph.micStream &&
    (deviceId === undefined || deviceId === graph.micDeviceId)
  ) {
    return;
  }

  if (!navigator.mediaDevices?.getUserMedia) {
    throw new Error('navigator.mediaDevices.getUserMedia unavailable in this browser.');
  }

  // Stop any previous stream's tracks before requesting a new one — otherwise
  // the browser keeps both open (recording indicator stays lit on the old one).
  if (graph.micSource) {
    try {
      graph.micSource.disconnect();
    } catch {
      // ignore
    }
    graph.micSource = null;
  }
  if (graph.micStream) {
    for (const t of graph.micStream.getTracks()) t.stop();
    graph.micStream = null;
  }

  // Keep browser DSP off — we want to test our own processing.
  const constraints: MediaTrackConstraints = {
    echoCancellation: false,
    noiseSuppression: false,
    autoGainControl: false
  };
  if (deviceId) constraints.deviceId = { exact: deviceId };

  const stream = await navigator.mediaDevices.getUserMedia({ audio: constraints });
  const src = graph.ctx.createMediaStreamSource(stream);
  src.connect(graph.micSplitter);
  graph.micStream = stream;
  graph.micSource = src;
  graph.micDeviceId = deviceId ?? null;

  await populateMicDevices();
}

async function populateMicDevices(): Promise<void> {
  if (!navigator.mediaDevices?.enumerateDevices) return;
  let devices: MediaDeviceInfo[];
  try {
    devices = await navigator.mediaDevices.enumerateDevices();
  } catch {
    return;
  }
  const inputs = devices.filter((d) => d.kind === 'audioinput');
  const select = ui.micDevice;
  const current = baseGraph?.micDeviceId ?? '';
  select.innerHTML = '';
  const defaultOpt = document.createElement('option');
  defaultOpt.value = '';
  defaultOpt.textContent = 'System default';
  select.appendChild(defaultOpt);
  for (const d of inputs) {
    const opt = document.createElement('option');
    opt.value = d.deviceId;
    opt.textContent = d.label || `Input (${d.deviceId.slice(0, 6)}…)`;
    select.appendChild(opt);
  }
  select.value = current;
}

function updateSourceUi(): void {
  if (!ui.sourceToggle) return;
  ui.sourceToggle.dataset.source = sourceMode;
  ui.sourceToggle.setAttribute('aria-checked', sourceMode === 'mic' ? 'true' : 'false');
  const haveMic = !!baseGraph?.micSource;
  if (sourceMode === 'tone') {
    ui.sourceStatus.textContent = 'Tone · 440 Hz';
  } else if (haveMic) {
    ui.sourceStatus.textContent = `Mic · live · ${micChannelMode}`;
  } else {
    ui.sourceStatus.textContent = 'Mic · awaiting permission…';
  }
  ui.micDeviceWrap.hidden = sourceMode !== 'mic';
  ui.micChannelWrap.hidden = sourceMode !== 'mic';
}

// Write the 4 routing gains to match the current `micChannelMode`. Safe to
// call before / without a live mic stream — gains stay set; audio just
// flows zero until the splitter is fed.
function applyMicChannelMode(): void {
  if (!baseGraph) return;
  const { micGainLL, micGainLR, micGainRL, micGainRR } = baseGraph;
  let ll = 0;
  let lr = 0;
  let rl = 0;
  let rr = 0;
  switch (micChannelMode) {
    case 'L':
      ll = 1;
      lr = 1;
      break;
    case 'R':
      rl = 1;
      rr = 1;
      break;
    case 'MONO':
      ll = 0.7071;
      lr = 0.7071;
      rl = 0.7071;
      rr = 0.7071;
      break;
    case 'STEREO':
      ll = 1;
      rr = 1;
      break;
  }
  micGainLL.gain.value = ll;
  micGainLR.gain.value = lr;
  micGainRL.gain.value = rl;
  micGainRR.gain.value = rr;
}

async function setMicChannelMode(mode: MicChannelMode): Promise<void> {
  if (mode === micChannelMode) return;
  micChannelMode = mode;
  for (const btn of ui.micChannelWrap.querySelectorAll<HTMLButtonElement>(
    '.micChannelOpt'
  )) {
    const isActive = btn.dataset.mode === mode;
    btn.classList.toggle('micChannelOptActive', isActive);
    btn.setAttribute('aria-checked', isActive ? 'true' : 'false');
  }
  if (!baseGraph) await ensureBaseGraph();
  applyMicChannelMode();
  if (sourceMode === 'mic') {
    updateSourceUi();
    if (baseGraph?.ctx.state === 'running') {
      setStatus(ui, statusForRunning());
    }
  }
}

function wireAudioState(ctx: AudioContext): void {
  const maxCh = ctx.destination.maxChannelCount;
  const baseLatencyMs = (ctx.baseLatency ?? 0) * 1000;
  const extra = `out=${maxCh}ch · base latency≈${baseLatencyMs.toFixed(1)}ms`;
  const update = (): void => {
    setAudioState(ui, ctx.state, extra);
    if (ctx.state === 'running') {
      ui.playBtn.disabled = true;
      ui.stopBtn.disabled = false;
    } else {
      ui.playBtn.disabled = false;
      ui.stopBtn.disabled = true;
    }
  };
  update();
  ctx.onstatechange = update;
}

// ---------------------------------------------------------------------------
// Master volume
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Shelf
// ---------------------------------------------------------------------------

function wireShelfUrlLoader(): void {
  const input = document.getElementById('shelfUrlInput') as HTMLInputElement | null;
  const btn = document.getElementById('shelfUrlAdd') as HTMLButtonElement | null;
  if (!input || !btn) return;
  const submit = (): void => {
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

// Same-origin proxy on the wclap-host worker (and mirrored by vite dev
// middleware). Pages never read cross-origin URLs directly.
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

  const probeUrl = proxiedUrl(parsed.href);
  btn.disabled = true;
  try {
    const res = await fetch(probeUrl, { method: 'HEAD' });
    if (!res.ok) throw new Error(`HEAD ${parsed.href}: ${res.status} ${res.statusText}`);
  } catch (err) {
    btn.disabled = false;
    showError(
      ui,
      new Error(
        `Couldn't reach ${parsed.href}: ${err instanceof Error ? err.message : String(err)}`
      )
    );
    return;
  }
  btn.disabled = false;

  const fileName = parsed.pathname.split('/').pop() ?? parsed.host;
  const label = fileName.replace(/\.(wclap\.tar\.gz|wasm)$/i, '');
  const item: ShelfItem = {
    id: itemId,
    label,
    url: proxiedUrl(parsed.href),
    vendor: parsed.host,
    hint: parsed.host
  };
  SHELF.push(item);
  renderShelf();
  input.value = '';
  setStatus(ui, `Added "${label}" to shelf.`);
}

async function loadShelf(): Promise<void> {
  try {
    const r = await fetch('/shelf.json');
    if (!r.ok) throw new Error('shelf.json: ' + r.status);
    const data = (await r.json()) as { items?: ShelfItem[] } | ShelfItem[];
    const items = Array.isArray(data) ? data : data.items ?? [];
    // Surface every bundle; the user puts what they like in any slot.
    SHELF = items;
    renderShelf();
  } catch (e) {
    showError(ui, e);
  }
}

function shelfDisplayName(item: ShelfItem): string {
  return item.label ?? item.name ?? item.id;
}

function renderShelf(): void {
  ui.shelf.innerHTML = '';
  for (const item of SHELF) {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'shelfChip';
    chip.draggable = true;
    chip.dataset.shelfId = item.id;

    const labelSpan = document.createElement('span');
    labelSpan.textContent = shelfDisplayName(item);
    chip.appendChild(labelSpan);

    chip.addEventListener('dragstart', (e) => {
      if (!e.dataTransfer) return;
      e.dataTransfer.setData(SHELF_DT_TYPE, item.id);
      e.dataTransfer.effectAllowed = 'copy';
    });

    chip.addEventListener('click', () => {
      const idx = firstEmptySlot();
      if (idx < 0) {
        setStatus(ui, 'Rack is full — remove a plugin first.');
        return;
      }
      void loadIntoSlot(idx, item);
    });

    ui.shelf.appendChild(chip);
  }
}

function firstEmptySlot(): number {
  return slots.findIndex((s) => !s.pluginId);
}

// ---------------------------------------------------------------------------
// Rack
// ---------------------------------------------------------------------------

function renderRack(): void {
  ui.rack.innerHTML = '';
  slots.forEach((slot, idx) => {
    const occupied = !!slot.pluginId;
    // Authoritative UI signal — the plugin's clap.webview/3.get_uri call
    // (forwarded through pluginGetInfo) is the only thing that knows
    // whether a given plugin in a (potentially multi-plugin) bundle has a
    // UI. We treat presence of a non-empty URI as the source of truth.
    const hasUi = occupied && !!slot.webviewUri;
    const slotEl = document.createElement('div');
    slotEl.className = `rackSlot ${occupied ? 'occupied' : 'empty'}${hasUi ? ' hasUi' : ''}`;
    slotEl.dataset.slotIndex = String(idx);

    const num = document.createElement('span');
    num.className = 'slotNum';
    num.textContent = String(idx + 1).padStart(2, '0');
    slotEl.appendChild(num);

    const label = document.createElement('span');
    label.className = 'slotLabel';
    label.textContent = occupied ? slot.label : 'drop a plugin here';
    if (occupied) {
      label.title = 'Click to open plugin UI';
      label.addEventListener('click', () => void openPluginUi(idx));
    }
    slotEl.appendChild(label);

    if (hasUi) {
      const stripBtn = document.createElement('button');
      stripBtn.type = 'button';
      stripBtn.className = 'slotStrip';
      stripBtn.textContent = 'strip';
      stripBtn.title = slot.manifest?.ui?.compact_size
        ? 'Open compact (strip) view'
        : 'Plugin manifest has no ui.compact_size';
      stripBtn.disabled = !slot.manifest?.ui?.compact_size;
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

      const bypassBtn = document.createElement('button');
      bypassBtn.type = 'button';
      bypassBtn.className = 'slotStrip slotBypass';
      if (slot.bypass) bypassBtn.classList.add('slotBypassActive');
      bypassBtn.textContent = slot.bypass ? 'byp' : 'on';
      bypassBtn.title = 'Toggle bypass';
      bypassBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleBypass(idx);
      });
      slotEl.appendChild(bypassBtn);

      // Latency readout — `clap.latency.get` in samples. Shown as `Nsmp`,
      // or `—` when the plugin reports 0 / hasn't been queried yet.
      const lat = document.createElement('span');
      lat.className = 'slotLatency';
      const samples = slot.latencySamples;
      lat.textContent = samples == null
        ? '—'
        : samples === 0
          ? '0smp'
          : `${samples}smp`;
      lat.title = samples == null
        ? 'Plugin latency not yet queried'
        : `Plugin reports ${samples} samples of processing latency`;
      slotEl.appendChild(lat);

      // Plugin cycle widget — only when this bundle holds more than one
      // plugin. Lets the user step through e.g. signalsmith-basics's six.
      // Sits to the left of the delete button so delete always lives at
      // the far right of the slot row.
      if (slot.plugins.length > 1) {
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

      const del = document.createElement('button');
      del.className = 'slotDelete';
      del.type = 'button';
      del.textContent = '✕';
      del.setAttribute('aria-label', `Remove plugin from slot ${idx + 1}`);
      del.title = 'Remove plugin';
      del.addEventListener('click', (e) => {
        e.stopPropagation();
        void unloadSlot(idx);
      });
      slotEl.appendChild(del);
    }

    // Always preventDefault on dragover, regardless of types — the
    // `dataTransfer.types` API doesn't reliably expose custom MIME data
    // during dragover in all browsers, and without preventDefault the
    // browser rejects the drop.
    slotEl.addEventListener('dragover', (e) => {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
      slotEl.classList.add('dragOver');
    });
    slotEl.addEventListener('dragleave', () => slotEl.classList.remove('dragOver'));
    slotEl.addEventListener('drop', (e) => {
      e.preventDefault();
      slotEl.classList.remove('dragOver');
      const shelfId = e.dataTransfer?.getData(SHELF_DT_TYPE);
      if (shelfId) {
        const item = SHELF.find((it) => it.id === shelfId);
        if (item) void loadIntoSlot(idx, item);
        return;
      }
      const file = e.dataTransfer?.files?.[0];
      if (file) void loadFileIntoSlot(idx, file);
    });

    ui.rack.appendChild(slotEl);
  });
}

function refreshPluginSummary(): void {
  const loaded = slots.filter((s) => s.pluginId).map((s) => s.label);
  setPlugin(ui, loaded.length ? loaded.join(' → ') : '—');
}

function toggleBypass(idx: number): void {
  const slot = slots[idx];
  if (!slot || !slot.pluginId || !chainNode) return;
  slot.bypass = !slot.bypass;
  chainNode.port.postMessage({ kind: 'set-bypass', slot: idx, bypass: slot.bypass });
  renderRack();
}

// State save / load — opaque plugin bytes round-tripped through base64 in
// the clipboard. The bytes themselves are the plugin's own format; the host
// makes no attempt to parse them. base64 just lets the user paste into any
// text field.
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
  if (!slot.pluginId) return;
  try {
    const res = await workletRequest<{ ok: boolean; buf: ArrayBuffer | null }>(
      'save-state',
      { slot: idx }
    );
    if (!res.ok || !res.buf) {
      setStatus(ui, `Slot ${idx + 1}: plugin returned no state.`);
      return;
    }
    const bytes = new Uint8Array(res.buf);
    const b64 = bytesToBase64(bytes);
    await navigator.clipboard.writeText(b64);
    setStatus(ui, `Slot ${idx + 1}: copied ${bytes.byteLength} bytes of state to clipboard.`);
  } catch (err) {
    showError(ui, err);
  }
}

async function loadStateFromClipboard(idx: number): Promise<void> {
  const slot = slots[idx];
  if (!slot.pluginId) return;
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
  } catch {
    setStatus(ui, `Slot ${idx + 1}: clipboard contents aren't valid base64.`);
    return;
  }
  try {
    const res = await workletRequest<{ ok: boolean }>('load-state', {
      slot: idx,
      buf: bytes.buffer
    });
    if (res.ok) {
      setStatus(ui, `Slot ${idx + 1}: loaded ${bytes.byteLength} bytes of state.`);
      // Latency may shift after a state load (some plugins recompute it).
      void refreshLatency(idx);
    } else {
      setStatus(ui, `Slot ${idx + 1}: plugin rejected the state.`);
    }
  } catch (err) {
    showError(ui, err);
  }
}

async function refreshLatency(idx: number): Promise<void> {
  const slot = slots[idx];
  if (!slot.pluginId) return;
  try {
    const latency = await workletRequest<number>('get-latency', { slot: idx });
    slot.latencySamples = typeof latency === 'number' ? latency : null;
    renderRack();
  } catch {
    /* swallow — latency is best-effort */
  }
}

// Cycle to the previous/next plugin in this slot's bundle. Re-runs the
// full load path with a different target id; we don't try to swap plugins
// inside the worklet because unload-then-load is the only path that has
// been validated end-to-end.
async function cycleSlot(idx: number, dir: 1 | -1): Promise<void> {
  const slot = slots[idx];
  if (!slot.url || slot.plugins.length < 2) return;
  const item = SHELF.find((it) => it.url === slot.url);
  if (!item) return;
  const len = slot.plugins.length;
  const newIndex = (slot.pluginIndex + dir + len) % len;
  // Capture the target id NOW — the worklet-side getInfo() list is the
  // authoritative source for multi-plugin bundles (external bundles like
  // signalsmith have no manifest.plugins to fall back on), and unloadSlot
  // in the upcoming reload will wipe slot.plugins before the reload picks
  // its target.
  const targetId = slot.plugins[newIndex]?.id;
  await loadIntoSlot(idx, item, newIndex, targetId);
}

// ---------------------------------------------------------------------------
// Plugin load / unload
// ---------------------------------------------------------------------------

async function loadIntoSlot(
  idx: number,
  item: ShelfItem,
  preferPluginIndex = 0,
  explicitPluginId?: string
): Promise<void> {
  clearError(ui);
  const name = shelfDisplayName(item);
  setStatus(ui, `Loading ${name} into slot ${idx + 1}…`);
  try {
    const node = await ensureChainNode();
    if (slots[idx].pluginId) await unloadSlot(idx);
    const { wclapConfig, pluginId, manifest, files } = await fetchWclap(item.url);
    slots[idx].url = item.url;
    slots[idx].manifest = manifest;
    slots[idx].files = files;
    slots[idx].label = name;
    slots[idx].pluginPath = (wclapConfig as { pluginPath?: string }).pluginPath ?? null;
    // Pick the target plugin id, in priority order:
    //   1. explicit override (used by cycleSlot — authoritative)
    //   2. nth entry of manifest.plugins for plinken bundles
    //   3. loader's default (first plugin in the bundle)
    let targetPluginId = pluginId;
    if (explicitPluginId) {
      targetPluginId = explicitPluginId;
    } else if (preferPluginIndex > 0) {
      const bundleList = Array.isArray(manifest?.plugins) ? manifest.plugins as BundlePlugin[] : [];
      const target = bundleList[preferPluginIndex];
      if (target?.id) targetPluginId = target.id;
    }
    // WORKAROUND: WebAssembly.Module cannot be transferred over an
    // AudioWorklet MessagePort in Chromium (works for Web Workers but not
    // for the audio thread). Strip the compiled module and let the worklet
    // recompile from the wasm bytes still present in `wclapConfig.files`
    // under the `<pluginPath>/module.wasm` key.
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    const { module: _strippedModule, ...wclapForWorklet } = wclapConfig as Record<string, unknown>;
    node.port.postMessage({ kind: 'load', slot: idx, wclap: wclapForWorklet, pluginId: targetPluginId });
  } catch (e) {
    showError(ui, e);
    setStatus(ui, `Failed to load slot ${idx + 1}.`);
  }
}

async function loadFileIntoSlot(idx: number, file: File): Promise<void> {
  const url = URL.createObjectURL(file);
  const item: ShelfItem = {
    id: `local:${file.name}`,
    label: file.name.replace(/\.wclap\.tar\.gz$/, ''),
    url
  };
  await loadIntoSlot(idx, item);
}

async function unloadSlot(idx: number): Promise<void> {
  const slot = slots[idx];
  if (!slot.pluginId) return;
  closePluginUi(idx);
  closeAutoUi(idx);
  chainNode?.port.postMessage({ kind: 'unload', slot: idx });
  slot.url = null;
  slot.pluginId = null;
  slot.manifest = null;
  slot.files = null;
  slot.label = '';
  slot.bypass = false;
  slot.plugins = [];
  slot.pluginIndex = 0;
  slot.webviewUri = null;
  slot.pluginPath = null;
  slot.latencySamples = null;
  renderRack();
  refreshPluginSummary();
}

// ---------------------------------------------------------------------------
// Plugin UI (iframe panels)
// ---------------------------------------------------------------------------

const openPanels = new Map<number, HTMLElement>();
let panelCascade = 0;
const proxyResolvers = new Map<number, (p: string) => Promise<ArrayBuffer | null>>();

type PanelMode = 'expanded' | 'compact';

async function openPluginUi(idx: number, mode: PanelMode = 'expanded'): Promise<void> {
  await proxyReady;
  const slot = slots[idx];
  if (!slot.pluginId || !slot.files) return;
  // If a panel for this slot is already open, swap modes if the user asked
  // for a different one (e.g. clicking 'strip' on an expanded panel);
  // otherwise just raise the existing panel to the front.
  if (openPanels.has(idx)) {
    const existing = openPanels.get(idx)!;
    if (existing.dataset.panelMode === mode) {
      bringPanelToFront(existing);
      return;
    }
    closePluginUi(idx);
  }
  // The plugin's clap.webview/3.get_uri tells us exactly which file to
  // load — already fetched by the worklet inside pluginGetInfo and parked
  // on `slot.webviewUri`. The URI looks like `file:/plugin/<hash>/ui/...`;
  // strip the `file:` prefix and prepend the proxy SW prefix to form the
  // iframe src. Null URI means the plugin has no UI extension at all
  // (e.g. the chorus / compressor plugins in signalsmith-clap-cpp).
  if (!slot.webviewUri) {
    setStatus(ui, `Slot ${idx + 1}: this plugin has no UI.`);
    return;
  }
  // Strip `file:` and any URL-authority slashes that follow. Plinken
  // plugins emit `file:/plugin/<hash>/...` (one slash), signalsmith emits
  // `file:///plugin/<hash>/...` (URI standard form with authority slash).
  // Normalise to a single leading slash so the proxy SW path matches the
  // file map keys.
  const uiKey = slot.webviewUri.replace(/^file:\/*/, '/');
  const files = slot.files;
  const pluginPath = slot.pluginPath ?? '';

  // Resolver tries the request path against the bundle's file map under
  // several forms. wclap-host-js mutates plugin paths (adding a `-copy-<hex>`
  // suffix on second instantiation), and some upstream plugins return a
  // get_uri relative to their pluginPath rather than absolute. So we fall
  // back to: prefixed, trimmed-leading-slash-prefixed, and re-mapping the
  // mutated prefix back to the original.
  proxyResolvers.set(idx, async (path: string) => {
    if (files[path]) return files[path];
    if (pluginPath) {
      const prefixed = pluginPath + (path.startsWith('/') ? path : '/' + path);
      if (files[prefixed]) return files[prefixed];
    }
    // Some bundles (`-copy-<hex>` suffix) — try stripping it from the path.
    const stripped = path.replace(/^(\/plugin\/[^/]+?)-copy-[0-9a-f]+\//i, '$1/');
    if (files[stripped]) return files[stripped];
    return null;
  });

  const iframe = document.createElement('iframe');
  iframe.src = PROXY_PREFIX + uiKey;
  iframe.title = slot.label;

  slot.iframe = iframe;
  iframeBridge?.register(idx, iframe);

  const panel = buildPluginPanel(idx, slot.label, iframe, mode, slot.manifest);
  document.getElementById('pluginPanels')?.appendChild(panel);
  slot.panel = panel;
  openPanels.set(idx, panel);
  positionPanel(panel);
  wirePanelDrag(panel);
}

function buildPluginPanel(
  idx: number,
  label: string,
  iframe: HTMLIFrameElement,
  mode: PanelMode = 'expanded',
  manifest?: WclapManifest | null
): HTMLElement {
  const panel = document.createElement('div');
  panel.className = `pluginPanel pluginPanel--${mode}`;
  panel.dataset.slotIndex = String(idx);
  panel.dataset.panelMode = mode;

  const head = document.createElement('div');
  head.className = 'pluginPanelHead';
  const title = document.createElement('span');
  title.className = 'pluginPanelTitle';
  title.textContent = `${idx + 1}. ${label}`;
  head.appendChild(title);

  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'pluginPanelClose';
  closeBtn.textContent = '×';
  closeBtn.title = 'Close';
  closeBtn.addEventListener('click', () => closePluginUi(idx));
  head.appendChild(closeBtn);

  panel.appendChild(head);

  const body = document.createElement('div');
  body.className = 'pluginPanelBody';
  body.appendChild(iframe);
  panel.appendChild(body);

  // Apply manifest-declared size to the panel itself (not the iframe — the
  // iframe is `width:100%; height:100%` of the body, and the panel's
  // default `width: clamp(20rem, ...)` would otherwise floor any compact
  // bundle at ~320×288). The plugin's UI uses viewport size to switch
  // between compact and expanded layout, so this controls layout inside
  // too. We approximate the head's height (~36px incl. border) so the
  // iframe body ends up close to the manifest's declared dimensions.
  const size =
    mode === 'compact'
      ? manifest?.ui?.compact_size
      : manifest?.ui?.expanded_size;
  if (size) {
    const HEAD_PX = 36;
    panel.style.width = `${size.width}px`;
    panel.style.height = `${size.height + HEAD_PX}px`;
    panel.style.minWidth = '0';
    panel.style.minHeight = '0';
  }

  return panel;
}

// ---------------------------------------------------------------------------
// Auto-generated params panel
// ---------------------------------------------------------------------------

interface ParamInfo {
  id: number;
  name: string;
  min: number;
  max: number;
  default: number;
  value?: number;
  flags: number;
}

const autoPanels = new Map<number, HTMLElement>();

async function toggleAutoUi(idx: number): Promise<void> {
  if (autoPanels.has(idx)) {
    closeAutoUi(idx);
    return;
  }
  const slot = slots[idx];
  if (!slot.pluginId) return;
  await ensureChainNode();

  let params: ParamInfo[] = [];
  try {
    params = await workletRequest<ParamInfo[]>('get-params', { slot: idx });
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
    for (const p of params) body.appendChild(buildParamRow(idx, p));
  }
  panel.appendChild(body);

  document.getElementById('pluginPanels')?.appendChild(panel);
  positionPanel(panel);
  autoPanels.set(idx, panel);
  wirePanelDrag(panel);
}

function buildParamRow(idx: number, p: ParamInfo): HTMLElement {
  const row = document.createElement('label');
  row.className = 'autoParam';

  const head = document.createElement('span');
  head.className = 'autoParamHead';
  const name = document.createElement('span');
  name.className = 'autoParamName';
  name.textContent = p.name || `#${p.id}`;
  const valueEl = document.createElement('span');
  valueEl.className = 'autoParamValue';
  const initial = typeof p.value === 'number' ? p.value : p.default;
  valueEl.textContent = initial.toFixed(3);
  head.appendChild(name);
  head.appendChild(valueEl);

  const slider = document.createElement('input');
  slider.type = 'range';
  slider.min = String(p.min);
  slider.max = String(p.max);
  const span = p.max - p.min;
  slider.step = span > 0 ? String(span / 1000) : '0.001';
  slider.value = String(initial);

  slider.addEventListener('input', () => {
    const v = parseFloat(slider.value);
    valueEl.textContent = v.toFixed(3);
    chainNode?.port.postMessage({ kind: 'set-param', slot: idx, paramId: p.id, value: v });
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

function closePluginUi(idx: number): void {
  const slot = slots[idx];
  const panel = openPanels.get(idx);
  if (panel) panel.remove();
  openPanels.delete(idx);
  proxyResolvers.delete(idx);
  iframeBridge?.unregister(idx);
  if (slot) {
    slot.iframe = null;
    slot.panel = null;
  }
}

function positionPanel(panel: HTMLElement): void {
  const offset = (panelCascade++ % 8) * 24;
  panel.style.left = `${40 + offset}px`;
  panel.style.top = `${40 + offset}px`;
}

function bringPanelToFront(panel: HTMLElement): void {
  panel.parentElement?.appendChild(panel);
}

function wirePanelDrag(panel: HTMLElement): void {
  const head = panel.querySelector('.pluginPanelHead') as HTMLElement | null;
  if (!head) return;
  let startX = 0;
  let startY = 0;
  let originX = 0;
  let originY = 0;
  let dragging = false;
  head.addEventListener('pointerdown', (e) => {
    if ((e.target as HTMLElement).closest('button')) return;
    dragging = true;
    startX = e.clientX;
    startY = e.clientY;
    const rect = panel.getBoundingClientRect();
    originX = rect.left;
    originY = rect.top;
    head.setPointerCapture(e.pointerId);
  });
  head.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    panel.style.left = `${originX + e.clientX - startX}px`;
    panel.style.top = `${originY + e.clientY - startY}px`;
  });
  head.addEventListener('pointerup', (e) => {
    dragging = false;
    head.releasePointerCapture(e.pointerId);
  });
}

// ---------------------------------------------------------------------------
// Plugin-proxy SW (serves bundled UI assets to iframes)
// ---------------------------------------------------------------------------

async function registerPluginProxy(): Promise<void> {
  if (!('serviceWorker' in navigator)) return;
  try {
    const reg = await navigator.serviceWorker.register('/plugin-proxy-sw.js', {
      scope: '/plugin-proxy/'
    });
    if (!reg.active) {
      const sw = reg.installing ?? reg.waiting;
      if (sw) {
        await new Promise<void>((resolve) => {
          const check = (): void => {
            if (sw.state === 'activated') resolve();
          };
          sw.addEventListener('statechange', check);
          check();
        });
      }
    }
  } catch (err) {
    console.warn('[vocal-host] plugin-proxy SW registration failed', err);
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

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

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
// Web MIDI → CLAP events
// ---------------------------------------------------------------------------
//
// V2 routing differs from V1: instead of one AudioWorkletNode per plugin
// each exposing an `acceptEvent` remote method, V2 has a single chain
// worklet. We encode events on main and ship them with the new
// `midi-event` worklet message; the worklet pushes them into the target
// slot's `pendingInputEvents` queue, which is drained at the start of
// each block via `hostApi.pluginAcceptEvent`. Plugin output_events
// flow forward to the next non-bypassed slot via the `eventsOutTryPush`
// host callback (also implemented inside the worklet).

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
  dv.setUint32(0, CLAP_EVENT_NOTE_SIZE, true); // size
  dv.setUint32(4, 0, true); // time = block start
  dv.setUint16(8, CLAP_CORE_EVENT_SPACE_ID, true);
  dv.setUint16(10, isOn ? CLAP_EVENT_NOTE_ON : CLAP_EVENT_NOTE_OFF, true);
  dv.setUint32(12, 1, true); // flags = CLAP_EVENT_IS_LIVE
  dv.setInt32(16, -1, true); // note_id (-1 = unspecified)
  dv.setInt16(20, 0, true); // port_index
  dv.setInt16(22, channel, true);
  dv.setInt16(24, key, true);
  // bytes 26..32 alignment pad for the f64
  dv.setFloat64(32, velocity, true); // velocity 0.0..1.0
  return buf;
}

function encodeClapMidiEvent(midiBytes: Uint8Array): ArrayBuffer {
  // Pass-through wrapper for any 1–3-byte channel-voice message (CC,
  // pitch bend, aftertouch, program change). Plugins consuming MIDI
  // dispatch on header.type = CLAP_EVENT_MIDI.
  const buf = new ArrayBuffer(CLAP_EVENT_MIDI_SIZE);
  const dv = new DataView(buf);
  dv.setUint32(0, CLAP_EVENT_MIDI_SIZE, true);
  dv.setUint32(4, 0, true);
  dv.setUint16(8, CLAP_CORE_EVENT_SPACE_ID, true);
  dv.setUint16(10, CLAP_EVENT_MIDI, true);
  dv.setUint32(12, 1, true); // flags = CLAP_EVENT_IS_LIVE
  dv.setUint16(16, 0, true); // port_index
  dv.setUint8(18, midiBytes[0] ?? 0);
  dv.setUint8(19, midiBytes[1] ?? 0);
  dv.setUint8(20, midiBytes[2] ?? 0);
  return buf;
}

// Send an event to the chain worklet for fan-out. `slot: null` (default)
// posts the event to every loaded slot — that's V1's behaviour and what
// the user expects for hardware MIDI. The worklet then forwards output
// events between slots so a keyboard plugin in slot 0 drives a synth
// plugin in slot 1 automatically.
function postEventToChain(buf: ArrayBuffer, slot: number | null = null): void {
  if (!chainNode) return;
  chainNode.port.postMessage(
    { kind: 'midi-event', slot, buf },
    [buf]
  );
}

function panicAllNotesOff(): void {
  // Send a NOTE_OFF for every key on channel 0 — covers stuck notes from
  // any plugin that consumed our input. Then clear the held-state UI.
  for (let key = 0; key < 128; key++) {
    postEventToChain(encodeClapNoteEvent(false, 0, key, 0));
  }
  heldNotes.clear();
  refreshMidiNotesUi();
  flashMidiLed(ui);
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
function midiNoteName(key: number): string {
  const name = NOTE_NAMES[key % 12] ?? '?';
  const octave = Math.floor(key / 12) - 1;
  return `${name}${octave}`;
}

// Held notes (currently down) keyed by `${channel}:${key}`. Display shows
// only what's actually held — the "chord log" from V1.
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

let midiAccess: MIDIAccess | null = null;

function rescanMidi(): void {
  if (!midiAccess) {
    void setupWebMidi();
    return;
  }
  for (const input of midiAccess.inputs.values()) wireMidiInput(input);
  refreshMidiInputsLabel(midiAccess);
  flashMidiLed(ui);
  setStatus(ui, `MIDI rescanned: ${midiAccess.inputs.size} input(s).`);
}

function wireMidiInput(input: MIDIInput): void {
  input.onmidimessage = (ev) => {
    flashMidiLed(ui);
    const data = ev.data;
    if (!data || data.length < 1) return;
    const status = data[0] ?? 0;
    const high = status & 0xf0;
    const channel = status & 0x0f;
    const key = data[1] ?? 0;
    const velRaw = data[2] ?? 0;
    // Make sure the chain worklet exists; we need a route for the event.
    // Don't block — if the worklet isn't built yet, drop the event.
    if (!chainNode) return;
    if (high === 0x90 && velRaw > 0) {
      const velocity = velRaw / 127;
      heldNotes.set(`${channel}:${key}`, { key, vel: velocity });
      refreshMidiNotesUi();
      postEventToChain(encodeClapNoteEvent(true, channel, key, velocity));
    } else if (high === 0x80 || (high === 0x90 && velRaw === 0)) {
      if (heldNotes.delete(`${channel}:${key}`)) refreshMidiNotesUi();
      postEventToChain(encodeClapNoteEvent(false, channel, key, velRaw / 127));
    } else if (
      high === 0xb0 || // CC
      high === 0xe0 || // pitch bend
      high === 0xd0 || // channel aftertouch
      high === 0xa0 || // poly aftertouch
      high === 0xc0    // program change
    ) {
      postEventToChain(encodeClapMidiEvent(data));
    }
  };
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
  midiAccess = access;
  for (const input of access.inputs.values()) wireMidiInput(input);
  refreshMidiInputsLabel(access);
  access.onstatechange = (ev) => {
    const port = (ev as MIDIConnectionEvent).port;
    if (port instanceof MIDIInput && port.state === 'connected') {
      wireMidiInput(port);
    }
    refreshMidiInputsLabel(access);
  };
  const count = access.inputs.size;
  if (count > 0) {
    console.log(`[wclap-host] Web MIDI: ${count} input(s) listening`);
  }
}

ui.midiPanic.addEventListener('click', panicAllNotesOff);
ui.midiRescan.addEventListener('click', rescanMidi);
void setupWebMidi();

