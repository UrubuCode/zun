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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use zun_common::Diagnostic;
use zun_module::{read_body, Module};

fn aot_target() -> Result<target_lexicon::Triple, String> {
    parse_aot_target(std::env::var("ZUN_TARGET").ok())
}

fn parse_aot_target(value: Option<String>) -> Result<target_lexicon::Triple, String> {
    match value {
        Some(value) => value
            .parse()
            .map_err(|error| format!("invalid ZUN_TARGET `{value}`: {error}")),
        None => Ok(target_lexicon::Triple::host()),
    }
}

fn main() -> ExitCode {
    // The compiler runs recursive passes (parse/sema/codegen walk nested
    // expressions and the growing prelude), and the OS default main-thread stack
    // is small on some platforms (~1 MiB on Windows), which overflows as the
    // nucleus prelude grows. Run the real work on a worker thread with a generous
    // stack, the same pattern rustc uses. The size is reserved address space, not
    // committed memory, so a large reserve is cheap.
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run)
        .expect("spawn zun worker thread")
        .join()
        .unwrap_or(ExitCode::FAILURE)
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(home) = zun_cli::zun_home() {
        zun_cli::cleanup_after_upgrade(&home);
    }
    match args.as_slice() {
        [cmd, flag, path] if cmd == "upgrade" && flag == "--from" => {
            match zun_cli::upgrade(Path::new(path)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("zun upgrade: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [cmd] if cmd == "install" => match zun_cli::install() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zun install: {error}");
                ExitCode::FAILURE
            }
        },
        [cmd, file] if cmd == "run" && file.ends_with(".zm") => run_zm(file),
        [cmd, flag, ext, file] if cmd == "run" && flag == "--ext" => {
            run_with_extensions(file, &[ext.as_str()])
        }
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
        }
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
        }
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

/// `zun run --ext <lib> <file>`: run a program with a loaded `zun_abi` native
/// extension (e.g. the `zun-node` library) answering its external module
/// imports. The extension's registered surface is turned into compiler external
/// declarations, so the program can import from the modules it provides
/// (`node:fs`, `fs`, …) with no hardcoded knowledge in the toolchain.
///
/// # Safety
/// Loading an extension runs trusted native code during registration; `--ext`
/// must name a trusted library.
fn run_with_extensions(file: &str, ext_paths: &[&str]) -> ExitCode {
    let path = Path::new(file);
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("zun: cannot read {file}: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Load every requested extension into one resolver.
    let mut resolver = zun_vm::ExtensionResolver::new();
    for ext in ext_paths {
        let absolute = match std::path::absolute(ext) {
            Ok(absolute) => absolute,
            Err(error) => {
                eprintln!("zun: cannot resolve extension path {ext}: {error}");
                return ExitCode::FAILURE;
            }
        };
        // SAFETY: `--ext` names a trusted extension library per the contract above.
        if let Err(error) = unsafe { resolver.load_path(&absolute) } {
            eprintln!("zun: cannot load extension {ext}: {error}");
            return ExitCode::FAILURE;
        }
    }

    // Derive the compiler's external surface from what the extensions registered.
    let externals = zun_compiler::external_functions_all(resolver.functions());

    let oxc_typescript = matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("mts") | Some("tsx")
    );
    let use_oxc = oxc_typescript
        || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("js") | Some("mjs") | Some("cjs") | Some("jsx")
        );
    let module_result = if use_oxc {
        zun_compiler::compile_with_oxc_and_externals(&source, oxc_typescript, &externals)
    } else {
        zun_compiler::compile_with_externals(&source, &externals)
    };
    let module = match module_result {
        Ok(module) => module,
        Err(diags) => {
            report(file, &source, &diags);
            return ExitCode::FAILURE;
        }
    };

    let hook = zun_vm::ExtensionCallHook::new(resolver);
    match zun_vm::execute_with_extensions_to_stdout(&module.write(), &hook) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zun: {error}");
            ExitCode::FAILURE
        }
    }
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
    let oxc_typescript = matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("mts") | Some("tsx")
    );
    let use_oxc = oxc_typescript
        || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("js") | Some("mjs") | Some("cjs") | Some("jsx")
        );
    let module_result = if use_oxc {
        if cached_prelude.is_some() {
            Err(vec![Diagnostic::error(
                "native OXC JavaScript input cannot be combined with a prelude cache",
            )])
        } else {
            zun_compiler::compile_with_oxc(&source, oxc_typescript)
        }
    } else {
        match cached_prelude.as_ref() {
            Some(prelude) => {
                zun_compiler::compile_with_cached_std(&source, prelude).and_then(|loader| {
                    let scope = loader.get("app").ok_or_else(|| {
                        vec![Diagnostic::error(
                            "cached prelude compilation did not produce app module",
                        )]
                    })?;
                    zun_module::Module::read(&scope.module().write())
                        .map_err(|error| vec![Diagnostic::error(error.to_string())])
                })
            }
            None => zun_compiler::compile_with_default_std(&source),
        }
    };
    let module = match module_result {
        Ok(m) => m,
        Err(diags) => {
            report(file, &source, &diags);
            return ExitCode::FAILURE;
        }
    };

    if let Action::Build { aot, .. } = &action {
        if module.requires_codegen {
            if *aot {
                let target = match aot_target() {
                    Ok(target) => target,
                    Err(error) => {
                        eprintln!("zun: {error}");
                        return ExitCode::FAILURE;
                    }
                };
                if let Some(message) = ios_codegen_error(&target, file, &source, &module) {
                    eprintln!("{message}");
                    return ExitCode::FAILURE;
                }
            }
            eprintln!("zun: note: this artifact requires the `codegen` capability (runtime code generation)");
        }
    }

    let preserve_roots = match &action {
        Action::Build {
            preserve_roots: Some(path),
            ..
        } => match read_preserve_roots(path) {
            Ok(roots) => roots,
            Err(error) => {
                eprintln!(
                    "zun: cannot read preserve roots {}: {error}",
                    path.display()
                );
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
                warn_dynamic_fallback(&report);
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
        Action::Build { aot: true, .. } => {
            if let Ok(report) = zun_compiler::trim_report_with_roots(&module, &preserve_roots) {
                if report.dynamic_fallback {
                    warn_dynamic_fallback(&report);
                }
            }
            module
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
        Action::Build {
            aot, prelude_cache, ..
        } => {
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
                                let target = match aot_target() {
                                    Ok(target) => target,
                                    Err(error) => {
                                        eprintln!("zun: {error}");
                                        return ExitCode::FAILURE;
                                    }
                                };
                                if target != target_lexicon::Triple::host()
                                    && resolve_runtime_library(&target).is_none()
                                {
                                    eprintln!(
                                        "zun: no runtime library provisioned for cross target `{target}`; set ZUN_RUNTIME_LIB_{}` or package a target-qualified runtime library",
                                        runtime_library_env_name(&target.to_string())
                                            .trim_start_matches("ZUN_RUNTIME_LIB_")
                                    );
                                    return ExitCode::FAILURE;
                                }
                                let object_out = out.with_extension("o");
                                match zun_codegen::compile_object_for_target(
                                    &module,
                                    target.clone(),
                                ) {
                                    Ok(object) => match std::fs::write(&object_out, object) {
                                        Ok(()) => {
                                            println!("wrote {}", object_out.display());
                                            let metadata_out = out.with_extension("meta.o");
                                            let metadata = match zun_codegen::compile_metadata_object_for_target(&module.write(), target.clone()) {
                                                Ok(metadata) => metadata,
                                                Err(error) => {
                                                    eprintln!("zun: cannot emit AOT metadata object: {error}");
                                                    return ExitCode::FAILURE;
                                                }
                                            };
                                            if let Err(error) =
                                                std::fs::write(&metadata_out, metadata)
                                            {
                                                eprintln!(
                                                    "zun: cannot write {}: {error}",
                                                    metadata_out.display()
                                                );
                                                return ExitCode::FAILURE;
                                            }
                                            println!("wrote {}", metadata_out.display());
                                            let native_manifest = manifest
                                                .clone()
                                                .with_native_object(
                                                    object_out
                                                        .file_name()
                                                        .expect("object output has a file name")
                                                        .to_string_lossy()
                                                        .into_owned(),
                                                )
                                                .with_metadata_object(
                                                    metadata_out
                                                        .file_name()
                                                        .expect("metadata output has a file name")
                                                        .to_string_lossy()
                                                        .into_owned(),
                                                );
                                            let native_manifest =
                                                match resolve_runtime_library(&target) {
                                                    Some(runtime) => native_manifest
                                                        .with_runtime_library(
                                                            runtime_manifest_path(&runtime),
                                                        ),
                                                    None => native_manifest,
                                                };
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
                                                resolve_runtime_library(&target).as_deref(),
                                                &executable_out,
                                                target.clone(),
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
                                                    if e == "AOT launcher supports only scalar numeric returns" {
                                                        if let Some(runtime) = resolve_runtime_library(&target) {
                                                            let fallback_manifest = manifest
                                                                .clone()
                                                                .without_native_object()
                                                                .with_metadata_object(
                                                                    metadata_out
                                                                        .file_name()
                                                                        .expect("metadata output has a file name")
                                                                        .to_string_lossy()
                                                                        .into_owned(),
                                                                )
                                                                .with_runtime_library(runtime_manifest_path(&runtime));
                                                            if let Err(error) = fallback_manifest.write_file(&manifest_out) {
                                                                eprintln!("zun: cannot update fallback manifest: {error}");
                                                                return ExitCode::FAILURE;
                                                            }
                                                            let return_kind = module.entry_point.and_then(|token| {
                                                                module.tables.method_defs.get(token.rid() as usize - 1)
                                                                    .and_then(|method| zun_module::read_body(&module.code, method.body_offset).ok())
                                                                    .map(|body| body.return_kind)
                                                            });
                                                            match link_aot_fallback(
                                                                &metadata_out,
                                                                &runtime,
                                                                &executable_out,
                                                                target.clone(),
                                                                return_kind,
                                                            ) {
                                                                Ok(()) => {
                                                                    println!("wrote {}", executable_out.display());
                                                                    ExitCode::SUCCESS
                                                                }
                                                                Err(error) => {
                                                                    eprintln!("zun: cannot link interpreter fallback: {error}");
                                                                    ExitCode::FAILURE
                                                                }
                                                            }
                                                        } else {
                                                            let fallback_manifest = manifest.clone().without_native_object();
                                                            if let Err(error) = fallback_manifest.write_file(&manifest_out) {
                                                                eprintln!("zun: warning: cannot update fallback manifest: {error}");
                                                            }
                                                            eprintln!("zun: warning: {e}; retaining embedded IL fallback");
                                                            ExitCode::SUCCESS
                                                        }
                                                    } else {
                                                        eprintln!("zun: cannot link AOT executable: {e}");
                                                        ExitCode::FAILURE
                                                    }
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
                                    Err(zun_codegen::JitError::Ineligible(rid)) => {
                                        eprintln!(
                                            "zun: warning: method #{rid} is not AOT-eligible; keeping embedded IL interpreter fallback"
                                        );
                                        let has_entrypoint = module.entry_point.is_some();
                                        if has_entrypoint {
                                            if let Some(runtime) = resolve_runtime_library(&target)
                                            {
                                                let metadata_out = out.with_extension("meta.o");
                                                match zun_codegen::compile_metadata_object_for_target(&module.write(), target.clone()) {
                                                Ok(metadata) => {
                                                    if let Err(error) = std::fs::write(&metadata_out, metadata) {
                                                        eprintln!("zun: warning: cannot write fallback metadata object: {error}");
                                                    } else {
                                                        let fallback_manifest = manifest.clone()
                                                            .with_metadata_object(
                                                                metadata_out
                                                                    .file_name()
                                                                    .expect("metadata output has a file name")
                                                                    .to_string_lossy()
                                                                    .into_owned(),
                                                            )
                                                            .with_runtime_library(runtime_manifest_path(&runtime));
                                                        if let Err(error) = fallback_manifest.write_file(&manifest_out) {
                                                            eprintln!("zun: warning: cannot update fallback manifest: {error}");
                                                        }
                                                        let executable_out = out.with_extension("exe");
                                                        let return_kind = module.entry_point.and_then(|token| {
                                                            module.tables.method_defs.get(token.rid() as usize - 1)
                                                                .and_then(|method| zun_module::read_body(&module.code, method.body_offset).ok())
                                                                .map(|body| body.return_kind)
                                                        });
                                                        match link_aot_fallback(
                                                            &metadata_out,
                                                            &runtime,
                                                            &executable_out,
                                                            target.clone(),
                                                            return_kind,
                                                        ) {
                                                            Ok(()) => println!("wrote {}", executable_out.display()),
                                                            Err(error) => eprintln!("zun: warning: cannot link interpreter fallback: {error}"),
                                                        }
                                                    }
                                                }
                                                Err(error) => eprintln!("zun: warning: cannot emit fallback metadata object: {error}"),
                                            }
                                            }
                                        }
                                        ExitCode::SUCCESS
                                    }
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
            value
                .trim()
                .parse::<u32>()
                .map_err(|error| format!("invalid method RID on line {}: {error}", line + 1))
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
fn resolve_lld(target: target_lexicon::Triple) -> Option<PathBuf> {
    let configured = if target == target_lexicon::Triple::host() {
        std::env::var_os("ZUN_LLD")
            .or_else(|| std::env::var_os(target_env_name("ZUN_LLD", &target.to_string())))
    } else {
        std::env::var_os(target_env_name("ZUN_LLD", &target.to_string()))
    };
    if let Some(path) = explicit_lld_override(configured.as_deref()) {
        return Some(path);
    }
    let path_candidates = if cfg!(windows) {
        ["lld-link.exe", "ld.lld.exe", "lld.exe"]
    } else {
        ["ld.lld", "lld", "lld-link"]
    };
    if let Some(path) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|directory| {
            path_candidates
                .iter()
                .map(|name| directory.join(name))
                .find(|candidate| candidate.is_file())
        })
    }) {
        return Some(path);
    }
    let target_libdir = Command::new("rustc")
        .args(["--print", "target-libdir", "--target"])
        .arg(target.to_string())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_owned()))?;
    let target_bin = target_libdir.parent()?.join("bin");
    let mut candidate = target_bin.join("rust-lld");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    if candidate.is_file() {
        return Some(candidate);
    }
    let mut bundled = target_bin.join("gcc-ld").join(if cfg!(windows) {
        "lld-link.exe"
    } else {
        "ld.lld"
    });
    bundled = bundled.canonicalize().ok()?;
    bundled.is_file().then_some(bundled)
}

fn explicit_lld_override(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// Resolve an explicitly provisioned static runtime library for AOT links.
/// The build remains usable without it while the embedded-IL launcher path is
/// used; packaged distributions set this to the matching `zun_vm.lib`.
fn resolve_runtime_library(target: &target_lexicon::Triple) -> Option<PathBuf> {
    let target_name = target.to_string();
    let target_env = runtime_library_env_name(&target_name);
    let configured = if *target == target_lexicon::Triple::host() {
        std::env::var("ZUN_RUNTIME_LIB").or_else(|_| std::env::var(&target_env))
    } else {
        std::env::var(&target_env)
    };
    if let Ok(path) = configured {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let directory = std::env::current_exe().ok()?.parent()?.to_owned();
    let target_specific = [
        format!("zun_vm-{target_name}.lib"),
        format!("libzun_vm-{target_name}.a"),
    ];
    target_specific
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
        .or_else(|| {
            if *target == target_lexicon::Triple::host() {
                ["zun_vm.lib".to_owned(), "libzun_vm.a".to_owned()]
                    .iter()
                    .map(|name| directory.join(name))
                    .find(|path| path.is_file())
            } else {
                None
            }
        })
}

fn runtime_library_env_name(target: &str) -> String {
    target_env_name("ZUN_RUNTIME_LIB", target)
}

fn target_env_name(prefix: &str, target: &str) -> String {
    format!(
        "{prefix}_{}",
        target
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

fn runtime_manifest_path(path: &Path) -> String {
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        if path.parent() == Some(parent.as_path()) {
            if let Some(name) = path.file_name() {
                return name.to_string_lossy().into_owned();
            }
        }
    }
    path.to_string_lossy().into_owned()
}

fn link_aot_executable(
    object: &Path,
    metadata: Option<&Path>,
    runtime: Option<&Path>,
    executable: &Path,
    target: target_lexicon::Triple,
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
        | zun_il::JitKind::Function
        | zun_il::JitKind::Other => {
            return Err("AOT launcher supports only scalar numeric returns".to_string())
        }
    };
    let launcher = object.with_extension("aot.rs");
    let runtime_declarations = if runtime.is_some() {
        "extern \"C\" { fn zun_rt_init(); fn zun_rt_shutdown(); }\n"
    } else {
        ""
    };
    let runtime_start = if runtime.is_some() {
        "unsafe { zun_rt_init(); }"
    } else {
        ""
    };
    let runtime_stop = if runtime.is_some() {
        "unsafe { zun_rt_shutdown(); }"
    } else {
        ""
    };
    let source = format!(
        "{runtime_declarations}extern \"C\" {{ fn m{entry}() -> {return_type}; static zun_embedded_module: u8; static zun_embedded_module_len: u64; }}\nfn main() {{ {runtime_start} let _metadata = unsafe {{ std::hint::black_box((&zun_embedded_module, &zun_embedded_module_len)) }}; let value = unsafe {{ m{entry}() }}; println!(\"{{}}\", {print_value}); {runtime_stop} }}\n"
    );
    std::fs::write(&launcher, source).map_err(|error| error.to_string())?;
    let mut command = Command::new("rustc");
    command
        .arg(&launcher)
        .arg("--target")
        .arg(target.to_string())
        .arg("--crate-name")
        .arg("zun_aot")
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()))
        .arg("-o")
        .arg(executable);
    if let Some(metadata) = metadata {
        command
            .arg("-C")
            .arg(format!("link-arg={}", metadata.display()));
    }
    if let Some(runtime) = runtime {
        command
            .arg("-C")
            .arg(format!("link-arg={}", runtime.display()));
        if cfg!(windows) {
            command
                .arg("-C")
                .arg("link-arg=bcrypt.lib")
                .arg("-C")
                .arg("link-arg=advapi32.lib");
        }
    }
    if let Some(lld) = resolve_lld(target) {
        command.arg("-C").arg(format!("linker={}", lld.display()));
    }
    let result = command.output().map_err(|error| error.to_string());
    let _ = std::fs::remove_file(&launcher);
    let output = result?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// Link an interpreter-backed AOT launcher for a method that cannot be lowered
/// to native code. The embedded metadata object supplies the original `.zm`
/// bytes and the static runtime supplies the Tier 0 entrypoint.
fn link_aot_fallback(
    metadata: &Path,
    runtime: &Path,
    executable: &Path,
    target: target_lexicon::Triple,
    return_kind: Option<u8>,
) -> Result<(), String> {
    let launcher = executable.with_extension("aot.rs");
    let unsigned = matches!(
        return_kind.map(zun_il::JitKind::from_byte),
        Some(zun_il::JitKind::Num(
            zun_il::NumKind::U8
                | zun_il::NumKind::U16
                | zun_il::NumKind::U32
                | zun_il::NumKind::U64
        ))
    );
    let boolean = matches!(
        return_kind.map(zun_il::JitKind::from_byte),
        Some(zun_il::JitKind::Bool)
    );
    let floating = matches!(
        return_kind.map(zun_il::JitKind::from_byte),
        Some(zun_il::JitKind::Num(
            zun_il::NumKind::F32 | zun_il::NumKind::F64
        ))
    );
    let source = if unsigned {
        "extern \"C\" { fn zun_rt_init(); fn zun_rt_shutdown(); fn zun_rt_run_embedded_u64(ptr: *const u8, len: u64) -> u64; static zun_embedded_module: u8; static zun_embedded_module_len: u64; }\nfn main() { unsafe { zun_rt_init(); let value = zun_rt_run_embedded_u64(&zun_embedded_module, zun_embedded_module_len); zun_rt_shutdown(); if value == u64::MAX { std::process::exit(1); } println!(\"{}\", value); } }\n"
    } else if boolean {
        "extern \"C\" { fn zun_rt_init(); fn zun_rt_shutdown(); fn zun_rt_run_embedded_i64(ptr: *const u8, len: u64) -> i64; static zun_embedded_module: u8; static zun_embedded_module_len: u64; }\nfn main() { unsafe { zun_rt_init(); let value = zun_rt_run_embedded_i64(&zun_embedded_module, zun_embedded_module_len); zun_rt_shutdown(); if value < 0 { std::process::exit(1); } println!(\"{}\", value != 0); } }\n"
    } else if floating {
        "extern \"C\" { fn zun_rt_init(); fn zun_rt_shutdown(); fn zun_rt_run_embedded_f64(ptr: *const u8, len: u64, out: *mut f64) -> i32; static zun_embedded_module: u8; static zun_embedded_module_len: u64; }\nfn main() { unsafe { zun_rt_init(); let mut value = 0.0; let status = zun_rt_run_embedded_f64(&zun_embedded_module, zun_embedded_module_len, &mut value); zun_rt_shutdown(); if status != 0 { std::process::exit(1); } println!(\"{}\", value); } }\n"
    } else {
        "extern \"C\" { fn zun_rt_init(); fn zun_rt_shutdown(); fn zun_rt_run_embedded_print(ptr: *const u8, len: u64) -> i32; static zun_embedded_module: u8; static zun_embedded_module_len: u64; }\nfn main() { unsafe { zun_rt_init(); let status = zun_rt_run_embedded_print(&zun_embedded_module, zun_embedded_module_len); zun_rt_shutdown(); if status != 0 { std::process::exit(1); } } }\n"
    };
    std::fs::write(&launcher, source).map_err(|error| error.to_string())?;
    let mut command = Command::new("rustc");
    command
        .arg(&launcher)
        .arg("--target")
        .arg(target.to_string())
        .arg("--crate-name")
        .arg("zun_aot_fallback")
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg(format!("link-arg={}", metadata.display()))
        .arg("-C")
        .arg(format!("link-arg={}", runtime.display()))
        .arg("-o")
        .arg(executable);
    if cfg!(windows) {
        command
            .arg("-C")
            .arg("link-arg=bcrypt.lib")
            .arg("-C")
            .arg("link-arg=advapi32.lib");
    }
    if let Some(lld) = resolve_lld(target) {
        command.arg("-C").arg(format!("linker={}", lld.display()));
    }
    let result = command.output().map_err(|error| error.to_string());
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

/// True for Apple-mobile targets (iOS, tvOS, watchOS, visionOS), where the
/// platform forbids runtime code generation.
fn is_apple_mobile(target: &target_lexicon::Triple) -> bool {
    let text = target.to_string();
    ["-ios", "-tvos", "-watchos", "-visionos"]
        .iter()
        .any(|marker| text.contains(marker))
}

/// Build error for a module that generates code at runtime on an iOS target.
fn ios_codegen_error(
    target: &target_lexicon::Triple,
    file: &str,
    source: &str,
    module: &Module,
) -> Option<String> {
    if !module.requires_codegen || !is_apple_mobile(target) {
        return None;
    }
    let sites = module
        .codegen_sites
        .iter()
        .map(|(start, _)| {
            let (line, col) = line_col(source, *start);
            format!("{file}:{line}:{col}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "zun: error: runtime code generation (`new Function`/`eval`) is not available on iOS targets (`{target}`) — used at {sites}"
    ))
}

fn warn_dynamic_fallback(report: &zun_compiler::TrimReport) {
    eprintln!(
        "zun: warning: dynamic/reflection operations require embedded-IL interpreter fallback"
    );
    for site in &report.dynamic_sites {
        match site.source_span {
            Some((start, end)) => eprintln!(
                "zun: warning: dynamic/reflection fallback site: method `{}` at PC {} (source {}..{})",
                site.method_name, site.pc, start, end
            ),
            None => eprintln!(
                "zun: warning: dynamic/reflection fallback site: method `{}` at PC {}",
                site.method_name, site.pc
            ),
        }
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

#[cfg(test)]
mod tests {
    use super::{
        explicit_lld_override, parse_aot_target, runtime_library_env_name, target_env_name,
    };
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn explicit_lld_override_keeps_stale_paths_authoritative() {
        assert_eq!(
            explicit_lld_override(Some(OsStr::new("missing-lld"))).as_deref(),
            Some(Path::new("missing-lld"))
        );
    }

    #[test]
    fn empty_lld_override_allows_discovery() {
        assert!(explicit_lld_override(Some(OsStr::new(""))).is_none());
        assert!(explicit_lld_override(None).is_none());
    }

    #[test]
    fn absent_aot_target_defaults_to_host() {
        assert_eq!(
            parse_aot_target(None).unwrap(),
            target_lexicon::Triple::host()
        );
    }

    #[test]
    fn invalid_aot_target_is_rejected_before_emission() {
        let error = parse_aot_target(Some("not-a-target".to_owned())).unwrap_err();
        assert!(error.contains("invalid ZUN_TARGET"));
    }

    #[test]
    fn target_runtime_override_uses_a_shell_safe_environment_name() {
        assert_eq!(
            runtime_library_env_name("aarch64-unknown-linux-gnu"),
            "ZUN_RUNTIME_LIB_AARCH64_UNKNOWN_LINUX_GNU"
        );
    }

    #[test]
    fn target_lld_override_uses_the_same_target_qualification() {
        assert_eq!(
            target_env_name("ZUN_LLD", "aarch64-unknown-linux-gnu"),
            "ZUN_LLD_AARCH64_UNKNOWN_LINUX_GNU"
        );
    }
}

#[cfg(test)]
mod codegen_gate_tests {
    use super::ios_codegen_error;

    const EVAL: &str = "function main() {\n  let f = new Function(\"a\", \"return a;\");\n  print((f(1.0)) as number);\n}\n";

    fn compile(src: &str) -> zun_module::Module {
        zun_compiler::compile(src).expect("compiles")
    }

    #[test]
    fn eval_program_requires_codegen_and_roundtrips() {
        let module = compile(EVAL);
        assert!(module.requires_codegen);
        assert_eq!(module.capabilities(), vec!["codegen"]);
        let read = zun_module::Module::read(&module.write()).unwrap();
        assert!(read.requires_codegen);
        assert_eq!(read.codegen_sites, module.codegen_sites);
    }

    #[test]
    fn plain_program_has_no_codegen() {
        let module = compile("function main(): number { return 1; }
");
        assert!(!module.requires_codegen);
        assert!(module.capabilities().is_empty());
    }

    #[test]
    fn ios_gate_errors_only_on_ios() {
        let module = compile(EVAL);
        let ios: target_lexicon::Triple = "aarch64-apple-ios".parse().unwrap();
        let message = ios_codegen_error(&ios, "x.ts", EVAL, &module).expect("ios errors");
        assert!(message.contains("not available on iOS"));
        assert!(message.contains("x.ts:2:"));
        let host = target_lexicon::Triple::host();
        assert!(ios_codegen_error(&host, "x.ts", EVAL, &module).is_none());
        let plain = compile("function main(): number { return 1; }
");
        assert!(ios_codegen_error(&ios, "x.ts", "", &plain).is_none());
    }
}

/// Which engine flavour executes a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeKind {
    /// Compiler-free runtime: `new Function` is disallowed.
    CompilerFree,
    /// Eval-capable runtime: the compiler-backed `EvalHost` is injected.
    EvalCapable,
}

/// Capability-driven selection: only a module declaring `codegen` needs the
/// eval-capable engine.
fn select_runtime(module: &Module) -> RuntimeKind {
    if module.capabilities().contains(&"codegen") {
        RuntimeKind::EvalCapable
    } else {
        RuntimeKind::CompilerFree
    }
}

/// `zun run <file.zm>`: load a compiled module and run it on the engine its
/// capabilities call for.
fn run_zm(file: &str) -> ExitCode {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("zun: cannot read {file}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let module = match Module::read(&bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("zun: cannot load {file}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match select_runtime(&module) {
        // Provisioning seam: once the compiler ships as a separate on-demand
        // component, this arm fetches it into `~/.zun/modules/` and loads
        // `zun_runtime` (eval flavour) via `zun_rt_run_eval` instead of linking
        // it in-process.
        RuntimeKind::EvalCapable => zun_vm::execute(&bytes),
        RuntimeKind::CompilerFree => zun_vm::execute_aot(&bytes),
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zun: {e}");
            ExitCode::FAILURE
        }
    }
}
