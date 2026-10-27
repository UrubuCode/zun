//! `time` group of `zun:core` — clocks.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn now_millis(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(NativeValue::Int(ms))
}

/// The `time` group of `zun:core`.
pub const MODULE: crate::Module = crate::Module {
    name: "time",
    functions: &[NativeFn {
        name: "nowMillis",
        params: &[],
        ret: I64,
        imp: now_millis,
    }],
};
