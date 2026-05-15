// Minimal CBOR encode/decode helpers for the WCLAP plugin-iframe transport.
//
// The plugin <-> UI postMessage protocol uses a few fixed CBOR shapes (see
// transport.mjs). Anything richer would warrant a real CBOR library; we
// don't need one for these messages.

// text(5) "ready" — UI signals plugin it has mounted and wants a snapshot.
export function encodeReady() {
  return new Uint8Array([0x65, 0x72, 0x65, 0x61, 0x64, 0x79]).buffer;
}

// { "set": [<u32 id>, <f64 value>] } — UI tells plugin a param changed.
// 20 bytes, fixed layout.
export function encodeSet(id, value) {
  const buf = new ArrayBuffer(20);
  const view = new DataView(buf);
  view.setUint8(0, 0xa1);              // map(1)
  view.setUint8(1, 0x63);              // text(3) "set"
  view.setUint8(2, 0x73);
  view.setUint8(3, 0x65);
  view.setUint8(4, 0x74);
  view.setUint8(5, 0x82);              // array(2)
  view.setUint8(6, 0x1a);              // u32
  view.setUint32(7, id, false);
  view.setUint8(11, 0xfb);             // f64
  view.setFloat64(12, value, false);
  return buf;
}

// Decode `{ "params": { <u32>: <f64>, ... } }`. Returns Map<id, value> or null.
// Used for both initial snapshots and meter / readonly param updates.
export function decodeParamsSnapshot(ab) {
  const view = new DataView(ab);
  let p = 0;
  if (view.byteLength < 2) return null;
  if (view.getUint8(p++) !== 0xa1) return null;
  if (view.getUint8(p++) !== 0x66) return null; // text(6)
  if (view.byteLength < p + 6) return null;
  if (
    view.getUint8(p) !== 0x70 ||
    view.getUint8(p + 1) !== 0x61 ||
    view.getUint8(p + 2) !== 0x72 ||
    view.getUint8(p + 3) !== 0x61 ||
    view.getUint8(p + 4) !== 0x6d ||
    view.getUint8(p + 5) !== 0x73
  ) return null;
  p += 6;
  const head = view.getUint8(p++);
  if ((head & 0xe0) !== 0xa0) return null; // major type 5 (map)
  let count = head & 0x1f;
  if (count > 23) return null; // We only handle short counts.
  const out = new Map();
  for (let i = 0; i < count; i++) {
    if (view.getUint8(p++) !== 0x1a) return null;
    const key = view.getUint32(p, false); p += 4;
    if (view.getUint8(p++) !== 0xfb) return null;
    const val = view.getFloat64(p, false); p += 8;
    out.set(key, val);
  }
  return out;
}
