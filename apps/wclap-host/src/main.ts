// Cap WebAssembly.Memory maxima before any wasm code runs (upstream getWclap
// otherwise reserves 2 GB of shared virtual memory per plugin load).
import './wclap-runtime/cap-wasm-memory';

import ClapAudioNode, {
  type ClapEffectAudioNode
} from './wclap-runtime/clap-audionode.mjs';
// Vite bundles this module standalone (resolving its imports against the
// alias) and returns its URL as a string. AudioWorklet.addModule accepts the
// same ES module bundle that a Worker would.
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
  showError,
  wireDropZone
} from './ui';

interface RunningGraph {
  ctx: AudioContext;
  oscL: OscillatorNode;
  oscR: OscillatorNode;
  effect: ClapEffectAudioNode | null;
  analyserL: AnalyserNode;
  analyserR: AnalyserNode;
  meterTimer: number;
  blobUrl: string | null;
}

const ui = getElements();

setCoi(ui, globalThis.crossOriginIsolated === true);
setAudioState(ui, 'idle (no context)');
setStatus(ui, 'Press Play for a 440 Hz test tone, or drop a .wclap to chain a plugin.');
setPlugin(ui, '(no plugin — direct test tone)');

let current: RunningGraph | null = null;

// Enable Play right away. The graph is built on first click — inside the
// user gesture, which keeps the autoplay policy happy on every browser.
ui.playBtn.disabled = false;

wireDropZone(ui, (file) => {
  void loadPlugin(file);
});

const SAMPLE_URL = '/samples/signalsmith-basics.wclap.tar.gz';
const SAMPLE_NAME = 'signalsmith-basics.wclap';

ui.sampleBtn.addEventListener('click', () => {
  void loadSample();
});

async function loadSample(): Promise<void> {
  ui.sampleBtn.disabled = true;
  try {
    setStatus(ui, `Fetching ${SAMPLE_NAME}…`);
    const res = await fetch(SAMPLE_URL);
    if (!res.ok) throw new Error(`Fetch failed: ${res.status} ${res.statusText}`);
    const blob = await res.blob();
    const file = new File([blob], SAMPLE_NAME, { type: 'application/gzip' });
    await loadPlugin(file);
  } catch (err) {
    showError(ui, err);
    setStatus(ui, 'Failed to fetch sample plugin. See error below.');
  } finally {
    ui.sampleBtn.disabled = false;
  }
}

ui.playBtn.addEventListener('click', () => {
  void onPlay();
});

async function onPlay(): Promise<void> {
  ui.playBtn.disabled = true;
  try {
    if (!current) {
      await loadRawTestTone();
    }
    if (!current) return;
    const label = current.effect ? 'plugin' : 'no plugin';
    await current.ctx.resume();
    setStatus(ui, `Playing — 440 Hz test tone (${label}).`);
    ui.stopBtn.disabled = false;
  } catch (err) {
    showError(ui, err);
    setStatus(ui, 'Failed to start audio. See error below.');
    ui.playBtn.disabled = false;
  }
}

ui.stopBtn.addEventListener('click', () => {
  if (!current) return;
  void current.ctx.suspend().then(() => {
    setStatus(ui, 'Stopped — press Play to resume.');
    ui.playBtn.disabled = false;
    ui.stopBtn.disabled = true;
  });
});

async function loadPlugin(file: File): Promise<void> {
  clearError(ui);
  setStatus(ui, `Loading ${file.name}…`);
  setPlugin(ui, '—');
  setSampleRate(ui, null);
  ui.playBtn.disabled = true;
  ui.stopBtn.disabled = true;

  await teardown();

  let blobUrl: string | null = null;

  try {
    const buf = await file.arrayBuffer();

    // Upstream `getWclap()` only unpacks `.tar.gz` bundles when handed a URL —
    // passing `{ module: arrayBuffer }` goes straight to `WebAssembly.compile`
    // and fails on the gzip magic. Sniff the header: bare wasm keeps the
    // direct path; gzip gets re-fed via a blob URL so the upstream tar.gz
    // path runs.
    const head = new Uint8Array(buf, 0, Math.min(4, buf.byteLength));
    const isWasm =
      head[0] === 0x00 &&
      head[1] === 0x61 &&
      head[2] === 0x73 &&
      head[3] === 0x6d;
    const isGzip = head[0] === 0x1f && head[1] === 0x8b;

    let node: ClapAudioNode;
    if (isWasm) {
      node = new ClapAudioNode({ module: buf });
    } else if (isGzip) {
      const blob = new Blob([buf], { type: 'application/gzip' });
      blobUrl = URL.createObjectURL(blob);
      node = new ClapAudioNode({ url: blobUrl });
    } else {
      const hex = Array.from(head, (b) => b.toString(16).padStart(2, '0')).join(
        ' '
      );
      throw new Error(
        `Unrecognized bundle format (header: ${hex}). Expected bare \`.wasm\` (00 61 73 6d) or \`.tar.gz\` (1f 8b).`
      );
    }

    setStatus(ui, 'Compiling plugin…');
    const plugins = await node.plugins();
    if (plugins.length === 0) {
      throw new Error('No CLAP plugins found in bundle.');
    }
    const first = plugins[0];
    if (!first) throw new Error('Plugin list was empty after length check.');

    const ctx = new AudioContext();
    setSampleRate(ui, ctx.sampleRate);
    wireAudioState(ctx);

    setStatus(ui, 'Starting AudioWorklet…');
    // Pre-register the worklet processor with the Vite-bundled URL. The
    // patched clap-audionode.mjs no longer attempts its own addModule call.
    await ctx.audioWorklet.addModule(workletUrl);

    const effect = await node.createNode(ctx, first.id ?? null, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [2]
    });

    const descriptor = effect.descriptor;
    const labelParts = [descriptor.name];
    if (descriptor.vendor) labelParts.push(`(${descriptor.vendor})`);
    setPlugin(ui, labelParts.join(' '));

    await buildToneGraph(ctx, effect, blobUrl);

    setStatus(ui, 'Ready — press Play to hear the test tone.');
    ui.playBtn.disabled = false;
  } catch (err) {
    showError(ui, err);
    setStatus(ui, 'Failed to load plugin. See error below.');
    if (blobUrl) URL.revokeObjectURL(blobUrl);
    await teardown();
  }
}

async function loadRawTestTone(): Promise<void> {
  clearError(ui);
  setPlugin(ui, '(no plugin — direct test tone)');
  await teardown();
  const ctx = new AudioContext();
  setSampleRate(ui, ctx.sampleRate);
  wireAudioState(ctx);
  await buildToneGraph(ctx, null, null);
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

// Build the test-tone audio graph and assign it to `current`. With an
// `effect`, the chain is `osc → merger → inGain → effect → (analysers,
// destination)`. Without one, `inGain` feeds the analysers and destination
// directly — useful for confirming audio routing without involving a plugin.
async function buildToneGraph(
  ctx: AudioContext,
  effect: ClapEffectAudioNode | null,
  blobUrl: string | null
): Promise<void> {
  // Stereo 440 Hz test tone. Two oscillators so the worklet sees two distinct
  // input channels; mono-correlated is fine for the PoC.
  const oscL = ctx.createOscillator();
  const oscR = ctx.createOscillator();
  oscL.frequency.value = 440;
  oscR.frequency.value = 440;
  oscL.type = 'sine';
  oscR.type = 'sine';

  const merger = ctx.createChannelMerger(2);
  oscL.connect(merger, 0, 0);
  oscR.connect(merger, 0, 1);

  // Plugin path attenuates so plugins with internal gain don't blow eardrums.
  // The raw path runs at a clearly audible level so a silent-speaker situation
  // is unambiguous.
  const inGain = ctx.createGain();
  inGain.gain.value = effect ? 0.25 : 0.7;
  merger.connect(inGain);

  const splitter = ctx.createChannelSplitter(2);
  const analyserL = ctx.createAnalyser();
  const analyserR = ctx.createAnalyser();
  analyserL.fftSize = 1024;
  analyserR.fftSize = 1024;

  // The node that feeds both the splitter (meters) and the destination
  // (speakers). When a plugin is loaded, that's the effect output; otherwise
  // it's the input gain — the same 440 Hz tone going straight through.
  let monitorSource: AudioNode;
  if (effect) {
    inGain.connect(effect);
    monitorSource = effect;
  } else {
    monitorSource = inGain;
  }
  monitorSource.connect(splitter);
  splitter.connect(analyserL, 0);
  splitter.connect(analyserR, 1);
  monitorSource.connect(ctx.destination);

  oscL.start();
  oscR.start();

  // Suspend so playback only starts when the user presses Play. AudioContext
  // also requires a user gesture before it can run; this matches that gate.
  await ctx.suspend();

  // Peak-meter ballistics: instant attack, ~300 ms exponential release. When
  // the AudioContext suspends (Stop), targets fall to 0 and the displayed
  // values decay smoothly instead of freezing at the last frame.
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

  current = {
    ctx,
    oscL,
    oscR,
    effect,
    analyserL,
    analyserR,
    meterTimer,
    blobUrl
  };
}

async function teardown(): Promise<void> {
  if (!current) return;
  window.clearInterval(current.meterTimer);
  try {
    current.oscL.stop();
    current.oscR.stop();
  } catch {
    // Already stopped or never started — ignore.
  }
  if (current.effect) {
    try {
      current.effect.disconnect();
    } catch {
      // Already disconnected — ignore.
    }
  }
  try {
    await current.ctx.close();
  } catch {
    // Already closed — ignore.
  }
  if (current.blobUrl) URL.revokeObjectURL(current.blobUrl);
  current = null;
  setMeters(ui, 0, 0);
  setAudioState(ui, 'idle (no context)');
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
