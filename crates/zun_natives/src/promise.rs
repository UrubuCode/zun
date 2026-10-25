use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "Promise intrinsic requires an interpreter heap".into(),
    ))
}

pub const MODULE: crate::Module = crate::Module {
    name: "promise",
    functions: &[
        NativeFn { name: "promiseNew", params: &[], ret: Promise, imp: unavailable },
        // `Promise.withResolvers()` → `{ promise, resolve, reject }` as a
        // dynamic object; the interpreter floor builds it (callable settlers in
        // the function slot).
        NativeFn { name: "promiseWithResolvers", params: &[], ret: Opaque, imp: unavailable },
        NativeFn {
            name: "promiseResolve",
            params: &[Promise, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "promiseReject",
            params: &[Promise, Str],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn { name: "promiseState", params: &[Promise], ret: U8, imp: unavailable },
        NativeFn { name: "promiseResult", params: &[Promise], ret: Opaque, imp: unavailable },
        NativeFn { name: "promiseIsReady", params: &[Promise], ret: Bool, imp: unavailable },
        // Chaining surface. Callbacks arrive as raw values (a function, or
        // `undefined`/`null` when absent); the interpreter floor dispatches on
        // the value, so the parameter type is `Opaque`.
        NativeFn {
            name: "promiseThen",
            params: &[Promise, Opaque, Opaque],
            ret: Promise,
            imp: unavailable,
        },
        NativeFn {
            name: "promiseCatch",
            params: &[Promise, Opaque],
            ret: Promise,
            imp: unavailable,
        },
        NativeFn {
            name: "promiseFinally",
            params: &[Promise, Opaque],
            ret: Promise,
            imp: unavailable,
        },
        // Statics `Promise.*`.
        NativeFn { name: "promiseResolveStatic", params: &[Opaque], ret: Promise, imp: unavailable },
        NativeFn { name: "promiseRejectStatic", params: &[Opaque], ret: Promise, imp: unavailable },
        NativeFn { name: "promiseAll", params: &[Array], ret: Promise, imp: unavailable },
        NativeFn { name: "promiseAllSettled", params: &[Array], ret: Promise, imp: unavailable },
        NativeFn { name: "promiseRace", params: &[Array], ret: Promise, imp: unavailable },
        NativeFn { name: "promiseAny", params: &[Array], ret: Promise, imp: unavailable },
        // `new Promise(executor)`: build a pending promise plus its resolve/reject
        // settlers (the `withResolvers` mechanism) and synchronously invoke the
        // executor with them; an executor throw rejects the promise. The executor
        // arrives as a raw value (a function), so the parameter type is `Opaque`.
        // Appended at the end of the table so existing native dispatch ids are
        // left unshifted.
        NativeFn { name: "promiseNewExecutor", params: &[Opaque], ret: Promise, imp: unavailable },
    ],
};
