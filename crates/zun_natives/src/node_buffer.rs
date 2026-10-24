//! Node-compatible `Buffer` intrinsics (`nodeBuffer` namespace of `zun:core`).
//!
//! `Buffer` is a `Uint8Array` subclass in Node, so these intrinsics do not own
//! any storage of their own: they allocate and operate over the existing
//! byte-view floor (an `ArrayBuffer` with a `Uint8Array` view, the `U8` kind of
//! the master table in `zun_core`). The signatures here fix the dispatch ids;
//! the implementations live on the interpreter's managed floor
//! (`zun_interp/src/collections.rs`, routed by name through
//! `managed_node_buffer_builtin`), exactly like the other allocating/throwing
//! builtins. Every impl is therefore `unavailable` here.
//!
//! This module is appended **last** in the registry (`lib.rs` `MODULES`), so
//! adding it keeps every existing dispatch id stable.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "managed Buffer intrinsic requires an interpreter heap".into(),
    ))
}

pub const MODULE: crate::Module = crate::Module {
    name: "nodeBuffer",
    functions: &[
        // --- statics -------------------------------------------------------
        // `Buffer.from(string, encoding?)` — encoding defaults are applied by
        // the checker (it passes "utf8" when omitted).
        NativeFn { name: "nodeBufferFromString", params: &[Str, Str], ret: U8Array, imp: unavailable },
        // `Buffer.from(array)` — an array of byte values.
        NativeFn { name: "nodeBufferFromArray", params: &[Array], ret: U8Array, imp: unavailable },
        // `Buffer.from(arrayBuffer)` — a view over the whole buffer.
        NativeFn { name: "nodeBufferFromArrayBuffer", params: &[Buffer], ret: U8Array, imp: unavailable },
        // `Buffer.alloc(size, fill?)` — zero-filled, or filled with a byte.
        NativeFn { name: "nodeBufferAlloc", params: &[U64, U8], ret: U8Array, imp: unavailable },
        // `Buffer.allocUnsafe(size)` — uninitialized in Node; zero-filled here.
        NativeFn { name: "nodeBufferAllocUnsafe", params: &[U64], ret: U8Array, imp: unavailable },
        // `Buffer.concat(list)` — the list is an Array of Buffers.
        NativeFn { name: "nodeBufferConcat", params: &[Array], ret: U8Array, imp: unavailable },
        // `Buffer.isBuffer(x)` — true for a Uint8Array byte view.
        NativeFn { name: "nodeBufferIsBuffer", params: &[Opaque], ret: Bool, imp: unavailable },
        // `Buffer.byteLength(string, encoding?)`.
        NativeFn { name: "nodeBufferByteLength", params: &[Str, Str], ret: U64, imp: unavailable },

        // --- instance ------------------------------------------------------
        // `buf.toString(encoding?, start?, end?)`.
        NativeFn { name: "nodeBufferToString", params: &[U8Array, Str, U64, U64], ret: Str, imp: unavailable },
        // `buf.write(string, offset?, encoding?)` -> bytes written.
        NativeFn { name: "nodeBufferWrite", params: &[U8Array, Str, U64, Str], ret: U64, imp: unavailable },
        // `buf.slice(start?, end?)` / `buf.subarray(...)` — shares memory.
        NativeFn { name: "nodeBufferSlice", params: &[U8Array, U64, U64], ret: U8Array, imp: unavailable },
        // `buf.fill(value)` — fill the whole buffer with a byte.
        NativeFn { name: "nodeBufferFill", params: &[U8Array, U8], ret: Bool, imp: unavailable },
        // `buf.equals(other)`.
        NativeFn { name: "nodeBufferEquals", params: &[U8Array, U8Array], ret: Bool, imp: unavailable },
        // `buf.copy(target, targetStart?, sourceStart?, sourceEnd?)` -> copied.
        NativeFn { name: "nodeBufferCopy", params: &[U8Array, U8Array, U64, U64, U64], ret: U64, imp: unavailable },

        // Fixed-width readers/writers (common LE/BE 8/16/32/64 set). A reader
        // is `(view, offset) -> elem`; a writer `(view, offset, value) -> u64`
        // (the next offset, as Node returns).
        NativeFn { name: "nodeBufferReadUInt8", params: &[U8Array, U64], ret: U8, imp: unavailable },
        NativeFn { name: "nodeBufferWriteUInt8", params: &[U8Array, U64, U8], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadInt8", params: &[U8Array, U64], ret: I8, imp: unavailable },
        NativeFn { name: "nodeBufferWriteInt8", params: &[U8Array, U64, I8], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadUInt16LE", params: &[U8Array, U64], ret: U16, imp: unavailable },
        NativeFn { name: "nodeBufferWriteUInt16LE", params: &[U8Array, U64, U16], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadUInt16BE", params: &[U8Array, U64], ret: U16, imp: unavailable },
        NativeFn { name: "nodeBufferWriteUInt16BE", params: &[U8Array, U64, U16], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadInt16LE", params: &[U8Array, U64], ret: I16, imp: unavailable },
        NativeFn { name: "nodeBufferWriteInt16LE", params: &[U8Array, U64, I16], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadInt16BE", params: &[U8Array, U64], ret: I16, imp: unavailable },
        NativeFn { name: "nodeBufferWriteInt16BE", params: &[U8Array, U64, I16], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadUInt32LE", params: &[U8Array, U64], ret: U32, imp: unavailable },
        NativeFn { name: "nodeBufferWriteUInt32LE", params: &[U8Array, U64, U32], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadUInt32BE", params: &[U8Array, U64], ret: U32, imp: unavailable },
        NativeFn { name: "nodeBufferWriteUInt32BE", params: &[U8Array, U64, U32], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadInt32LE", params: &[U8Array, U64], ret: I32, imp: unavailable },
        NativeFn { name: "nodeBufferWriteInt32LE", params: &[U8Array, U64, I32], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadInt32BE", params: &[U8Array, U64], ret: I32, imp: unavailable },
        NativeFn { name: "nodeBufferWriteInt32BE", params: &[U8Array, U64, I32], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadBigInt64LE", params: &[U8Array, U64], ret: I64, imp: unavailable },
        NativeFn { name: "nodeBufferWriteBigInt64LE", params: &[U8Array, U64, I64], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadBigInt64BE", params: &[U8Array, U64], ret: I64, imp: unavailable },
        NativeFn { name: "nodeBufferWriteBigInt64BE", params: &[U8Array, U64, I64], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadBigUInt64LE", params: &[U8Array, U64], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferWriteBigUInt64LE", params: &[U8Array, U64, U64], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferReadBigUInt64BE", params: &[U8Array, U64], ret: U64, imp: unavailable },
        NativeFn { name: "nodeBufferWriteBigUInt64BE", params: &[U8Array, U64, U64], ret: U64, imp: unavailable },
    ],
};
