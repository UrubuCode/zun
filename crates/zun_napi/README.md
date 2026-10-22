# Zun Node host

The default crate build exposes the stable `zun_host_*` C ABI without a Node
dependency. The optional `node-api` feature builds a Node addon entry point
that exports:

- `runI64(arrayBuffer)`
- `runF64(arrayBuffer)`
- `runBool(arrayBuffer)`
- `runString(arrayBuffer)`

Each function accepts an `ArrayBuffer` or Node `Buffer` containing the bytes of
a compiled `.zm` module and executes its entry point.

The C ABI also exposes `zun_host_execute_u64` for raw unsigned integer results,
and `zun_host_execute_string`, which copies a UTF-8 result
into caller-owned storage and reports the required size when the buffer is too
small. Call it first with `out = null` and `out_len = 0` to query the required
size, then allocate that many bytes and call it again to copy the result.

Status codes are exported as `ZUN_HOST_OK`, `ZUN_HOST_INVALID_ARGUMENT`,
`ZUN_HOST_VM_ERROR`, and `ZUN_HOST_BUFFER_TOO_SMALL`.

The C declarations are available in [`include/zun_napi.h`](include/zun_napi.h).

Zun source can declare a scalar native export explicitly and bind it through
the same typed `MemberRef` path used by the VM:

```ts
extern function GetCurrentProcessId(): i64 from "kernel32";
```

The compiler checks the declaration against the host-provided linker signature
and the VM validates the encoded scalar signature before dispatch. The current
direct ABI supports homogeneous `i64` and `f64` calls plus explicit
integer-style C boolean calls with up to three boolean parameters; strings,
buffers, aggregates, and callbacks remain separate generated-marshalling work.

On platforms that require an import library, provide the Node N-API library at
build time:

```powershell
$env:ZUN_NODE_LIB = 'C:\path\to\node.lib'
cargo build -p zun_napi --features node-api
```

On Unix-like systems, set `ZUN_NODE_LIB` to the corresponding `libnode.so` or
`libnode.dylib` path. `cargo check` can validate the Rust surface without the
library; producing a loadable shared library requires the host SDK.
