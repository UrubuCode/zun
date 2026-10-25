//! The interpreter's share of the Node `Buffer` surface (the `nodeBuffer`
//! intrinsics declared in `zun_natives::node_buffer`).
//!
//! `Buffer` is a `Uint8Array` subclass in Node, so nothing here owns storage:
//! every Buffer is a `U8` typed-array view over an `ArrayBuffer`, built with
//! the same heap primitives the typed arrays use (`alloc_buffer` +
//! `alloc_typed_array`), and every read/write goes through the view's own
//! `u8_array_get`/`u8_array_set` (so a view's byte offset is respected). The
//! only Buffer-specific work is encoding/decoding and the fixed-width
//! readers/writers, assembled here from single-byte access.

use super::value::Value;
use super::RunError;
use zun_gc::Obj;
use zun_il::NumKind;

fn object(value: &Value) -> Option<Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

fn as_usize(value: Option<&Value>) -> Option<usize> {
    match value? {
        Value::Int(value, _) => Some(*value as usize),
        Value::Float(value, _) => Some(*value as usize),
        _ => None,
    }
}

fn as_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Int(value, _) => Some(*value),
        Value::Float(value, _) => Some(*value as i64),
        _ => None,
    }
}

/// Resolve a possibly-negative relative index against `len` (ES/Node slice
/// semantics): a negative index counts from the end, and the result is clamped
/// to `0..=len`.
fn relative(index: i64, len: usize) -> usize {
    if index < 0 {
        (len as i64 + index).max(0) as usize
    } else {
        (index as usize).min(len)
    }
}

fn as_string(value: Option<&Value>) -> Option<std::rc::Rc<str>> {
    match value? {
        Value::Str(text) => Some(text.clone()),
        _ => None,
    }
}

/// Build a fresh Buffer (a `U8` view over a new buffer) holding `bytes`.
fn buffer_from_bytes(bytes: &[u8]) -> Option<Value> {
    zun_gc::with_current_heap_mut(|heap| {
        let buffer = heap.alloc_buffer(bytes.len());
        for (index, byte) in bytes.iter().enumerate() {
            heap.buffer_set(buffer, index, *byte);
        }
        let view = heap.alloc_typed_array(buffer, 0, bytes.len(), zun_core::TypedElement::U8)?;
        Some(Value::Object(view))
    })
}

/// Read every byte of a `U8` view (offset-adjusted through the view).
fn view_bytes(view: Obj) -> Option<Vec<u8>> {
    zun_gc::with_current_heap(|heap| {
        let len = heap.typed_array_len(view)?;
        let mut bytes = Vec::with_capacity(len);
        for index in 0..len {
            bytes.push(heap.u8_array_get(view, index)?);
        }
        Some(bytes)
    })
}

fn view_len(view: Obj) -> Option<usize> {
    zun_gc::with_current_heap(|heap| heap.typed_array_len(view))
}

/// Normalize an encoding name. Unknown names fall back to utf8 (lenient).
fn normalize_encoding(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "hex" => "hex",
        "base64" => "base64",
        "base64url" => "base64url",
        "latin1" | "binary" => "latin1",
        // `ucs2`/`ucs-2`/`utf16le`/`utf-16le` are all aliases for little-endian
        // UTF-16 (two bytes per code unit) in Node.
        "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => "utf16le",
        // utf8/utf-8 and anything else.
        _ => "utf8",
    }
}

/// Encode a string into bytes under an encoding.
fn encode(text: &str, encoding: &str) -> Vec<u8> {
    match normalize_encoding(encoding) {
        "hex" => decode_hex(text),
        "base64" | "base64url" => decode_base64(text),
        "latin1" => text.chars().map(|c| (c as u32 & 0xff) as u8).collect(),
        "utf16le" => encode_utf16le(text),
        _ => text.as_bytes().to_vec(),
    }
}

/// Decode bytes into a string under an encoding.
fn decode(bytes: &[u8], encoding: &str) -> String {
    match normalize_encoding(encoding) {
        "hex" => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        "base64" => encode_base64(bytes),
        "base64url" => encode_base64url(bytes),
        "latin1" => bytes.iter().map(|&b| b as char).collect(),
        "utf16le" => decode_utf16le(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Little-endian UTF-16 (ucs2/utf16le): two bytes per code unit, low byte first.
fn encode_utf16le(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in text.encode_utf16() {
        out.push((unit & 0xff) as u8);
        out.push((unit >> 8) as u8);
    }
    out
}

fn decode_utf16le(bytes: &[u8]) -> String {
    // A trailing odd byte is dropped, matching Node's truncation.
    let units: Vec<u16> =
        bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn decode_hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter_map(|b| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        })
        .collect();
    digits.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0] << 4) | c[1]).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Base64url (`-`/`_`, unpadded): the URL-safe alphabet Node emits for the
/// `base64url` encoding. Padding is omitted; the decoder above is tolerant of
/// both alphabets so no separate decoder is needed.
fn encode_base64url(bytes: &[u8]) -> String {
    encode_base64(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

fn decode_base64(text: &str) -> Vec<u8> {
    let value = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            // Accept both the standard (`+`/`/`) and URL-safe (`-`/`_`)
            // alphabets so one decoder serves `base64` and `base64url`.
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    };
    let symbols: Vec<u32> = text.bytes().filter_map(value).collect();
    let mut out = Vec::new();
    for chunk in symbols.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &s)| acc | (s << (18 - 6 * i)));
        out.push((n >> 16 & 0xff) as u8);
        if chunk.len() >= 3 {
            out.push((n >> 8 & 0xff) as u8);
        }
        if chunk.len() >= 4 {
            out.push((n & 0xff) as u8);
        }
    }
    out
}

/// Read `width` bytes from `view` starting at `offset`, assembling a little- or
/// big-endian unsigned integer. Returns `None` if the range is out of bounds.
fn read_uint(view: Obj, offset: usize, width: usize, little_endian: bool) -> Option<u64> {
    zun_gc::with_current_heap(|heap| {
        let mut value: u64 = 0;
        for i in 0..width {
            let byte = heap.u8_array_get(view, offset + i)? as u64;
            let shift = if little_endian { 8 * i } else { 8 * (width - 1 - i) };
            value |= byte << shift;
        }
        Some(value)
    })
}

/// Write the low `width` bytes of `value` into `view` at `offset`. Returns the
/// next offset (`offset + width`), as Node's `write*` do.
fn write_uint(view: Obj, offset: usize, value: u64, width: usize, little_endian: bool) -> Option<usize> {
    zun_gc::with_current_heap_mut(|heap| {
        for i in 0..width {
            let shift = if little_endian { 8 * i } else { 8 * (width - 1 - i) };
            let byte = (value >> shift & 0xff) as u8;
            heap.u8_array_set(view, offset + i, byte)?;
        }
        Some(offset + width)
    })
}

/// A fixed-width reader/writer descriptor derived from the native name.
struct Accessor {
    width: usize,
    signed: bool,
    little_endian: bool,
    num_kind: NumKind,
}

fn accessor(name: &str) -> Option<Accessor> {
    // `name` is e.g. `nodeBufferReadUInt16LE` / `nodeBufferWriteInt32BE` /
    // `nodeBufferReadBigUInt64BE`.
    let tail = name
        .strip_prefix("nodeBufferRead")
        .or_else(|| name.strip_prefix("nodeBufferWrite"))?;
    // Unsigned iff the type word is `UInt`/`BigUInt`.
    let signed = !(tail.starts_with("UInt") || tail.starts_with("BigUInt"));
    let little_endian = !tail.ends_with("BE");
    let (width, num_kind) = if tail.contains("64") {
        (8, if signed { NumKind::I64 } else { NumKind::U64 })
    } else if tail.contains("32") {
        (4, if signed { NumKind::I32 } else { NumKind::U32 })
    } else if tail.contains("16") {
        (2, if signed { NumKind::I16 } else { NumKind::U16 })
    } else if tail.contains('8') {
        (1, if signed { NumKind::I8 } else { NumKind::U8 })
    } else {
        return None;
    };
    Some(Accessor { width, signed, little_endian, num_kind })
}

/// Scan `haystack` for `needle`, honoring a possibly-negative `byte_offset`
/// (negative counts from the end). `last` searches backward (for
/// `lastIndexOf`). Returns the matching byte index, or `None`.
fn buffer_search(haystack: &[u8], needle: &[u8], byte_offset: i64, last: bool) -> Option<usize> {
    let len = haystack.len();
    if needle.is_empty() {
        let off = if byte_offset < 0 {
            (len as i64 + byte_offset).max(0)
        } else {
            byte_offset.min(len as i64)
        };
        return Some(off as usize);
    }
    if needle.len() > len {
        return None;
    }
    let max_start = len - needle.len();
    if last {
        // Highest start index to consider (byteOffset defaults to the length).
        let start = if byte_offset < 0 { len as i64 + byte_offset } else { byte_offset };
        if start < 0 {
            return None;
        }
        let start = (start as usize).min(max_start);
        (0..=start).rev().find(|&i| &haystack[i..i + needle.len()] == needle)
    } else {
        let start = if byte_offset < 0 { (len as i64 + byte_offset).max(0) as usize } else { byte_offset as usize };
        if start > max_start {
            return None;
        }
        (start..=max_start).find(|&i| &haystack[i..i + needle.len()] == needle)
    }
}

/// Sign-extend an unsigned value of `width` bytes.
fn sign_extend(value: u64, width: usize) -> i64 {
    let bits = width * 8;
    if bits >= 64 {
        value as i64
    } else {
        let shift = 64 - bits;
        ((value << shift) as i64) >> shift
    }
}

/// Run a `nodeBuffer*` intrinsic, or `None` when `name` is not one.
pub(crate) fn call(name: &str, args: &[Value]) -> Option<Result<Value, RunError>> {
    match name {
        "nodeBufferFromString" => Some((|| {
            let text = as_string(args.first()).ok_or(RunError::BadToken)?;
            let encoding = as_string(args.get(1)).unwrap_or_else(|| "utf8".into());
            buffer_from_bytes(&encode(&text, &encoding)).ok_or(RunError::BadToken)
        })()),
        "nodeBufferFromArray" => Some((|| {
            let array = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let bytes = zun_gc::with_current_heap(|heap| {
                let len = heap.array_len(array)?;
                let mut bytes = Vec::with_capacity(len);
                for index in 0..len {
                    let any = heap.array_get(array, index)?;
                    let value = any
                        .as_int()
                        .or_else(|| any.as_float().map(|f| f as i64))
                        .unwrap_or(0);
                    bytes.push((value & 0xff) as u8);
                }
                Some(bytes)
            })
            .ok_or(RunError::BadToken)?;
            buffer_from_bytes(&bytes).ok_or(RunError::BadToken)
        })()),
        "nodeBufferFromArrayBuffer" => Some((|| {
            let buffer = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            zun_gc::with_current_heap_mut(|heap| {
                let len = heap.buffer_len(buffer)?;
                let view = heap.alloc_typed_array(buffer, 0, len, zun_core::TypedElement::U8)?;
                Some(Value::Object(view))
            })
            .ok_or(RunError::BadToken)
        })()),
        "nodeBufferAlloc" => Some((|| {
            let size = as_usize(args.first()).ok_or(RunError::BadToken)?;
            let fill = as_usize(args.get(1)).unwrap_or(0) as u8;
            buffer_from_bytes(&vec![fill; size]).ok_or(RunError::BadToken)
        })()),
        "nodeBufferAllocUnsafe" => Some((|| {
            let size = as_usize(args.first()).ok_or(RunError::BadToken)?;
            buffer_from_bytes(&vec![0u8; size]).ok_or(RunError::BadToken)
        })()),
        "nodeBufferConcat" => Some((|| {
            let list = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            // Collect the member views first, then read their bytes.
            let members = zun_gc::with_current_heap(|heap| {
                let len = heap.array_len(list)?;
                let mut members = Vec::with_capacity(len);
                for index in 0..len {
                    let any = heap.array_get(list, index)?;
                    let address = any.as_object()?;
                    members.push(unsafe { Obj::from_addr(address as *mut u64) });
                }
                Some(members)
            })
            .ok_or(RunError::BadToken)?;
            let mut bytes = Vec::new();
            for view in members {
                bytes.extend(view_bytes(view).ok_or(RunError::BadToken)?);
            }
            buffer_from_bytes(&bytes).ok_or(RunError::BadToken)
        })()),
        "nodeBufferIsBuffer" => Some((|| {
            let is_buffer = match object(args.first().ok_or(RunError::BadToken)?) {
                Some(view) => zun_gc::with_current_heap(|heap| {
                    heap.typed_array_element(view) == Some(zun_core::TypedElement::U8)
                }),
                None => false,
            };
            Ok(Value::Bool(is_buffer))
        })()),
        "nodeBufferByteLength" => Some((|| {
            let text = as_string(args.first()).ok_or(RunError::BadToken)?;
            let encoding = as_string(args.get(1)).unwrap_or_else(|| "utf8".into());
            Ok(Value::Int(encode(&text, &encoding).len() as i64, NumKind::U64))
        })()),
        "nodeBufferToString" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let encoding = as_string(args.get(1)).unwrap_or_else(|| "utf8".into());
            let bytes = view_bytes(view).ok_or(RunError::BadToken)?;
            let start = as_usize(args.get(2)).unwrap_or(0).min(bytes.len());
            let end = as_usize(args.get(3)).unwrap_or(bytes.len()).min(bytes.len());
            let slice = if start <= end { &bytes[start..end] } else { &[][..] };
            Ok(Value::Str(std::rc::Rc::from(decode(slice, &encoding).as_str())))
        })()),
        "nodeBufferWrite" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let text = as_string(args.get(1)).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(2)).unwrap_or(0);
            let encoding = as_string(args.get(3)).unwrap_or_else(|| "utf8".into());
            let bytes = encode(&text, &encoding);
            let len = view_len(view).ok_or(RunError::BadToken)?;
            let writable = bytes.len().min(len.saturating_sub(offset));
            zun_gc::with_current_heap_mut(|heap| {
                for i in 0..writable {
                    heap.u8_array_set(view, offset + i, bytes[i]);
                }
            });
            Ok(Value::Int(writable as i64, NumKind::U64))
        })()),
        "nodeBufferSlice" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let len = view_len(view).ok_or(RunError::BadToken)?;
            // Signed indices: negatives count from the end (Node/ES slice).
            let start = relative(as_i64(args.get(1)).unwrap_or(0), len);
            let end = relative(as_i64(args.get(2)).unwrap_or(len as i64), len);
            let end = end.max(start);
            let child = zun_gc::with_current_heap_mut(|heap| heap.typed_array_subarray(view, start, end))
                .ok_or(RunError::BadToken)?;
            Ok(Value::Object(child))
        })()),
        "nodeBufferFill" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let value = as_usize(args.get(1)).unwrap_or(0) as u8;
            let len = view_len(view).ok_or(RunError::BadToken)?;
            zun_gc::with_current_heap_mut(|heap| {
                for i in 0..len {
                    heap.u8_array_set(view, i, value);
                }
            });
            Ok(Value::Bool(true))
        })()),
        "nodeBufferEquals" => Some((|| {
            let a = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let b = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            Ok(Value::Bool(view_bytes(a) == view_bytes(b)))
        })()),
        "nodeBufferCopy" => Some((|| {
            let source = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let target = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let source_bytes = view_bytes(source).ok_or(RunError::BadToken)?;
            let target_len = view_len(target).ok_or(RunError::BadToken)?;
            // Signed indices: a negative counts from the end (NUCLEUS #21).
            // `target_start` is relative to the target length; the source
            // bounds are relative to the source length and clamped to it.
            let target_start = relative(as_i64(args.get(2)).unwrap_or(0), target_len);
            let source_start = relative(as_i64(args.get(3)).unwrap_or(0), source_bytes.len());
            let source_end = relative(
                as_i64(args.get(4)).unwrap_or(source_bytes.len() as i64),
                source_bytes.len(),
            );
            let mut copied = 0usize;
            zun_gc::with_current_heap_mut(|heap| {
                let mut si = source_start;
                while si < source_end && target_start + copied < target_len {
                    heap.u8_array_set(target, target_start + copied, source_bytes[si]);
                    si += 1;
                    copied += 1;
                }
            });
            Ok(Value::Int(copied as i64, NumKind::U64))
        })()),
        // --- Buffer gap closure (NUCLEUS #21) -----------------------------
        // IEEE-754 float/double readers/writers. The raw bit pattern rides the
        // existing fixed-width `read_uint`/`write_uint` helpers, then is
        // reinterpreted. Reads yield a JS number (`F64`); writes return the
        // next offset like the integer writers.
        "nodeBufferReadFloatLE" | "nodeBufferReadFloatBE" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(1)).unwrap_or(0);
            let le = name.ends_with("LE");
            let bits = read_uint(view, offset, 4, le).ok_or(RunError::BadToken)? as u32;
            Ok(Value::Float(f32::from_bits(bits) as f64, NumKind::F64))
        })()),
        "nodeBufferReadDoubleLE" | "nodeBufferReadDoubleBE" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(1)).unwrap_or(0);
            let le = name.ends_with("LE");
            let bits = read_uint(view, offset, 8, le).ok_or(RunError::BadToken)?;
            Ok(Value::Float(f64::from_bits(bits), NumKind::F64))
        })()),
        "nodeBufferWriteFloatLE" | "nodeBufferWriteFloatBE" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(1)).unwrap_or(0);
            let value = match args.get(2) {
                Some(Value::Float(v, _)) => *v as f32,
                Some(Value::Int(v, _)) => *v as f32,
                _ => return Err(RunError::BadToken),
            };
            let le = name.ends_with("LE");
            let next = write_uint(view, offset, value.to_bits() as u64, 4, le).ok_or(RunError::BadToken)?;
            Ok(Value::Int(next as i64, NumKind::U64))
        })()),
        "nodeBufferWriteDoubleLE" | "nodeBufferWriteDoubleBE" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(1)).unwrap_or(0);
            let value = match args.get(2) {
                Some(Value::Float(v, _)) => *v,
                Some(Value::Int(v, _)) => *v as f64,
                _ => return Err(RunError::BadToken),
            };
            let le = name.ends_with("LE");
            let next = write_uint(view, offset, value.to_bits(), 8, le).ok_or(RunError::BadToken)?;
            Ok(Value::Int(next as i64, NumKind::U64))
        })()),
        // Variable-width integer readers/writers (`readUIntLE(offset, byteLength)`
        // etc., 1..=6 bytes in Node, incl. the 24/40/48-bit cases). The width is
        // a runtime argument. Reads yield a JS number (`F64`, which holds every
        // value up to 48 bits exactly); writes return the next offset.
        "nodeBufferReadUIntLE" | "nodeBufferReadUIntBE" | "nodeBufferReadIntLE"
        | "nodeBufferReadIntBE" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(1)).unwrap_or(0);
            let width = as_usize(args.get(2)).unwrap_or(0);
            if width == 0 || width > 6 {
                return Err(RunError::BadToken);
            }
            let le = name.ends_with("LE");
            let raw = read_uint(view, offset, width, le).ok_or(RunError::BadToken)?;
            let signed = name.contains("ReadInt");
            let value = if signed { sign_extend(raw, width) as f64 } else { raw as f64 };
            Ok(Value::Float(value, NumKind::F64))
        })()),
        "nodeBufferWriteUIntLE" | "nodeBufferWriteUIntBE" | "nodeBufferWriteIntLE"
        | "nodeBufferWriteIntBE" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let offset = as_usize(args.get(1)).unwrap_or(0);
            let value = match args.get(2) {
                Some(Value::Float(v, _)) => *v as i64 as u64,
                Some(Value::Int(v, _)) => *v as u64,
                _ => return Err(RunError::BadToken),
            };
            let width = as_usize(args.get(3)).unwrap_or(0);
            if width == 0 || width > 6 {
                return Err(RunError::BadToken);
            }
            let le = name.ends_with("LE");
            let next = write_uint(view, offset, value, width, le).ok_or(RunError::BadToken)?;
            Ok(Value::Int(next as i64, NumKind::U64))
        })()),
        // In-place byte swaps. Node throws a RangeError when the length is not a
        // multiple of the element width; the project's buffer ops signal failure
        // with `RunError::BadToken`, so a length mismatch returns that here.
        "nodeBufferSwap16" | "nodeBufferSwap32" | "nodeBufferSwap64" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let width = match name {
                "nodeBufferSwap16" => 2,
                "nodeBufferSwap32" => 4,
                _ => 8,
            };
            let len = view_len(view).ok_or(RunError::BadToken)?;
            if len % width != 0 {
                return Err(RunError::BadToken);
            }
            zun_gc::with_current_heap_mut(|heap| {
                let mut base = 0;
                while base < len {
                    for i in 0..width / 2 {
                        let a = heap.u8_array_get(view, base + i)?;
                        let b = heap.u8_array_get(view, base + width - 1 - i)?;
                        heap.u8_array_set(view, base + i, b)?;
                        heap.u8_array_set(view, base + width - 1 - i, a)?;
                    }
                    base += width;
                }
                Some(())
            })
            .ok_or(RunError::BadToken)?;
            Ok(Value::Object(view))
        })()),
        // Search: `indexOf`/`lastIndexOf`/`includes`. The needle is a byte value
        // (number), a string (interpreted under the given encoding), or another
        // buffer; the optional byteOffset may be negative (counts from the end).
        "nodeBufferIndexOf" | "nodeBufferLastIndexOf" | "nodeBufferIncludes" => Some((|| {
            let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
            let haystack = view_bytes(view).ok_or(RunError::BadToken)?;
            let encoding = as_string(args.get(3)).unwrap_or_else(|| "utf8".into());
            // The needle arrives boxed to `Any`; unbox to its concrete value.
            let needle_val = match args.get(1).ok_or(RunError::BadToken)? {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                    .ok_or(RunError::BadToken)?,
                other => other.clone(),
            };
            let needle: Vec<u8> = match &needle_val {
                Value::Str(text) => encode(text, &encoding),
                Value::Int(v, _) => vec![(*v & 0xff) as u8],
                Value::Float(v, _) => vec![(*v as i64 & 0xff) as u8],
                Value::Object(obj) => view_bytes(*obj).ok_or(RunError::BadToken)?,
                _ => return Err(RunError::BadToken),
            };
            let last = name == "nodeBufferLastIndexOf";
            let offset = as_i64(args.get(2)).unwrap_or(0);
            let found = buffer_search(&haystack, &needle, offset, last);
            match name {
                "nodeBufferIncludes" => Ok(Value::Bool(found.is_some())),
                _ => Ok(Value::Int(found.map(|i| i as i64).unwrap_or(-1), NumKind::I64)),
            }
        })()),
        _ => {
            // Fixed-width readers/writers.
            if let Some(acc) = accessor(name) {
                let is_write = name.contains("Write");
                let view = match object(args.first()?) {
                    Some(view) => view,
                    None => return Some(Err(RunError::BadToken)),
                };
                if is_write {
                    let offset = as_usize(args.get(1)).unwrap_or(0);
                    let value = match args.get(2) {
                        Some(Value::Int(value, _)) => *value as u64,
                        Some(Value::Float(value, _)) => *value as i64 as u64,
                        _ => return Some(Err(RunError::BadToken)),
                    };
                    return Some(match write_uint(view, offset, value, acc.width, acc.little_endian) {
                        Some(next) => Ok(Value::Int(next as i64, NumKind::U64)),
                        None => Err(RunError::BadToken),
                    });
                }
                let offset = as_usize(args.get(1)).unwrap_or(0);
                return Some(match read_uint(view, offset, acc.width, acc.little_endian) {
                    Some(raw) => {
                        let value = if acc.signed { sign_extend(raw, acc.width) } else { raw as i64 };
                        Ok(Value::Int(value, acc.num_kind))
                    }
                    None => Err(RunError::BadToken),
                });
            }
            None
        }
    }
}
