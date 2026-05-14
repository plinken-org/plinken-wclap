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
  merger: ChannelMergerNode;
  effect: ClapEffectAudioNode;
  analyserL: AnalyserNode;
  analyserR: AnalyserNode;
  meterTimer: number;
}

const ui = getElements();

setCoi(ui, globalThis.crossOriginIsolated === true);
setStatus(ui, 'Idle — waiting for a plugin.');

let current: RunningGraph | null = null;

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
  if (!current) return;
  void current.ctx.resume().then(() => {
    setStatus(ui, 'Playing — test tone routed through plugin.');
    ui.playBtn.disabled = true;
    ui.stopBtn.disabled = false;
  });
});

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

  try {
    const buf = await file.arrayBuffer();

    // `getWclap()` (inside ClapAudioNode) accepts a raw ArrayBuffer that's
    // either bare wasm or a tar.gz bundle. It sniffs which.
    const node = new ClapAudioNode({ module: buf });

    setStatus(ui, 'Compiling plugin…');
    const plugins = await node.plugins();
    if (plugins.length === 0) {
      throw new Error('No CLAP plugins found in bundle.');
    }
    const first = plugins[0];
    if (!first) throw new Error('Plugin list was empty after length check.');

    const ctx = new AudioContext();
    setSampleRate(ui, ctx.sampleRate);

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

    // Stereo 440 Hz test tone → ChannelMerger → effect → destination.
    // We use two oscillators so the worklet sees two distinct input channels;
    // a single OscillatorNode merged into stereo is still mono-correlated, but
    // for this PoC that's fine.
    const oscL = ctx.createOscillator();
    const oscR = ctx.createOscillator();
    oscL.frequency.value = 440;
    oscR.frequency.value = 440;
    oscL.type = 'sine';
    oscR.type = 'sine';

    const merger = ctx.createChannelMerger(2);
    oscL.connect(merger, 0, 0);
    oscR.connect(merger, 0, 1);

    // Attenuate the input so plugins with internal gain don't blow eardrums.
    const inGain = ctx.createGain();
    inGain.gain.value = 0.25;
    merger.connect(inGain);
    inGain.connect(effect);

    // Tap the effect output into two analysers (split L/R) for RMS metering,
    // then send the same signal to the speakers.
    const splitter = ctx.createChannelSplitter(2);
    const analyserL = ctx.createAnalyser();
    const analyserR = ctx.createAnalyser();
    analyserL.fftSize = 1024;
    analyserR.fftSize = 1024;
    effect.connect(splitter);
    splitter.connect(analyserL, 0);
    splitter.connect(analyserR, 1);
    effect.connect(ctx.destination);

    oscL.start();
    oscR.start();

    // Suspend so playback only starts when the user presses Play. AudioContext
    // also requires a user gesture before it can run; this matches that gate.
    await ctx.suspend();

    const meterTimer = window.setInterval(() => {
      setMeters(ui, rms(analyserL), rms(analyserR));
    }, 50);

    current = {
      ctx,
      oscL,
      oscR,
      merger,
      effect,
      analyserL,
      analyserR,
      meterTimer
    };

    setStatus(ui, 'Ready — press Play to hear the test tone.');
    ui.playBtn.disabled = false;
  } catch (err) {
    showError(ui, err);
    setStatus(ui, 'Failed to load plugin. See error below.');
    await teardown();
  }
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
  try {
    current.effect.disconnect();
  } catch {
    // Already disconnected — ignore.
  }
  try {
    await current.ctx.close();
  } catch {
    // Already closed — ignore.
  }
  current = null;
  setMeters(ui, 0, 0);
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
