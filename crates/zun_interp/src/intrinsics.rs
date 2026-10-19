//! Intrinsic implementations — the bodies of the `zun:fs`, `zun:io`, `zun:env`,
//! and `zun:time` modules.
//!
//! These are the runtime side of the intrinsics whose signatures and ids live
//! in `zun_intrinsics` (the analogue of .NET InternalCall bodies / Rust
//! `std::sys`). [`dispatch`] maps a stable id to its Rust implementation, taking
//! the already-popped arguments and the output sink. This is where "the code of
//! `zun:fs`" actually lives.

use crate::value::Value;
use crate::{Output, RunError};
use zun_intrinsics as ix;

/// Run the intrinsic `id` with `args`, returning its result value.
pub(crate) fn dispatch(
    id: u32,
    args: &[Value],
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    Ok(match id {
        // zun:io (and the implicit `print`)
        ix::PRINT | ix::IO_PRINTLN => {
            output.println(&args[0].display());
            Value::Void
        }

        // zun:fs
        ix::FS_READ_TEXT => {
            let text = std::fs::read_to_string(args[0].as_str())
                .map_err(|e| RunError::Native(format!("readText: {e}")))?;
            Value::Str(text.into())
        }
        ix::FS_WRITE_TEXT => {
            std::fs::write(args[0].as_str(), args[1].as_str())
                .map_err(|e| RunError::Native(format!("writeText: {e}")))?;
            Value::Void
        }
        ix::FS_EXISTS => Value::Bool(std::path::Path::new(args[0].as_str()).exists()),
        ix::FS_REMOVE => {
            std::fs::remove_file(args[0].as_str())
                .map_err(|e| RunError::Native(format!("removeFile: {e}")))?;
            Value::Void
        }

        // zun:env
        ix::ENV_GET => Value::Str(std::env::var(args[0].as_str()).unwrap_or_default().into()),

        // zun:time
        ix::TIME_NOW_MILLIS => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Value::Int(ms, zun_il::NumKind::I64)
        }

        other => return Err(RunError::Malformed(format!("unknown intrinsic {other}"))),
    })
}
