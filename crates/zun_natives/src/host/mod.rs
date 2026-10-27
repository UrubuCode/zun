//! `zun:core` host group — stdin, raw stdout/stderr writes, process exit, and
//! ambient process state (environment, working directory, argv, tty).
//!
//! These are exposed under the single importable `zun:core` module, the same as
//! every other native group. They are intentionally kept here (not as a separate
//! `zun:*` module) so a future capability flag can hide `zun:core` as a whole
//! from untrusted libraries.
//!
//! Output/input/exit route through the host [`Sink`] so a capturing host (tests)
//! stays in control. Read-only ambient state (`env`/`cwd`/args/tty) is read
//! straight from the OS, like the clock in `zun:time`.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

/// Read one line from stdin without its trailing newline; `""` at end-of-input.
fn stdin_line(_: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Str(sink.read_line().unwrap_or_default()))
}

/// Write to stdout with no trailing newline.
fn stdout_write(args: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    sink.write_out(&args[0].display());
    Ok(NativeValue::Void)
}

/// Write to stderr with no trailing newline.
fn stderr_write(args: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    sink.write_err(&args[0].display());
    Ok(NativeValue::Void)
}

/// Terminate the process with the given exit code.
fn exit_process(args: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let code = match &args[0] {
        NativeArg::Int(n) => *n as i32,
        NativeArg::Float(f) => *f as i32,
        _ => 0,
    };
    sink.exit(code);
    Ok(NativeValue::Void)
}

/// Read an environment variable; `""` when it is unset or not valid Unicode.
fn env_get(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let name = args[0].as_str();
    Ok(NativeValue::Str(std::env::var(name).unwrap_or_default()))
}

/// Whether an environment variable is set.
fn env_has(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(std::env::var_os(args[0].as_str()).is_some()))
}

/// The process working directory (lossy UTF-8); `""` if it cannot be read.
fn cwd(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(NativeValue::Str(dir))
}

/// The number of raw process arguments (`std::env::args`), including the engine
/// binary. Program-argument forwarding is a CLI concern layered on top later.
fn arg_count(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(std::env::args().count() as i64))
}

/// The raw process argument at `index`; `""` when out of range.
fn arg_at(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let index = match &args[0] {
        NativeArg::Int(n) if *n >= 0 => *n as usize,
        _ => return Ok(NativeValue::Str(String::new())),
    };
    Ok(NativeValue::Str(std::env::args().nth(index).unwrap_or_default()))
}

/// Whether standard output is attached to a terminal.
fn is_tty(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use std::io::IsTerminal;
    Ok(NativeValue::Bool(std::io::stdout().is_terminal()))
}

/// The `zun:core` host group. Appended last in [`MODULES`](crate::MODULES) so
/// every existing dispatch id stays stable.
pub const MODULE: crate::Module = crate::Module {
    name: "host",
    functions: &[
        NativeFn { name: "stdinLine", params: &[], ret: Str, imp: stdin_line },
        NativeFn { name: "stdoutWrite", params: &[Str], ret: Void, imp: stdout_write },
        NativeFn { name: "stderrWrite", params: &[Str], ret: Void, imp: stderr_write },
        NativeFn { name: "exitProcess", params: &[I64], ret: Void, imp: exit_process },
        NativeFn { name: "envGet", params: &[Str], ret: Str, imp: env_get },
        NativeFn { name: "envHas", params: &[Str], ret: Bool, imp: env_has },
        NativeFn { name: "cwd", params: &[], ret: Str, imp: cwd },
        NativeFn { name: "argCount", params: &[], ret: I64, imp: arg_count },
        NativeFn { name: "argAt", params: &[I64], ret: Str, imp: arg_at },
        NativeFn { name: "isTty", params: &[], ret: Bool, imp: is_tty },
    ],
};
