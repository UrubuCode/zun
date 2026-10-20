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

The C ABI also exposes `zun_host_execute_string`, which copies a UTF-8 result
into caller-owned storage and reports the required size when the buffer is too
small.

On platforms that require an import library, provide the Node N-API library at
build time:

```powershell
$env:ZUN_NODE_LIB = 'C:\path\to\node.lib'
cargo build -p zun_napi --features node-api
```

On Unix-like systems, set `ZUN_NODE_LIB` to the corresponding `libnode.so` or
`libnode.dylib` path. `cargo check` can validate the Rust surface without the
library; producing a loadable shared library requires the host SDK.
