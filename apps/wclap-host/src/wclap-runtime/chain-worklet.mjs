// Plinken vocal-host chain worklet.
//
// ONE AudioWorkletNode owns the Rust host wasm AND every plugin wasm in the
// chain. The audio loop iterates loaded slots in order inside a single
// quantum, eliminating the per-AudioWorkletNode render-quantum cost
// (~2.67 ms @ 48 kHz × N plugins) that V1 paid for one-AWN-per-plugin.
//
// See apps/vocal-host/DESIGN.md for the full architecture.

import { getHost, startHost } from '@webclap/wclap-host-js';
import { hostImports } from './host-imports.mjs';
import CBOR from './cbor.mjs';

export default null;
if (!globalThis.AudioWorkletProcessor) {
  globalThis.AudioWorkletProcessor = globalThis.registerProcessor = function () {};
}

const NUM_SLOTS = 5;

class ChainProcessor extends AudioWorkletProcessor {
  /** @type {any} */ host;
  /** @type {any} */ hostApi;
  /** @type {Array<SlotState|null>} */ slots = new Array(NUM_SLOTS).fill(null);
  /** Lookup table: pluginPtr → slot index, for host-callback routing. */
  pluginToSlot = new Map();
  hostedBytes = 0;
  hostReady = false;
  hostReadyResolve;
  hostReadyPromise;
  fatalError = null;
  scratchA = null;
  scratchB = null;

  constructor(options) {
    super();
    this.maxFrames = 128;
    this.hostReadyPromise = new Promise((r) => (this.hostReadyResolve = r));

    // messageerror fires when structured-clone deserialization fails inside
    // the audio realm (most common cause: a WebAssembly.Module slipped into
    // the message — Chromium refuses to transfer them over an AudioWorklet
    // port). Post a structured 'msgerror' so main can surface it in the UI.
    this.port.onmessageerror = (e) => {
      this.port.postMessage({
        kind: 'msgerror',
        info: {
          type: e?.type,
          origin: e?.origin ?? '',
          lastEventId: e?.lastEventId ?? '',
          ports: (e?.ports ?? []).length
        }
      });
    };
    this.port.onmessage = (e) => this.onMessage(e.data);

    const opts = options?.processorOptions || {};
    if (opts.host) this.startHost(opts.host).catch((e) => this.fail(e));
  }

  async startHost(hostInit) {
    const imports = hostImports();
    Object.assign(imports.env, {
      webviewSend: (pluginPtr, ptr, length) => {
        const slot = this.lookupSlotByPlugin(pluginPtr);
        if (!slot) return;
        const bytes = new Uint8Array(slot.memory.buffer, ptr, length).slice();
        this.port.postMessage(
          { kind: 'webview', slot: slot.index, buf: bytes.buffer },
          [bytes.buffer]
        );
      },
      eventsOutTryPush: (pluginPtr, ptr, length) => {
        // Plugin emitted an event (note on/off, MIDI CC, etc). Forward it
        // to the NEXT non-bypassed slot in the chain so MIDI flows downstream.
        const slot = this.lookupSlotByPlugin(pluginPtr);
        if (!slot) return;
        const bytes = new Uint8Array(slot.memory.buffer, ptr, length).slice();
        for (let j = slot.index + 1; j < NUM_SLOTS; j++) {
          const next = this.slots[j];
          if (!next || next.bypass) continue;
          next.pendingInputEvents.push(bytes);
          break;
        }
      },
      stateMarkDirty: (pluginPtr) => {
        const slot = this.lookupSlotByPlugin(pluginPtr);
        if (slot) this.port.postMessage({ kind: 'state-dirty', slot: slot.index });
      },
      paramsRescan: (pluginPtr, flags) => {
        const slot = this.lookupSlotByPlugin(pluginPtr);
        if (slot) {
          this.port.postMessage({ kind: 'params-rescan', slot: slot.index, flags });
        }
      },
      log: (pluginPtr, severity, msgPtr, length) => {
        const slot = this.lookupSlotByPlugin(pluginPtr);
        if (!slot) return;
        const bytes = new Uint8Array(slot.memory.buffer, msgPtr, length);
        let str = '';
        for (let i = 0; i < length; ++i) str += String.fromCharCode(bytes[i]);
        if (severity >= 2) console.error(`[plugin@${slot.index}]`, str);
        else console.log(`[plugin@${slot.index}]`, str);
      }
    });

    this.host = await startHost(hostInit, imports);
    this.hostApi = this.host.hostInstance.exports;
    this.hostedBytes = this.hostApi.createBytes();
    this.hostReady = true;
    this.hostReadyResolve();
    this.port.postMessage({ kind: 'host-ready' });
  }

  lookupSlotByPlugin(pluginPtr) {
    const idx = this.pluginToSlot.get(pluginPtr);
    return idx == null ? null : this.slots[idx];
  }

  async onMessage(data) {
    if (!data || typeof data !== 'object') return;
    try {
      switch (data.kind) {
        case 'load':
          await this.handleLoad(data);
          break;
        case 'unload':
          await this.handleUnload(data);
          break;
        case 'set-param':
          this.handleSetParam(data);
          break;
        case 'set-bypass':
          this.handleSetBypass(data);
          break;
        case 'plugin-msg':
          this.handlePluginMessage(data);
          break;
        case 'midi-event':
          this.handleMidiEvent(data);
          break;
        case 'get-params':
          await this.handleGetParams(data);
          break;
        case 'save-state':
          this.handleSaveState(data);
          break;
        case 'load-state':
          this.handleLoadState(data);
          break;
        case 'get-latency':
          this.handleGetLatency(data);
          break;
        default:
          console.warn('[chain] unknown message kind:', data.kind);
      }
    } catch (e) {
      console.error('[chain] message handler error:', e);
      this.port.postMessage({
        kind: 'error',
        slot: data?.slot ?? null,
        error: String(e && e.message ? e.message : e)
      });
    }
  }

  async handleLoad({ slot, wclap, pluginId }) {
    if (!this.hostReady) await this.hostReadyPromise;
    if (slot < 0 || slot >= NUM_SLOTS) throw new Error('slot out of range: ' + slot);
    if (this.slots[slot]) await this.unloadSlot(slot);

    // Recompile the wasm module on this side — main stripped `wclap.module`
    // because Chromium refuses to transfer WebAssembly.Module over the
    // AudioWorklet port. The raw bytes live in `wclap.files[<pluginPath>/module.wasm]`.
    if (!wclap.module) {
      const wasmPath = `${wclap.pluginPath}/module.wasm`;
      const wasmBytes = wclap.files?.[wasmPath];
      if (!wasmBytes) throw new Error('handleLoad: wasm bytes missing at ' + wasmPath);
      wclap.module = await WebAssembly.compile(wasmBytes);
    }

    const wclapInstance = await this.host.startWclap(wclap, (_host, _threadData) => {
      // No worker threads in the vocal chain — plugins run single-threaded.
      return false;
    });
    const hostedPtr = this.hostApi.makeHosted(wclapInstance.ptr);
    if (!hostedPtr) throw new Error('makeHosted failed');

    // Enumerate every plugin in the bundle so the host can show a cycle
    // widget when the bundle has more than one.
    const bundleInfo = this.decodeCbor(this.hostApi.getInfo(hostedPtr, this.hostedBytes));
    const bundlePlugins = Array.isArray(bundleInfo?.plugins) ? bundleInfo.plugins : [];

    let resolvedId = pluginId;
    if (!resolvedId) {
      resolvedId = bundlePlugins[0]?.id;
      if (!resolvedId) throw new Error('bundle has no plugins');
    }

    // createPlugin takes the plugin id as a wasm-side string. The host owns
    // a scratch "bytes" pointer we re-use to ship the bytes across.
    const idBytes = new TextEncoder().encode(resolvedId);
    this.writeHostBytes(idBytes);
    const pluginPtr = this.hostApi.createPlugin(hostedPtr, this.hostedBytes);
    if (!pluginPtr) throw new Error('createPlugin failed for ' + resolvedId);

    const audioPointers = this.decodeCbor(
      this.hostApi.pluginStart(
        pluginPtr,
        globalThis.sampleRate,
        0,
        this.maxFrames,
        this.hostedBytes
      )
    );
    if (!audioPointers) throw new Error('pluginStart failed');

    const pluginInfo = this.decodeCbor(
      this.hostApi.pluginGetInfo(pluginPtr, this.hostedBytes)
    );

    const slotState = {
      index: slot,
      hostedPtr,
      pluginPtr,
      pluginId: resolvedId,
      memory: wclapInstance.memory,
      audioPointers,
      bypass: false,
      info: pluginInfo,
      /** CLAP event byte arrays waiting to be pushed to the plugin next block. */
      pendingInputEvents: []
    };
    this.slots[slot] = slotState;
    this.pluginToSlot.set(pluginPtr, slot);

    // Run one main-thread tick so the plugin can finish setup.
    this.hostApi.pluginMainThread(pluginPtr);

    this.port.postMessage({
      kind: 'loaded',
      slot,
      pluginId: resolvedId,
      info: pluginInfo,
      bundlePlugins
    });
  }

  async handleUnload({ slot }) {
    await this.unloadSlot(slot);
    this.port.postMessage({ kind: 'unloaded', slot });
  }

  async unloadSlot(slot) {
    const s = this.slots[slot];
    if (!s) return;
    this.slots[slot] = null;
    this.pluginToSlot.delete(s.pluginPtr);
    try {
      this.hostApi.destroyPlugin(s.pluginPtr);
    } catch (e) {
      console.warn('[chain] destroyPlugin threw:', e);
    }
    try {
      this.hostApi.removeHosted(s.hostedPtr);
    } catch (e) {
      console.warn('[chain] removeHosted threw:', e);
    }
  }

  handleSetParam({ slot, paramId, value }) {
    const s = this.slots[slot];
    if (!s) return;
    this.hostApi.pluginSetParam(s.pluginPtr, paramId, value);
    this.hostApi.pluginParamsFlush(s.pluginPtr);
  }

  handleSetBypass({ slot, bypass }) {
    const s = this.slots[slot];
    if (!s) return;
    s.bypass = !!bypass;
  }

  handlePluginMessage({ slot, buf }) {
    const s = this.slots[slot];
    if (!s) return;
    const bytes = new Uint8Array(buf);
    this.writeHostBytes(bytes);
    this.hostApi.pluginMessage(s.pluginPtr, this.hostedBytes);
  }

  /**
   * Queue a CLAP-encoded event for one slot (`slot >= 0`) or fan out to
   * every loaded slot (`slot === null` / undefined). Events are drained
   * at the start of the slot's next `process()` call via
   * `hostApi.pluginAcceptEvent`. Buffers are copied per-slot so each
   * plugin sees its own Uint8Array (we mutate hostedBytes when pushing,
   * so sharing one Uint8Array reference is fine — pluginAcceptEvent
   * copies into wasm memory immediately).
   */
  handleMidiEvent({ slot, buf }) {
    if (!buf) return;
    const bytes = new Uint8Array(buf);
    if (slot == null) {
      for (let i = 0; i < NUM_SLOTS; i++) {
        const s = this.slots[i];
        if (s) s.pendingInputEvents.push(bytes);
      }
    } else {
      const s = this.slots[slot];
      if (s) s.pendingInputEvents.push(bytes);
    }
  }

  async handleGetParams({ slot, requestId }) {
    const s = this.slots[slot];
    if (!s) {
      this.port.postMessage({ kind: 'params', slot, requestId, params: [] });
      return;
    }
    const params = this.decodeCbor(
      this.hostApi.pluginGetParams(s.pluginPtr, this.hostedBytes)
    );
    for (const p of params) {
      p.value = this.decodeCbor(
        this.hostApi.pluginGetParam(s.pluginPtr, p.id, this.hostedBytes)
      );
    }
    this.port.postMessage({ kind: 'params', slot, requestId, params });
  }

  /**
   * Save plugin state via clap.state.save. Bytes are returned as a fresh
   * ArrayBuffer over the port — the host base64-encodes for clipboard.
   */
  handleSaveState({ slot, requestId }) {
    const s = this.slots[slot];
    if (!s) {
      this.port.postMessage({ kind: 'state-saved', slot, requestId, ok: false, buf: null });
      return;
    }
    const ok = this.hostApi.pluginSaveState(s.pluginPtr, this.hostedBytes) !== 0;
    if (!ok) {
      this.port.postMessage({ kind: 'state-saved', slot, requestId, ok: false, buf: null });
      return;
    }
    const ptr = this.hostApi.getBytesData(this.hostedBytes);
    const len = this.hostApi.getBytesLength(this.hostedBytes);
    const bytes = new Uint8Array(this.host.hostMemory.buffer).slice(ptr, ptr + len);
    this.port.postMessage(
      { kind: 'state-saved', slot, requestId, ok: true, buf: bytes.buffer },
      [bytes.buffer]
    );
  }

  /** Load plugin state via clap.state.load. */
  handleLoadState({ slot, requestId, buf }) {
    const s = this.slots[slot];
    if (!s) {
      this.port.postMessage({ kind: 'state-loaded', slot, requestId, ok: false });
      return;
    }
    this.writeHostBytes(new Uint8Array(buf));
    const ok = this.hostApi.pluginLoadState(s.pluginPtr, this.hostedBytes) !== 0;
    this.port.postMessage({ kind: 'state-loaded', slot, requestId, ok });
  }

  /**
   * Query the plugin's reported latency via clap.latency.get. Requires the
   * host wasm to export pluginLatency (added in step "latency display").
   */
  handleGetLatency({ slot, requestId }) {
    const s = this.slots[slot];
    const latency = (s && typeof this.hostApi.pluginLatency === 'function')
      ? this.hostApi.pluginLatency(s.pluginPtr) >>> 0
      : 0;
    this.port.postMessage({ kind: 'latency', slot, requestId, latency });
  }

  decodeCbor() {
    const ptr = this.hostApi.getBytesData(this.hostedBytes);
    const len = this.hostApi.getBytesLength(this.hostedBytes);
    const bytes = new Uint8Array(this.host.hostMemory.buffer).slice(ptr, ptr + len);
    return CBOR.decode(bytes);
  }

  writeHostBytes(bytes) {
    const ptr = this.hostApi.resizeBytes(this.hostedBytes, bytes.length);
    const view = new Uint8Array(this.host.hostMemory.buffer).subarray(
      ptr,
      ptr + bytes.length
    );
    view.set(bytes);
  }

  fail(e) {
    console.error('[chain] fatal:', e);
    this.fatalError = e;
    this.port.postMessage({ kind: 'crashed', error: String(e?.message || e) });
  }

  process(inputs, outputs) {
    if (this.fatalError || !this.hostReady) return true;

    const out = outputs[0];
    const inp = inputs[0];
    if (!out || out.length === 0) return true;
    const blockLength = out[0].length;
    const channels = out.length;

    // Initialize scratch ping-pong buffers (Float32Array per channel).
    if (!this.scratchA || this.scratchA[0].length !== blockLength) {
      this.scratchA = new Array(channels)
        .fill(null)
        .map(() => new Float32Array(blockLength));
      this.scratchB = new Array(channels)
        .fill(null)
        .map(() => new Float32Array(blockLength));
    }

    // Seed scratchA with the JS input (or silence).
    for (let c = 0; c < channels; c++) {
      const src = inp && inp[c % (inp.length || 1)];
      if (src && src.length === blockLength) {
        this.scratchA[c].set(src);
      } else {
        this.scratchA[c].fill(0);
      }
    }

    let current = this.scratchA;
    let next = this.scratchB;

    for (let i = 0; i < NUM_SLOTS; i++) {
      const s = this.slots[i];
      if (!s || s.bypass) continue;

      // Copy `current` channels into this plugin's input ports/channels.
      const inputs0 = s.audioPointers.inputs[0];
      if (inputs0) {
        for (let c = 0; c < inputs0.length; c++) {
          const dst = new Float32Array(s.memory.buffer, inputs0[c], blockLength);
          dst.set(current[c % current.length]);
        }
      }

      // Drain queued input events (Web MIDI fanout + previous slot's
      // output events) into the plugin before processing audio.
      if (s.pendingInputEvents.length) {
        for (const evtBytes of s.pendingInputEvents) {
          this.writeHostBytes(evtBytes);
          this.hostApi.pluginAcceptEvent(s.pluginPtr, this.hostedBytes);
        }
        s.pendingInputEvents.length = 0;
      }

      let status;
      try {
        status = this.hostApi.pluginProcess(s.pluginPtr, blockLength);
        this.hostApi.pluginMainThread(s.pluginPtr);
      } catch (e) {
        this.fail(e);
        return true;
      }
      if (status === 0 /* CLAP_PROCESS_ERROR */) {
        console.error('[chain] plugin process error in slot', i);
        // Pass-through this slot on error.
        continue;
      }

      // Pull this plugin's outputs into `next`.
      const outputs0 = s.audioPointers.outputs[0];
      if (outputs0 && outputs0.length) {
        for (let c = 0; c < channels; c++) {
          const ptr = outputs0[c % outputs0.length];
          const src = new Float32Array(s.memory.buffer, ptr, blockLength);
          next[c].set(src);
        }
        const tmp = current;
        current = next;
        next = tmp;
      }
      // else: plugin has no outputs (rare), leave `current` as-is.
    }

    // Write `current` to JS output.
    for (let c = 0; c < channels; c++) {
      out[c].set(current[c]);
    }
    return true;
  }
}

registerProcessor('vocal-chain', ChainProcessor);
