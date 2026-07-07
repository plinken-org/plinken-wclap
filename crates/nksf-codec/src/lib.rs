//! NKSF (Native Kontrol Standard) preset-container codec.
//!
//! Pure Rust: a RIFF (`NIKS`) container with four chunks — `NISI` (summary
//! metadata, MessagePack), `NICA` (controller pages, MessagePack — reserved),
//! `PLID` (plugin id, MessagePack), `PCHK` (opaque plugin state, raw). This
//! crate is the single implementation behind the `plinken:nksf` WIT world
//! (`wit/nksf.wit`); the native rlib is used directly by the runner and by
//! these tests, and the same code compiles to a wasm component (feature
//! `component`) that the browser consumes via jco.
//!
//! It never interprets `PCHK` — for a WCLAP plugin that blob is the
//! `clap.state` PLST payload, applied by the host, not by this codec.

/// NKS PLID — which plugin a preset needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plid {
    pub clap_id: Option<String>,
    pub vst3_uid: Option<[i32; 4]>,
    pub vst_magic: Option<i32>,
}

/// NKS NISI summary — the queryable facets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NksMeta {
    pub name: String,
    pub author: String,
    pub vendor: String,
    pub comment: String,
    pub device_type: String,
    pub bankchain: Vec<String>,
    pub types: Vec<Vec<String>>,
    pub modes: Vec<String>,
    pub uuid: String,
}

/// A fully-parsed `.nksf`/`.nksfx`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub meta: NksMeta,
    pub plugin_id: Plid,
    pub pchk: Vec<u8>,
}

const CHUNK_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse NKSF container bytes into a structured value.
pub fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    if bytes.len() < 12 {
        return Err("too short to be a RIFF file".into());
    }
    if &bytes[0..4] != b"RIFF" {
        return Err("missing RIFF magic".into());
    }
    if &bytes[8..12] != b"NIKS" {
        return Err("not an NKSF file (form type != NIKS)".into());
    }

    let chunks = walk_chunks(&bytes[12..])?;

    let meta = match find(&chunks, b"NISI") {
        Some(body) => decode_nisi(body)?,
        None => return Err("missing NISI chunk".into()),
    };
    let plugin_id = match find(&chunks, b"PLID") {
        Some(body) => decode_plid(body)?,
        None => Plid::default(),
    };
    let pchk = find(&chunks, b"PCHK").map(|b| b.to_vec()).unwrap_or_default();

    Ok(Parsed { meta, plugin_id, pchk })
}

/// Encode a structured value into NKSF container bytes.
pub fn encode(value: &Parsed) -> Result<Vec<u8>, String> {
    let nisi = versioned(encode_nisi(&value.meta));
    let nica = versioned(msgpack::encode(&msgpack::Value::Map(vec![]))); // reserved
    let plid = versioned(encode_plid(&value.plugin_id));

    let mut inner = Vec::new();
    put_chunk(&mut inner, b"NISI", &nisi);
    put_chunk(&mut inner, b"NICA", &nica);
    put_chunk(&mut inner, b"PLID", &plid);
    put_chunk(&mut inner, b"PCHK", &value.pchk);

    let mut out = Vec::with_capacity(12 + inner.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + inner.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"NIKS");
    out.extend_from_slice(&inner);
    Ok(out)
}

// ---------------------------------------------------------------------------
// RIFF container
// ---------------------------------------------------------------------------

fn versioned(mut body: Vec<u8>) -> Vec<u8> {
    let mut v = CHUNK_VERSION.to_le_bytes().to_vec();
    v.append(&mut body);
    v
}

fn put_chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0); // pad to even
    }
}

/// Returns (fourcc, body) pairs. Bounds-checked; truncation is an error.
fn walk_chunks(mut data: &[u8]) -> Result<Vec<([u8; 4], &[u8])>, String> {
    let mut chunks = Vec::new();
    while data.len() >= 8 {
        let mut id = [0u8; 4];
        id.copy_from_slice(&data[0..4]);
        let size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        let body_start: usize = 8;
        let body_end = body_start
            .checked_add(size)
            .filter(|&e| e <= data.len())
            .ok_or_else(|| format!("chunk {:?} claims {size} bytes past end of file", ascii(&id)))?;
        chunks.push((id, &data[body_start..body_end]));
        let advance = body_end + (size & 1); // skip pad byte on odd sizes
        if advance > data.len() {
            break;
        }
        data = &data[advance..];
    }
    Ok(chunks)
}

fn find<'a>(chunks: &'a [([u8; 4], &'a [u8])], id: &[u8; 4]) -> Option<&'a [u8]> {
    chunks.iter().find(|(cid, _)| cid == id).map(|(_, b)| *b)
}

fn ascii(id: &[u8; 4]) -> String {
    String::from_utf8_lossy(id).into_owned()
}

fn strip_version(body: &[u8]) -> Result<&[u8], String> {
    if body.len() < 4 {
        return Err("chunk shorter than its 4-byte version prefix".into());
    }
    Ok(&body[4..])
}

// ---------------------------------------------------------------------------
// NISI / PLID  <->  MessagePack
// ---------------------------------------------------------------------------

fn encode_nisi(m: &NksMeta) -> Vec<u8> {
    use msgpack::Value::*;
    let arr = |v: &[String]| Arr(v.iter().map(|s| Str(s.clone())).collect());
    let types = Arr(m
        .types
        .iter()
        .map(|path| Arr(path.iter().map(|s| Str(s.clone())).collect()))
        .collect());
    let map = Map(vec![
        ("name".into(), Str(m.name.clone())),
        ("author".into(), Str(m.author.clone())),
        ("vendor".into(), Str(m.vendor.clone())),
        ("comment".into(), Str(m.comment.clone())),
        ("deviceType".into(), Str(m.device_type.clone())),
        ("bankchain".into(), arr(&m.bankchain)),
        ("types".into(), types),
        ("modes".into(), arr(&m.modes)),
        ("uuid".into(), Str(m.uuid.clone())),
    ]);
    msgpack::encode(&map)
}

fn decode_nisi(body: &[u8]) -> Result<NksMeta, String> {
    let pairs = as_map(msgpack::decode(strip_version(body)?)?)?;
    let mut m = NksMeta::default();
    for (k, v) in pairs {
        match k.as_str() {
            "name" => m.name = as_str(v)?,
            "author" => m.author = as_str(v)?,
            "vendor" => m.vendor = as_str(v)?,
            "comment" => m.comment = as_str(v)?,
            "deviceType" => m.device_type = as_str(v)?,
            "uuid" => m.uuid = as_str(v)?,
            "bankchain" => m.bankchain = as_str_list(v)?,
            "modes" => m.modes = as_str_list(v)?,
            "types" => {
                m.types = as_arr(v)?
                    .into_iter()
                    .map(as_str_list)
                    .collect::<Result<_, _>>()?
            }
            _ => {} // forward-compatible: ignore unknown NISI keys
        }
    }
    Ok(m)
}

fn encode_plid(p: &Plid) -> Vec<u8> {
    use msgpack::Value::*;
    let mut pairs = Vec::new();
    if let Some(id) = &p.clap_id {
        pairs.push(("CLAP.id".into(), Str(id.clone())));
    }
    // NOTE: VST3.uid / VST.magic serialization needs MessagePack integers, which
    // the container's minimal codec does not emit yet — tracked as follow-up.
    msgpack::encode(&Map(pairs))
}

fn decode_plid(body: &[u8]) -> Result<Plid, String> {
    let pairs = as_map(msgpack::decode(strip_version(body)?)?)?;
    let mut p = Plid::default();
    for (k, v) in pairs {
        if k == "CLAP.id" {
            p.clap_id = Some(as_str(v)?);
        }
    }
    Ok(p)
}

fn as_map(v: msgpack::Value) -> Result<Vec<(String, msgpack::Value)>, String> {
    match v {
        msgpack::Value::Map(p) => Ok(p),
        other => Err(format!("expected a map, got {}", other.kind())),
    }
}
fn as_arr(v: msgpack::Value) -> Result<Vec<msgpack::Value>, String> {
    match v {
        msgpack::Value::Arr(a) => Ok(a),
        other => Err(format!("expected an array, got {}", other.kind())),
    }
}
fn as_str(v: msgpack::Value) -> Result<String, String> {
    match v {
        msgpack::Value::Str(s) => Ok(s),
        msgpack::Value::Nil => Ok(String::new()),
        other => Err(format!("expected a string, got {}", other.kind())),
    }
}
fn as_str_list(v: msgpack::Value) -> Result<Vec<String>, String> {
    as_arr(v)?.into_iter().map(as_str).collect()
}

// ---------------------------------------------------------------------------
// Minimal MessagePack — the subset NISI/PLID use (nil, str, array, map).
// ---------------------------------------------------------------------------

pub mod msgpack {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Value {
        Nil,
        Str(String),
        Arr(Vec<Value>),
        Map(Vec<(String, Value)>),
    }

    impl Value {
        pub fn kind(&self) -> &'static str {
            match self {
                Value::Nil => "nil",
                Value::Str(_) => "string",
                Value::Arr(_) => "array",
                Value::Map(_) => "map",
            }
        }
    }

    pub fn encode(v: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        enc(v, &mut out);
        out
    }

    fn enc(v: &Value, out: &mut Vec<u8>) {
        match v {
            Value::Nil => out.push(0xc0),
            Value::Str(s) => {
                let b = s.as_bytes();
                let n = b.len();
                if n < 32 {
                    out.push(0xa0 | n as u8);
                } else if n < 256 {
                    out.push(0xd9);
                    out.push(n as u8);
                } else {
                    out.push(0xda);
                    out.extend_from_slice(&(n as u16).to_be_bytes());
                }
                out.extend_from_slice(b);
            }
            Value::Arr(a) => {
                write_len(out, a.len(), 0x90, 0xdc);
                for e in a {
                    enc(e, out);
                }
            }
            Value::Map(p) => {
                write_len(out, p.len(), 0x80, 0xde);
                for (k, val) in p {
                    enc(&Value::Str(k.clone()), out);
                    enc(val, out);
                }
            }
        }
    }

    fn write_len(out: &mut Vec<u8>, n: usize, fix_base: u8, ext16: u8) {
        if n < 16 {
            out.push(fix_base | n as u8);
        } else {
            out.push(ext16);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Value, String> {
        let mut r = Reader { b: bytes, pos: 0 };
        let v = r.value()?;
        Ok(v)
    }

    struct Reader<'a> {
        b: &'a [u8],
        pos: usize,
    }

    impl Reader<'_> {
        fn u8(&mut self) -> Result<u8, String> {
            let v = *self.b.get(self.pos).ok_or("unexpected end of MessagePack")?;
            self.pos += 1;
            Ok(v)
        }
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len());
            let end = end.ok_or("MessagePack length past end")?;
            let s = &self.b[self.pos..end];
            self.pos = end;
            Ok(s)
        }
        fn u16(&mut self) -> Result<usize, String> {
            let s = self.take(2)?;
            Ok(u16::from_be_bytes([s[0], s[1]]) as usize)
        }
        fn string(&mut self, n: usize) -> Result<Value, String> {
            let s = self.take(n)?;
            Ok(Value::Str(String::from_utf8_lossy(s).into_owned()))
        }
        fn array(&mut self, n: usize) -> Result<Value, String> {
            let mut a = Vec::with_capacity(n);
            for _ in 0..n {
                a.push(self.value()?);
            }
            Ok(Value::Arr(a))
        }
        fn map(&mut self, n: usize) -> Result<Value, String> {
            let mut p = Vec::with_capacity(n);
            for _ in 0..n {
                let k = match self.value()? {
                    Value::Str(s) => s,
                    other => return Err(format!("map key must be a string, got {}", other.kind())),
                };
                p.push((k, self.value()?));
            }
            Ok(Value::Map(p))
        }
        fn value(&mut self) -> Result<Value, String> {
            let tag = self.u8()?;
            match tag {
                0xc0 => Ok(Value::Nil),
                0xa0..=0xbf => {
                    let n = (tag & 0x1f) as usize;
                    self.string(n)
                }
                0xd9 => {
                    let n = self.u8()? as usize;
                    self.string(n)
                }
                0xda => {
                    let n = self.u16()?;
                    self.string(n)
                }
                0x90..=0x9f => {
                    let n = (tag & 0x0f) as usize;
                    self.array(n)
                }
                0xdc => {
                    let n = self.u16()?;
                    self.array(n)
                }
                0x80..=0x8f => {
                    let n = (tag & 0x0f) as usize;
                    self.map(n)
                }
                0xde => {
                    let n = self.u16()?;
                    self.map(n)
                }
                other => Err(format!("unsupported MessagePack tag 0x{other:02x}")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Component Model glue — off by default (needs the component toolchain).
// ---------------------------------------------------------------------------
//
// Build with: `cargo component build --release --features component`.
// The generated binding paths are finalized by wit-bindgen against wit/nksf.wit;
// this module maps them 1:1 onto the native `parse`/`encode` above.
#[cfg(feature = "component")]
mod component {
    wit_bindgen::generate!({ world: "nksf", path: "wit" });

    use exports::plinken::nksf::codec::{Guest, NksMeta as WMeta, Parsed as WParsed, Plid as WPlid};

    struct Component;

    impl Guest for Component {
        fn parse(bytes: Vec<u8>) -> Result<WParsed, String> {
            super::parse(&bytes).map(to_wit)
        }
        fn encode(value: WParsed) -> Result<Vec<u8>, String> {
            super::encode(&from_wit(value))
        }
    }

    fn to_wit(p: super::Parsed) -> WParsed {
        WParsed {
            meta: WMeta {
                name: p.meta.name,
                author: p.meta.author,
                vendor: p.meta.vendor,
                comment: p.meta.comment,
                device_type: p.meta.device_type,
                bankchain: p.meta.bankchain,
                types: p.meta.types,
                modes: p.meta.modes,
                uuid: p.meta.uuid,
            },
            plugin_id: WPlid {
                clap_id: p.plugin_id.clap_id,
                vst3_uid: p.plugin_id.vst3_uid.map(|u| u.to_vec()),
                vst_magic: p.plugin_id.vst_magic,
            },
            pchk: p.pchk,
        }
    }

    fn from_wit(w: WParsed) -> super::Parsed {
        super::Parsed {
            meta: super::NksMeta {
                name: w.meta.name,
                author: w.meta.author,
                vendor: w.meta.vendor,
                comment: w.meta.comment,
                device_type: w.meta.device_type,
                bankchain: w.meta.bankchain,
                types: w.meta.types,
                modes: w.meta.modes,
                uuid: w.meta.uuid,
            },
            plugin_id: super::Plid {
                clap_id: w.plugin_id.clap_id,
                vst3_uid: w
                    .plugin_id
                    .vst3_uid
                    .and_then(|v| <[i32; 4]>::try_from(v).ok()),
                vst_magic: w.plugin_id.vst_magic,
            },
            pchk: w.pchk,
        }
    }

    export!(Component);
}

// ---------------------------------------------------------------------------
// Tests — native, no component toolchain needed.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Parsed {
        Parsed {
            meta: NksMeta {
                name: "Synome Circuit".into(),
                author: "PLINKEN".into(),
                vendor: "PLINKEN".into(),
                comment: "Gritty mono bass".into(),
                device_type: "INST".into(),
                bankchain: vec!["Synome".into(), "Bass".into(), String::new()],
                types: vec![vec!["Synth".into(), "Bass".into()]],
                modes: vec!["Mono".into()],
                uuid: "abc-123".into(),
            },
            plugin_id: Plid { clap_id: Some("com.plinken.synome".into()), ..Default::default() },
            pchk: vec![0x50, 0x4c, 0x53, 0x54, 1, 2, 3, 4, 5], // "PLST" + junk
        }
    }

    #[test]
    fn round_trips() {
        let p = sample();
        let bytes = encode(&p).unwrap();
        let back = parse(&bytes).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn container_is_well_formed() {
        let bytes = encode(&sample()).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"NIKS");
        // declared RIFF size covers everything after the 8-byte header
        let declared = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        assert_eq!(declared, bytes.len() - 8);
        // all four chunks present, in order
        let body = &bytes[12..];
        let order: Vec<[u8; 4]> = walk_chunks(body).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, [*b"NISI", *b"NICA", *b"PLID", *b"PCHK"]);
    }

    #[test]
    fn pchk_is_preserved_verbatim() {
        let p = sample();
        let back = parse(&encode(&p).unwrap()).unwrap();
        assert_eq!(back.pchk, p.pchk);
    }

    #[test]
    fn odd_length_pchk_pads_and_recovers() {
        let mut p = sample();
        p.pchk = vec![1, 2, 3]; // odd length forces a pad byte
        let back = parse(&encode(&p).unwrap()).unwrap();
        assert_eq!(back.pchk, vec![1, 2, 3]);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = encode(&sample()).unwrap();
        bytes[1] = b'X';
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_truncated_chunk() {
        let mut bytes = encode(&sample()).unwrap();
        bytes.truncate(20); // cut mid-chunk
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn ignores_unknown_nisi_keys() {
        // Hand-build a NISI with an extra key and confirm forward-compat.
        use msgpack::Value::*;
        let map = Map(vec![
            ("name".into(), Str("X".into())),
            ("future".into(), Str("ignored".into())),
            ("modes".into(), Arr(vec![])),
            ("types".into(), Arr(vec![])),
            ("bankchain".into(), Arr(vec![])),
        ]);
        let meta = decode_nisi(&{
            let mut v = 1u32.to_le_bytes().to_vec();
            v.extend_from_slice(&msgpack::encode(&map));
            v
        })
        .unwrap();
        assert_eq!(meta.name, "X");
    }
}
