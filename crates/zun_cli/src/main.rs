//! The `zun` command-line driver.
//!
//! Phase-1 commands:
//!
//! - `zun run <file.ts>`   — compile and execute.
//! - `zun build <file.ts>` — compile to a `<file>.zm` on disk.
//! - `zun build --prelude-cache <cache> <file.ts>` — also persist the
//!   fingerprinted standalone prelude cache; combine with `--trim` or `--aot`.
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
        [cmd, file] if cmd == "build" => dispatch(
            file,
            Action::Build {
                trim: false,
                aot: false,
                prelude_cache: None,
            },
        ),
        [cmd, flag, cache, file] if cmd == "build" && flag == "--prelude-cache" => dispatch(
            file,
            Action::Build {
                trim: false,
                aot: false,
                prelude_cache: Some(PathBuf::from(cache)),
            },
        ),
        [cmd, trim, cache_flag, cache, file]
            if cmd == "build" && trim == "--trim" && cache_flag == "--prelude-cache" =>
        {
            dispatch(
                file,
                Action::Build {
                    trim: true,
                    aot: false,
                    prelude_cache: Some(PathBuf::from(cache)),
                },
            )
        },
        [cmd, aot, cache_flag, cache, file]
            if cmd == "build" && aot == "--aot" && cache_flag == "--prelude-cache" =>
        {
            dispatch(
                file,
                Action::Build {
                    trim: true,
                    aot: true,
                    prelude_cache: Some(PathBuf::from(cache)),
                },
            )
        },
        [cmd, flag, file] if cmd == "build" && flag == "--trim" => dispatch(
            file,
            Action::Build {
                trim: true,
                aot: false,
                prelude_cache: None,
            },
        ),
        [cmd, flag, file] if cmd == "build" && flag == "--aot" => dispatch(
            file,
            Action::Build {
                trim: true,
                aot: true,
                prelude_cache: None,
            },
        ),
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
  build    compile to a .zm module (use --trim, --aot, or --prelude-cache <path>)
  disasm   compile and print the ZIL disassembly";

#[derive(Clone)]
enum Action {
    Run,
    Build {
        trim: bool,
        aot: bool,
        prelude_cache: Option<PathBuf>,
    },
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

    let cached_prelude = match &action {
        Action::Build {
            prelude_cache: Some(path),
            ..
        } => {
            let result = if path.is_file() {
                zun_compiler::read_prelude_cache(path)
            } else {
                zun_compiler::write_prelude_cache(path)
                    .and_then(|_| zun_compiler::read_prelude_cache(path))
                    .map_err(|error| error)
            };
            match result {
                Ok(module) => Some(module),
                Err(error) => {
                    eprintln!("zun: cannot load prelude cache {}: {error}", path.display());
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => None,
    };
    let module_result = match cached_prelude.as_ref() {
        Some(prelude) => zun_compiler::compile_with_cached_prelude(&source, prelude)
            .and_then(|loader| {
                let scope = loader.get("app").ok_or_else(|| {
                    vec![Diagnostic::error(
                        "cached prelude compilation did not produce app module",
                    )]
                })?;
                zun_module::Module::read(&scope.module().write())
                    .map_err(|error| vec![Diagnostic::error(error.to_string())])
            }),
        None => zun_compiler::compile(&source),
    };
    let module = match module_result {
        Ok(m) => m,
        Err(diags) => {
            report(file, &source, &diags);
            return ExitCode::FAILURE;
        }
    };

    let module = match action {
        Action::Build { trim: true, .. } => {
            let report = match zun_compiler::trim_report(&module) {
                Ok(report) => report,
                Err(error) => {
                    eprintln!("zun: cannot analyze trim: {error}");
                    return ExitCode::FAILURE;
                }
            };
            println!(
                "trim: retaining {} methods, removing {} method bodies",
                report.reachable_methods.len(),
                report.removed_methods.len()
            );
            for rid in &report.removed_methods {
                if let Some(method) = module.tables.method_defs.get(*rid as usize - 1) {
                    println!(
                        "trim: removing method #{rid} {}",
                        module.strings.get(method.name)
                    );
                }
            }
            match zun_compiler::trim_module(&module) {
                Ok(module) => module,
                Err(error) => {
                    eprintln!("zun: cannot trim module: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
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
        Action::Build { aot, prelude_cache, .. } => {
            let out = out_path(path);
            let manifest_out = out.with_extension("zman");
            if let Some(cache) = prelude_cache.as_ref() {
                println!("using {}", cache.display());
            }
            match std::fs::write(&out, module.write()) {
                Ok(()) => {
                    let dependencies = cached_prelude
                        .as_ref()
                        .map(|_| vec!["zun:prelude".to_owned()])
                        .unwrap_or_default();
                    let manifest = zun_loader::ModuleManifest::new(file, dependencies)
                    .with_prelude_fingerprint(zun_prelude::fingerprint());
                    match manifest.write_file(&manifest_out) {
                        Ok(()) => {
                            println!("wrote {}", out.display());
                            println!("wrote {}", manifest_out.display());
                            if aot {
                                let object_out = out.with_extension("o");
                                match zun_codegen::compile_object(&module) {
                                    Ok(object) => match std::fs::write(&object_out, object) {
                                        Ok(()) => {
                                            println!("wrote {}", object_out.display());
                                            let native_manifest = manifest.clone().with_native_object(
                                                object_out
                                                    .file_name()
                                                    .expect("object output has a file name")
                                                    .to_string_lossy()
                                                    .into_owned(),
                                            );
                                            if let Err(error) =
                                                native_manifest.write_file(&manifest_out)
                                            {
                                                eprintln!(
                                                    "zun: cannot update {}: {error}",
                                                    manifest_out.display()
                                                );
                                                return ExitCode::FAILURE;
                                            }
                                            let executable_out = out.with_extension("exe");
                                            match link_aot_executable(
                                                &object_out,
                                                &executable_out,
                                                module.entry_point.map(|token| token.rid()),
                                                module.entry_point.and_then(|token| {
                                                    zun_module::read_body(
                                                        &module.code,
                                                        module.tables.method_defs
                                                            [token.rid() as usize - 1]
                                                            .body_offset,
                                                    )
                                                    .ok()
                                                    .map(|body| body.return_kind)
                                                }),
                                            ) {
                                                Ok(()) => {
                                                    println!("wrote {}", executable_out.display());
                                                    ExitCode::SUCCESS
                                                }
                                                Err(e) => {
                                                    eprintln!(
                                                        "zun: cannot link AOT executable: {e}"
                                                    );
                                                    ExitCode::FAILURE
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "zun: cannot write {}: {e}",
                                                object_out.display()
                                            );
                                            ExitCode::FAILURE
                                        }
                                    },
                                    Err(e) => {
                                        eprintln!("zun: cannot emit AOT object: {e}");
                                        ExitCode::FAILURE
                                    }
                                }
                            } else {
                                ExitCode::SUCCESS
                            }
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

fn link_aot_executable(
    object: &Path,
    executable: &Path,
    entry: Option<u32>,
    return_kind: Option<u8>,
) -> Result<(), String> {
    let entry = entry.ok_or_else(|| "module has no entry point".to_string())?;
    let kind = return_kind.ok_or_else(|| "entry point has no readable body".to_string())?;
    let return_type = match zun_il::JitKind::from_byte(kind) {
        zun_il::JitKind::Num(zun_il::NumKind::F32 | zun_il::NumKind::F64) => "f64",
        zun_il::JitKind::Num(zun_il::NumKind::U64) => "u64",
        zun_il::JitKind::Num(_) | zun_il::JitKind::Bool => "i64",
        zun_il::JitKind::Object
        | zun_il::JitKind::Array
        | zun_il::JitKind::String
        | zun_il::JitKind::Other => {
            return Err("AOT launcher supports only scalar numeric returns".to_string())
        }
    };
    let launcher = object.with_extension("aot.rs");
    let source = format!(
        "extern \"C\" {{ fn m{entry}() -> {return_type}; }}\nfn main() {{ println!(\"{{}}\", unsafe {{ m{entry}() }}); }}\n"
    );
    std::fs::write(&launcher, source).map_err(|error| error.to_string())?;
    let result = std::process::Command::new("rustc")
        .arg(&launcher)
        .arg("--crate-name")
        .arg("zun_aot")
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()))
        .arg("-o")
        .arg(executable)
        .output()
        .map_err(|error| error.to_string());
    let _ = std::fs::remove_file(&launcher);
    let output = result?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
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
