// Plugin-UI ↔ host postMessage transport.
//
// Usage:
//   const t = new Transport();
//   t.onSnapshot(map => { /* map of param/meter id → value */ });
//   t.sendReady();
//   t.sendSet(paramId, value);
//
// The plugin iframe runs sandboxed; the only way to communicate with the
// audio side is postMessage to window.parent. The host forwards control
// values to/from the wasm plugin.

import { encodeReady, encodeSet, decodeParamsSnapshot } from './cbor.mjs';

export class Transport {
  constructor(target = window.parent) {
    this.target = target;
    this.snapshotListeners = new Set();
    window.addEventListener('message', this.#onMessage.bind(this));
  }

  // The UI tells the plugin "I'm mounted, send me current state".
  // Posted in a microtask so the parent has time to register its listener.
  sendReady() {
    Promise.resolve().then(() => {
      this.target.postMessage(encodeReady(), '*');
    });
  }

  // User changed a control — push to plugin.
  sendSet(id, value) {
    this.target.postMessage(encodeSet(id, value), '*');
  }

  // Register a callback for incoming snapshots (initial state + readonly
  // updates like meter values). Returns an unsubscribe function.
  onSnapshot(callback) {
    this.snapshotListeners.add(callback);
    return () => this.snapshotListeners.delete(callback);
  }

  #onMessage(event) {
    const data = event.data;
    if (!(data instanceof ArrayBuffer)) return;
    const params = decodeParamsSnapshot(data);
    if (!params) return;
    for (const cb of this.snapshotListeners) cb(params);
  }
}
