//! The `zun` command-line driver.
//!
//! Phase-1 commands:
//!
//! - `zun run <file.ts>`   — compile and execute.
//! - `zun build <file.ts>` — compile to a `<file>.zm` on disk.
//! - `zun disasm <file.ts>` — compile and print the ZIL disassembly.
//!
//! The front-end pipeline (parse → check → lower) is shared by all three; only
//! what happens to the resulting module differs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zun_common::Diagnostic;
use zun_module::{read_body, Module};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, file] if cmd == "run" => dispatch(file, Action::Run),
        [cmd, file] if cmd == "build" => dispatch(file, Action::Build { trim: false }),
        [cmd, flag, file] if cmd == "build" && flag == "--trim" => {
            dispatch(file, Action::Build { trim: true })
        }
        [cmd, file] if cmd == "disasm" => dispatch(file, Action::Disasm),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
usage: zun <command> <file.ts>
  run      compile and execute the program
  build    compile to a .zm module next to the source (use --trim to remove unreachable bodies)
  disasm   compile and print the ZIL disassembly";

#[derive(Clone, Copy)]
enum Action {
    Run,
    Build { trim: bool },
    Disasm,
}

fn dispatch(file: &str, action: Action) -> ExitCode {
    let path = Path::new(file);
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zun: cannot read {file}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let module = match zun_compiler::compile(&source) {
        Ok(m) => m,
        Err(diags) => {
            report(file, &source, &diags);
            return ExitCode::FAILURE;
        }
    };

    let module = match action {
        Action::Build { trim: true } => match zun_compiler::trim_module(&module) {
            Ok(module) => module,
            Err(error) => {
                eprintln!("zun: cannot trim module: {error}");
                return ExitCode::FAILURE;
            }
        },
        _ => module,
    };

    match action {
        Action::Run => match zun_vm::execute(&module.write()) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("zun: {e}");
                ExitCode::FAILURE
            }
        },
        Action::Build { .. } => {
            let out = out_path(path);
            let manifest_out = out.with_extension("zman");
            match std::fs::write(&out, module.write()) {
                Ok(()) => {
                    let manifest =
                        zun_loader::ModuleManifest::new(file, std::iter::empty::<String>());
                    match manifest.write_file(&manifest_out) {
                        Ok(()) => {
                            println!("wrote {}", out.display());
                            println!("wrote {}", manifest_out.display());
                            ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("zun: cannot write {}: {e}", manifest_out.display());
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(e) => {
                    eprintln!("zun: cannot write {}: {e}", out.display());
                    ExitCode::FAILURE
                }
            }
        }
        Action::Disasm => {
            print_disassembly(&module);
            ExitCode::SUCCESS
        }
    }
}

/// Print each function's name and ZIL body.
fn print_disassembly(module: &Module) {
    for method in &module.tables.method_defs {
        let name = module.strings.get(method.name);
        println!("; function {name}");
        match read_body(&module.code, method.body_offset) {
            Ok(body) => match zun_il::disassemble(body.code) {
                Ok(text) => print!("{text}"),
                Err(e) => println!("  <bad code: {e}>"),
            },
            Err(e) => println!("  <bad body: {e}>"),
        }
        println!();
    }
}

/// `foo.ts` -> `foo.zm` in the same directory.
fn out_path(source: &Path) -> PathBuf {
    source.with_extension("zm")
}

/// Render diagnostics as `file:line:col: message`, resolving spans to line and
/// column by counting newlines in the source.
fn report(file: &str, source: &str, diags: &[Diagnostic]) {
    for d in diags {
        let location = match d.span {
            Some(span) => {
                let (line, col) = line_col(source, span.start);
                format!("{file}:{line}:{col}")
            }
            None => file.to_string(),
        };
        let code = if d.code.is_empty() {
            String::new()
        } else {
            format!("[{}] ", d.code)
        };
        eprintln!("{location}: error: {code}{}", d.message);
    }
}

fn line_col(source: &str, offset: u32) -> (usize, usize) {
    let offset = offset as usize;
    let mut line = 1;
    let mut col = 1;
    for (i, c) in source.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
