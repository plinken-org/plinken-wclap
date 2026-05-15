// Auto-Pan — a stereo auto-panner driven by a sine LFO.
//
// Parameters
//   0x1001 Speed   — LFO rate in Hz (0.1 .. 20, default 5).
//   0x1002 Wet/Dry — mix between dry input and panned signal (0 .. 1,
//                    default 1 = full wet).
//
// At wet = 1 the LFO sweeps the signal across the stereo field at the
// configured rate using equal-power panning. At wet = 0 the input passes
// through untouched.
//
// The plugin ships a small `clap.webview/3` UI (see `ui/index.html`).
// Wire protocol with the iframe is CBOR-encoded ArrayBuffers — matching
// the convention used by other WCLAPs in the ecosystem (signalsmith,
// signalsmith-cpp). We only encode/decode the two shapes we need.

import * as Clap from 'as-clap';
import { CNumPtr } from 'as-clap';

const PARAM_SPEED: u32 = 0x1001;
const PARAM_WET: u32 = 0x1002;

const TWO_PI: f32 = 6.28318530717958647692;
const HALF_PI: f32 = 1.57079632679489661923;
const SQRT2: f32 = 1.41421356237309504880;

// CBOR-encoded UTF-8 "ready" — a 6-byte ArrayBuffer the UI sends after load
// to request a state snapshot. major-type-3 (text), length-5, then bytes.
// 0x65, 'r', 'e', 'a', 'd', 'y'
const READY_BYTES: u8[] = [0x65, 0x72, 0x65, 0x61, 0x64, 0x79];

// Path to the UI entrypoint inside the wclap bundle. `_get_uri` joins this
// with `Clap.modulePath` (the host's per-instance `/plugin/<hash>...` prefix
// the bundle was extracted under) and prepends the `file:` scheme so the
// host's clap-audionode resolves it through the proxy SW.
const UI_INDEX: string = '/ui/index.html';

class AutoPan extends Clap.Plugin {
  sampleRate: f32 = 48000;
  phase: f32 = 0;

  speedTarget: f32 = 5;
  wetTarget: f32 = 1;

  speedSmoothed: f32 = 5;
  wetSmoothed: f32 = 1;

  // Cached on `pluginInit`. Null until/unless the host advertises webview.
  hostWebview: Clap.clap_host_webview | null = null;

  constructor(host: Clap.clap_host) {
    super(host);
  }

  pluginInit(): bool {
    if (!super.pluginInit()) return false;
    this.hostWebview = this.hostGetExtensionUtf8<Clap.clap_host_webview>(
      Clap.Utf8.EXT_WEBVIEW
    );
    pluginInstance = this;
    return true;
  }

  pluginDestroy(): void {
    if (pluginInstance === this) pluginInstance = null;
    super.pluginDestroy();
  }

  pluginActivate(sampleRate: f64, minFrames: u32, maxFrames: u32): bool {
    this.sampleRate = f32(sampleRate);
    this.phase = 0;
    this.speedSmoothed = this.speedTarget;
    this.wetSmoothed = this.wetTarget;
    return true;
  }

  pluginProcess(process: Clap.Process): i32 {
    const audioIn = process.audioInputs[0];
    const audioOut = process.audioOutputs[0];
    const length = process.framesCount;

    this.paramsFlush(process.inEvents, process.outEvents);

    let phase = this.phase;
    let speedSm = this.speedSmoothed;
    let wetSm = this.wetSmoothed;
    const speedTarget = this.speedTarget;
    const wetTarget = this.wetTarget;
    const sr = this.sampleRate;
    // ~20 ms one-pole smoothing so parameter changes don't click.
    const smoothCoeff: f32 = f32(1) / (f32(0.02) * sr);

    const bufLIn = audioIn.data32[0];
    const bufRIn = audioIn.data32[1];
    const bufLOut = audioOut.data32[0];
    const bufROut = audioOut.data32[1];

    for (let i: u32 = 0; i < length; i++) {
      speedSm += (speedTarget - speedSm) * smoothCoeff;
      wetSm += (wetTarget - wetSm) * smoothCoeff;

      phase += TWO_PI * speedSm / sr;
      if (phase >= TWO_PI) phase -= TWO_PI;

      const lfo = Mathf.sin(phase);
      const pos = (lfo + f32(1)) * f32(0.5); // [0, 1]
      const lGain = Mathf.cos(pos * HALF_PI) * SQRT2;
      const rGain = Mathf.sin(pos * HALF_PI) * SQRT2;

      const li = bufLIn[i];
      const ri = bufRIn[i];
      const dry = f32(1) - wetSm;

      bufLOut[i] = li * dry + li * lGain * wetSm;
      bufROut[i] = ri * dry + ri * rGain * wetSm;
    }

    this.phase = phase;
    this.speedSmoothed = speedSm;
    this.wetSmoothed = wetSm;

    return Clap.PROCESS_CONTINUE;
  }

  audioPortsCount(isInput: bool): u32 {
    return 1;
  }
  audioPortsGet(index: u32, isInput: bool, info: Clap.AudioPortInfo): bool {
    if (index > 0) return false;
    info.id = isInput ? 0 : 1;
    info.name = 'main';
    info.channelCount = 2;
    info.portType = 'stereo';
    return true;
  }

  paramsCount(): u32 {
    return 2;
  }
  paramsGetInfo(index: u32, info: Clap.ParamInfo): bool {
    if (index == 0) {
      info.id = PARAM_SPEED;
      info.name = 'Speed';
      info.flags = Clap.PARAM_IS_AUTOMATABLE;
      info.minValue = 0.1;
      info.maxValue = 20.0;
      info.defaultValue = 5.0;
      return true;
    }
    if (index == 1) {
      info.id = PARAM_WET;
      info.name = 'Wet/Dry';
      info.flags = Clap.PARAM_IS_AUTOMATABLE;
      info.minValue = 0.0;
      info.maxValue = 1.0;
      info.defaultValue = 1.0;
      return true;
    }
    return false;
  }
  paramsGetValue(id: Clap.clap_id, value: CNumPtr<f64>): bool {
    if (id == PARAM_SPEED) {
      value[0] = this.speedTarget;
      return true;
    }
    if (id == PARAM_WET) {
      value[0] = this.wetTarget;
      return true;
    }
    return false;
  }
  paramsValueToText(id: Clap.clap_id, value: f64): string | null {
    if (id == PARAM_SPEED) {
      const v = Math.round(value * 100) / 100;
      return `${v} Hz`;
    }
    if (id == PARAM_WET) {
      const v = Math.round(value * 100);
      return `${v} %`;
    }
    return null;
  }
  paramsFlush(
    inputEvents: Clap.InputEvents,
    outputEvents: Clap.OutputEvents
  ): void {
    const count = inputEvents.size();
    for (let i: u32 = 0; i < count; i++) {
      const event = inputEvents.get(i);
      if (!this.handleEvent(event)) {
        outputEvents.tryPush(event);
      }
    }
  }

  handleEvent(event: Clap.clap_event_header): bool {
    if (event._space_id != Clap.CORE_EVENT_SPACE_ID) return false;
    if (event._type == Clap.EVENT_PARAM_VALUE) {
      const v = changetype<Clap.clap_event_param_value>(event);
      if (v._param_id == PARAM_SPEED) {
        this.speedTarget = f32(v._value);
        if (this.hostState) this.hostStateMarkDirty();
        return true;
      }
      if (v._param_id == PARAM_WET) {
        this.wetTarget = f32(v._value);
        if (this.hostState) this.hostStateMarkDirty();
        return true;
      }
    }
    return false;
  }

  stateSave(ostream: Clap.OStream): bool {
    const buffer = new ArrayBuffer(8);
    store<f32>(changetype<usize>(buffer), this.speedTarget);
    store<f32>(changetype<usize>(buffer) + 4, this.wetTarget);
    const wrote = ostream.write(changetype<usize>(buffer), 8);
    return wrote == 8;
  }
  stateLoad(istream: Clap.IStream): bool {
    const buffer = new ArrayBuffer(8);
    const read = istream.read(changetype<usize>(buffer), 8);
    if (read != 8) return false;
    this.speedTarget = load<f32>(changetype<usize>(buffer));
    this.wetTarget = load<f32>(changetype<usize>(buffer) + 4);
    // Notify the open UI so its pots track loaded state.
    this.webviewSendParams();
    return true;
  }

  // Override so `clap.webview/3` and `clap.latency` are advertised. Other
  // extension IDs fall through to the parent log-and-return-zero behaviour.
  pluginGetExtensionUtf8(extIdPtr: usize): usize {
    if (Clap.equalCStr(extIdPtr, Clap.Utf8.EXT_WEBVIEW)) {
      return changetype<usize>(corePluginWebview);
    }
    if (Clap.equalCStr(extIdPtr, Clap.Utf8.EXT_LATENCY)) {
      return changetype<usize>(corePluginLatency);
    }
    return super.pluginGetExtensionUtf8(extIdPtr);
  }

  // Called from the static `corePluginWebview._get_uri` trampoline below.
  // The host's clap-audionode strips `file:` and any following slashes from
  // the URI, then asks the file proxy for whatever remains. The files map
  // is keyed under the host's per-instance `modulePath` prefix, so we have
  // to include it here.
  //
  // Two-call protocol: host may pass cap=0 to probe required length, then
  // re-call with a sized buffer. Returns the length (excluding null term).
  getUri(uriPtr: usize, capacity: u32): i32 {
    const uri = 'file:' + Clap.modulePath + UI_INDEX;
    const byteLen = String.UTF8.byteLength(uri, false);
    if (capacity > 0) {
      const writable = u32(byteLen) < capacity ? u32(byteLen) : capacity - 1;
      String.UTF8.encodeUnsafe(
        changetype<usize>(uri),
        uri.length,
        uriPtr,
        false
      );
      store<u8>(uriPtr + writable, 0); // null-terminate within the buffer
    }
    return i32(byteLen);
  }

  // We bundle UI assets in the wclap tar.gz under `/ui/...`, so the host
  // resolves them through its file map rather than this callback. Returning
  // false is correct: it tells the host "no in-wasm resource for this path."
  getResource(pathPtr: usize, mimePtr: usize, mimeCap: u32, ostream: usize): bool {
    return false;
  }

  // The UI emits CBOR ArrayBuffers; runs on the audio thread, so it's safe
  // to mutate `speedTarget`/`wetTarget` directly.
  receive(bufferPtr: usize, size: u32): bool {
    if (size == 6 && bytesEqual(bufferPtr, READY_BYTES)) {
      this.webviewSendParams();
      return true;
    }
    return decodeSetMessage(bufferPtr, size, this);
  }

  webviewSendParams(): void {
    const hw = this.hostWebview;
    if (hw == null) return;
    const buf = encodeParamsSnapshot(this.speedTarget, this.wetTarget);
    call_indirect<bool>(
      u32((hw as Clap.clap_host_webview)._send),
      this._host,
      changetype<usize>(buf),
      buf.byteLength
    );
  }
}

// Singleton: the auto-pan plugin is instantiated at most once per host
// node, and the static webview trampolines need a way to reach `this`.
// Set in `pluginInit`, cleared in `pluginDestroy`.
let pluginInstance: AutoPan | null = null;

// Single shared `clap_plugin_webview` instance. The trampolines forward
// to the active `AutoPan` via the singleton — we don't reach for the
// plugin ptr's `_plugin_data` because there is only one instance per host
// node, and `fnPtr` rejects functions with captured environments.
function webviewGetUri(_plugin: Clap.clap_plugin, uri: usize, cap: u32): i32 {
  const inst = pluginInstance;
  if (inst == null) return 0;
  return inst.getUri(uri, cap);
}
function webviewGetResource(
  _plugin: Clap.clap_plugin,
  path: usize,
  mime: usize,
  mimeCap: u32,
  stream: usize
): bool {
  const inst = pluginInstance;
  if (inst == null) return false;
  return inst.getResource(path, mime, mimeCap, stream);
}
function webviewReceive(
  _plugin: Clap.clap_plugin,
  buffer: usize,
  size: u32
): bool {
  const inst = pluginInstance;
  if (inst == null) return false;
  return inst.receive(buffer, size);
}
const corePluginWebview = new Clap.clap_plugin_webview();
corePluginWebview._get_uri = Clap.fnPtr(webviewGetUri);
corePluginWebview._get_resource = Clap.fnPtr(webviewGetResource);
corePluginWebview._receive = Clap.fnPtr(webviewReceive);

// Feedback-mode auto-panner: no lookahead, no internal delay → 0 samples.
function latencyGet(_plugin: Clap.clap_plugin): u32 {
  return 0;
}
const corePluginLatency = new Clap.clap_plugin_latency();
corePluginLatency._get = Clap.fnPtr(latencyGet);

// --- tiny CBOR helpers ---------------------------------------------------
//
// We only handle the two message shapes used by the auto-pan UI, so the
// codec is hand-rolled rather than pulling in a general CBOR library.
//
// Outgoing (plugin → UI):
//   { params: { 0x1001: <f64>, 0x1002: <f64> } }
//
// Incoming (UI → plugin):
//   "ready"                                    — request a snapshot
//   { set: [<u32 paramId>, <f64 value>] }      — user moved a pot
//
// Anything we don't recognise is silently dropped.

@inline function bytesEqual(ptr: usize, ref: u8[]): bool {
  const n = ref.length;
  for (let i = 0; i < n; i++) {
    if (load<u8>(ptr + i) != ref[i]) return false;
  }
  return true;
}

// Encode `{ "params": { 0x1001: <speed>, 0x1002: <wet> } }`.
// Fixed shape, fixed size: 37 bytes.
//   a1                       map(1)
//   66 'params'              text(6) "params"
//   a2                       map(2)
//   1a 00 00 10 01           u32(0x1001)
//   fb <8 BE bytes>          f64 speed
//   1a 00 00 10 02           u32(0x1002)
//   fb <8 BE bytes>          f64 wet
function encodeParamsSnapshot(speed: f32, wet: f32): ArrayBuffer {
  const buf = new ArrayBuffer(37);
  const p = changetype<usize>(buf);
  store<u8>(p + 0, 0xa1);
  store<u8>(p + 1, 0x66);
  store<u8>(p + 2, 0x70); // p
  store<u8>(p + 3, 0x61); // a
  store<u8>(p + 4, 0x72); // r
  store<u8>(p + 5, 0x61); // a
  store<u8>(p + 6, 0x6d); // m
  store<u8>(p + 7, 0x73); // s
  store<u8>(p + 8, 0xa2);
  store<u8>(p + 9, 0x1a);
  storeU32BE(p + 10, PARAM_SPEED);
  store<u8>(p + 14, 0xfb);
  storeF64BE(p + 15, f64(speed));
  store<u8>(p + 23, 0x1a);
  storeU32BE(p + 24, PARAM_WET);
  store<u8>(p + 28, 0xfb);
  storeF64BE(p + 29, f64(wet));
  return buf;
}

@inline function storeU32BE(p: usize, v: u32): void {
  store<u8>(p + 0, u8(v >> 24));
  store<u8>(p + 1, u8(v >> 16));
  store<u8>(p + 2, u8(v >> 8));
  store<u8>(p + 3, u8(v));
}

@inline function loadU32BE(p: usize): u32 {
  return (
    (u32(load<u8>(p + 0)) << 24) |
    (u32(load<u8>(p + 1)) << 16) |
    (u32(load<u8>(p + 2)) << 8) |
    u32(load<u8>(p + 3))
  );
}

@inline function storeF64BE(p: usize, v: f64): void {
  const bits = reinterpret<u64>(v);
  store<u8>(p + 0, u8(bits >> 56));
  store<u8>(p + 1, u8(bits >> 48));
  store<u8>(p + 2, u8(bits >> 40));
  store<u8>(p + 3, u8(bits >> 32));
  store<u8>(p + 4, u8(bits >> 24));
  store<u8>(p + 5, u8(bits >> 16));
  store<u8>(p + 6, u8(bits >> 8));
  store<u8>(p + 7, u8(bits));
}

@inline function loadF64BE(p: usize): f64 {
  const bits: u64 =
    (u64(load<u8>(p + 0)) << 56) |
    (u64(load<u8>(p + 1)) << 48) |
    (u64(load<u8>(p + 2)) << 40) |
    (u64(load<u8>(p + 3)) << 32) |
    (u64(load<u8>(p + 4)) << 24) |
    (u64(load<u8>(p + 5)) << 16) |
    (u64(load<u8>(p + 6)) << 8) |
    u64(load<u8>(p + 7));
  return reinterpret<f64>(bits);
}

// Recognise `{ set: [<u32 paramId>, <f64 value>] }` and apply it. Anything
// else returns false and the plugin treats the message as unhandled.
// Expected wire bytes (20 total):
//   a1                       map(1)        — 1 B
//   63 's' 'e' 't'           text(3) "set" — 4 B
//   82                       array(2)      — 1 B
//   1a <4 BE bytes>          u32 paramId   — 5 B
//   fb <8 BE bytes>          f64 value     — 9 B
function decodeSetMessage(p: usize, size: u32, plug: AutoPan): bool {
  if (size != 20) return false;
  if (load<u8>(p + 0) != 0xa1) return false;
  if (load<u8>(p + 1) != 0x63) return false;
  if (load<u8>(p + 2) != 0x73) return false; // s
  if (load<u8>(p + 3) != 0x65) return false; // e
  if (load<u8>(p + 4) != 0x74) return false; // t
  if (load<u8>(p + 5) != 0x82) return false;
  if (load<u8>(p + 6) != 0x1a) return false;
  const id = loadU32BE(p + 7);
  if (load<u8>(p + 11) != 0xfb) return false;
  const value = loadF64BE(p + 12);
  if (id == PARAM_SPEED) {
    const clamped = f32(Math.max(0.1, Math.min(20.0, value)));
    plug.speedTarget = clamped;
    if (plug.hostState) plug.hostStateMarkDirty();
    return true;
  }
  if (id == PARAM_WET) {
    const clamped = f32(Math.max(0.0, Math.min(1.0, value)));
    plug.wetTarget = clamped;
    if (plug.hostState) plug.hostStateMarkDirty();
    return true;
  }
  return false;
}

const spec = Clap.registerPlugin<AutoPan>('Auto-Pan', 'com.plinken.auto-pan');
spec.vendor = 'Plinken';
spec.description = 'Stereo auto-panner with sine LFO.';
// Feature tags as listed in plugin-features.h. Audio-effect for the type,
// utility because this is a simple insert effect.
spec.features = ['audio-effect', 'utility'];
