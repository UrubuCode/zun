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
use std::process::{Command, ExitCode};

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
                preserve_roots: None,
            },
        ),
        [cmd, flag, cache, file] if cmd == "build" && flag == "--prelude-cache" => dispatch(
            file,
            Action::Build {
                trim: false,
                aot: false,
                prelude_cache: Some(PathBuf::from(cache)),
                preserve_roots: None,
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
                    preserve_roots: None,
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
                    preserve_roots: None,
                },
            )
        },
        [cmd, flag, file] if cmd == "build" && flag == "--trim" => dispatch(
            file,
            Action::Build {
                trim: true,
                aot: false,
                prelude_cache: None,
                preserve_roots: None,
            },
        ),
        [cmd, flag, file] if cmd == "build" && flag == "--aot" => dispatch(
            file,
            Action::Build {
                trim: true,
                aot: true,
                prelude_cache: None,
                preserve_roots: None,
            },
        ),
        [cmd, trim, roots_flag, roots, file]
            if cmd == "build" && trim == "--trim" && roots_flag == "--preserve-roots" =>
        {
            dispatch(
                file,
                Action::Build {
                    trim: true,
                    aot: false,
                    prelude_cache: None,
                    preserve_roots: Some(PathBuf::from(roots)),
                },
            )
        }
        [cmd, aot, roots_flag, roots, file]
            if cmd == "build" && aot == "--aot" && roots_flag == "--preserve-roots" =>
        {
            dispatch(
                file,
                Action::Build {
                    trim: true,
                    aot: true,
                    prelude_cache: None,
                    preserve_roots: Some(PathBuf::from(roots)),
                },
            )
        }
        [cmd, trim, aot, roots_flag, roots, file]
            if cmd == "build"
                && trim == "--trim"
                && aot == "--aot"
                && roots_flag == "--preserve-roots" =>
        {
            dispatch(
                file,
                Action::Build {
                    trim: true,
                    aot: true,
                    prelude_cache: None,
                    preserve_roots: Some(PathBuf::from(roots)),
                },
            )
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
  build    compile to a .zm module (use --trim, --aot, --preserve-roots <path>, or --prelude-cache <path>)
  disasm   compile and print the ZIL disassembly";

#[derive(Clone)]
enum Action {
    Run,
    Build {
        trim: bool,
        aot: bool,
        prelude_cache: Option<PathBuf>,
        preserve_roots: Option<PathBuf>,
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
        Some(prelude) => zun_compiler::compile_with_cached_std(&source, prelude)
            .and_then(|loader| {
                let scope = loader.get("app").ok_or_else(|| {
                    vec![Diagnostic::error(
                        "cached prelude compilation did not produce app module",
                    )]
                })?;
                zun_module::Module::read(&scope.module().write())
                    .map_err(|error| vec![Diagnostic::error(error.to_string())])
            }),
        None => zun_compiler::compile_with_default_std(&source),
    };
    let module = match module_result {
        Ok(m) => m,
        Err(diags) => {
            report(file, &source, &diags);
            return ExitCode::FAILURE;
        }
    };

    let preserve_roots = match &action {
        Action::Build { preserve_roots: Some(path), .. } => match read_preserve_roots(path) {
            Ok(roots) => roots,
            Err(error) => {
                eprintln!("zun: cannot read preserve roots {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        },
        _ => Vec::new(),
    };

    let module = match action {
        Action::Build { trim: true, .. } => {
            let report = match zun_compiler::trim_report_with_roots(&module, &preserve_roots) {
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
            if report.dynamic_fallback {
                eprintln!(
                    "zun: warning: unresolved member calls require embedded-IL interpreter fallback"
                );
            }
            for rid in &report.removed_methods {
                if let Some(method) = module.tables.method_defs.get(*rid as usize - 1) {
                    println!(
                        "trim: removing method #{rid} {}",
                        module.strings.get(method.name)
                    );
                }
            }
            match zun_compiler::trim_module_with_roots(&module, &preserve_roots) {
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
                        .map(|_| vec!["zun:std".to_owned()])
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
                                            let metadata_out = out.with_extension("meta.o");
                                            let metadata = match zun_codegen::compile_metadata_object(&module.write()) {
                                                Ok(metadata) => metadata,
                                                Err(error) => {
                                                    eprintln!("zun: cannot emit AOT metadata object: {error}");
                                                    return ExitCode::FAILURE;
                                                }
                                            };
                                            if let Err(error) = std::fs::write(&metadata_out, metadata) {
                                                eprintln!("zun: cannot write {}: {error}", metadata_out.display());
                                                return ExitCode::FAILURE;
                                            }
                                            println!("wrote {}", metadata_out.display());
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
                                                Some(&metadata_out),
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

fn read_preserve_roots(path: &Path) -> Result<Vec<u32>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|(line, value)| {
            value.trim().parse::<u32>().map_err(|error| {
                format!("invalid method RID on line {}: {error}", line + 1)
            })
        })
        .collect()
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

/// Resolve an external LLD linker without making it a mandatory host
/// dependency. `ZUN_LLD` is an explicit override; otherwise use the copy
/// shipped with the active Rust toolchain when available.
fn resolve_lld() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ZUN_LLD") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_owned()))?;
    let mut candidate = sysroot.join("bin").join("rust-lld");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    candidate.is_file().then_some(candidate)
}

fn link_aot_executable(
    object: &Path,
    metadata: Option<&Path>,
    executable: &Path,
    entry: Option<u32>,
    return_kind: Option<u8>,
) -> Result<(), String> {
    let entry = entry.ok_or_else(|| "module has no entry point".to_string())?;
    let kind = return_kind.ok_or_else(|| "entry point has no readable body".to_string())?;
    let (return_type, print_value) = match zun_il::JitKind::from_byte(kind) {
        zun_il::JitKind::Num(zun_il::NumKind::F32) => ("f32", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::F64) => ("f64", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::I8) => ("i8", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::U8) => ("u8", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::I16) => ("i16", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::U16) => ("u16", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::I32) => ("i32", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::U32) => ("u32", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::I64) => ("i64", "value"),
        zun_il::JitKind::Num(zun_il::NumKind::U64) => ("u64", "value"),
        zun_il::JitKind::Bool => ("i32", "value != 0"),
        zun_il::JitKind::Object
        | zun_il::JitKind::Array
        | zun_il::JitKind::String
        | zun_il::JitKind::Other => {
            return Err("AOT launcher supports only scalar numeric returns".to_string())
        }
    };
    let launcher = object.with_extension("aot.rs");
    let source = format!(
        "extern \"C\" {{ fn m{entry}() -> {return_type}; }}\nfn main() {{ let value = unsafe {{ m{entry}() }}; println!(\"{{}}\", {print_value}); }}\n"
    );
    std::fs::write(&launcher, source).map_err(|error| error.to_string())?;
    let mut command = Command::new("rustc");
    command
        .arg(&launcher)
        .arg("--crate-name")
        .arg("zun_aot")
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()))
        .arg("-o")
        .arg(executable);
    if let Some(metadata) = metadata {
        command.arg("-C").arg(format!("link-arg={}", metadata.display()));
    }
    if let Some(lld) = resolve_lld() {
        command.arg("-C").arg(format!("linker={}", lld.display()));
    }
    let result = command
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
