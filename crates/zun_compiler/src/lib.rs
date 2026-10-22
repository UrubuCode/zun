//! Compiler driver for the source-to-ZIL pipeline.
//!
//! The compiler owns the front-end boundary: source text enters here and a
//! self-contained [`zun_module::Module`] leaves here. Runtime crates consume
//! only that module format and never depend on the compiler or its AST.

use std::collections::{HashMap, HashSet};
use zun_common::{Diagnostic, Severity};
use zun_il::{decode_all, Opcode, Operand};
use zun_module::{read_body, Module};

/// Compute the conservative set of method RIDs reachable from a module entry.
///
/// This is the analysis boundary for future trimming/AOT. Direct calls follow
/// their method token; virtual calls retain every method with the same name,
/// because an override may be selected by a runtime receiver type. MemberRef
/// calls remain conservative and retain the complete local method set until
/// cross-module resolution supplies a narrower graph.
pub fn reachable_methods(module: &Module) -> Result<HashSet<u32>, String> {
    reachable_methods_with_roots(module, &[])
}

/// Compute reachability from the entry point plus explicitly preserved method
/// RIDs supplied by a dynamic-import/reflection root manifest.
pub fn reachable_methods_with_roots(
    module: &Module,
    roots: &[u32],
) -> Result<HashSet<u32>, String> {
    let Some(entry) = module.entry_point else {
        if roots.is_empty() {
            return Ok(HashSet::new());
        }
        let mut worklist = roots.to_vec();
        let mut reachable = HashSet::new();
        return walk_reachable_methods(module, &mut worklist, &mut reachable);
    };
    let mut reachable = HashSet::new();
    let mut worklist = Vec::with_capacity(roots.len() + 1);
    worklist.push(entry.rid());
    worklist.extend_from_slice(roots);
    walk_reachable_methods(module, &mut worklist, &mut reachable)
}

fn walk_reachable_methods(
    module: &Module,
    worklist: &mut Vec<u32>,
    reachable: &mut HashSet<u32>,
) -> Result<HashSet<u32>, String> {
    while let Some(rid) = worklist.pop() {
        if !reachable.insert(rid) {
            continue;
        }
        let method = module
            .tables
            .method_defs
            .get(rid as usize - 1)
            .ok_or_else(|| format!("method RID {rid} is out of range"))?;
        if method.body_offset == u32::MAX {
            continue;
        }
        let body = read_body(&module.code, method.body_offset).map_err(|e| e.to_string())?;
        for instruction in decode_all(body.code).map_err(|e| e.to_string())? {
            let Operand::Token(token) = instruction.operand else {
                continue;
            };
            match instruction.opcode {
                Opcode::Call if token.table() == Some(zun_metadata::TableKind::MethodDef) => {
                    worklist.push(token.rid());
                }
                Opcode::CallVirt if token.table() == Some(zun_metadata::TableKind::MethodDef) => {
                    let name = module.tables.method_defs[token.rid() as usize - 1].name;
                    for (index, candidate) in module.tables.method_defs.iter().enumerate() {
                        if candidate.name == name {
                            worklist.push(index as u32 + 1);
                        }
                    }
                }
                Opcode::Call if token.table() == Some(zun_metadata::TableKind::MemberRef) => {
                    // Cross-module resolution is performed by the loader; keep
                    // local methods conservative until that graph is available.
                    reachable.extend(1..=module.tables.method_defs.len() as u32);
                }
                _ => {}
            }
        }
    }
    Ok(reachable.clone())
}

/// Deterministic reachability information produced by the trimmer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimReport {
    /// Method definition RIDs whose bodies are retained.
    pub reachable_methods: Vec<u32>,
    /// Method definition RIDs whose bodies are removed.
    pub removed_methods: Vec<u32>,
}

/// Analyze a module without changing it, returning the trimmer's decisions.
pub fn trim_report(module: &Module) -> Result<TrimReport, String> {
    trim_report_with_roots(module, &[])
}

/// Analyze reachability while preserving explicit dynamic roots.
pub fn trim_report_with_roots(module: &Module, roots: &[u32]) -> Result<TrimReport, String> {
    let reachable = reachable_methods_with_roots(module, roots)?;
    let mut reachable_methods: Vec<_> = reachable.iter().copied().collect();
    reachable_methods.sort_unstable();
    let removed_methods: Vec<_> = (1..=module.tables.method_defs.len() as u32)
        .filter(|rid| !reachable.contains(rid))
        .collect();
    Ok(TrimReport {
        reachable_methods,
        removed_methods,
    })
}

/// Remove unreachable method bodies while preserving metadata RIDs.
///
/// Keeping definition rows stable means existing tokens and cross-module
/// references remain valid. An unreachable row becomes an abstract/no-body
/// row, while reachable bodies are copied into a compact code section.
pub fn trim_module(module: &Module) -> Result<Module, String> {
    trim_module_with_roots(module, &[])
}

/// Remove unreachable bodies while retaining methods named by a root manifest.
pub fn trim_module_with_roots(module: &Module, roots: &[u32]) -> Result<Module, String> {
    let reachable: HashSet<u32> = trim_report_with_roots(module, roots)?.reachable_methods.into_iter().collect();
    let mut tables = module.tables.clone();
    let mut code = Vec::new();
    for (index, method) in module.tables.method_defs.iter().enumerate() {
        let rid = index as u32 + 1;
        let Some(body_offset) = reachable.contains(&rid).then_some(method.body_offset) else {
            tables.method_defs[index].body_offset = u32::MAX;
            continue;
        };
        let body = read_body(&module.code, body_offset).map_err(|e| e.to_string())?;
        let body_code = body.code.to_vec();
        let local_kinds = body.local_kinds.to_vec();
        let new_offset = zun_module::write_body(
            &mut code,
            body.param_count,
            body.local_count,
            body.returns_value,
            body.return_kind,
            &local_kinds,
            &body_code,
        );
        tables.method_defs[index].body_offset = new_offset;
    }
    Ok(Module {
        entry_point: module.entry_point,
        code,
        strings: module.strings.clone(),
        blobs: module.blobs.clone(),
        tables,
        sequence_points: module.sequence_points.clone(),
    })
}

/// Compile source text with the standard prelude into a `.zm` module.
pub fn compile(source: &str) -> Result<Module, Vec<Diagnostic>> {
    compile_with_externals(source, &[])
}

/// Compile source text with linker-provided source-module signatures.
pub fn compile_with_externals(
    source: &str,
    external_functions: &[zun_sema::check::ExternalFunction],
) -> Result<Module, Vec<Diagnostic>> {
    let source_with_prelude = zun_prelude::with_prelude(source);
    compile_source_with_externals(&source_with_prelude, external_functions)
}

/// Compile source without implicitly concatenating the standard prelude.
///
/// This is the stable compiler boundary for independently emitted modules:
/// callers must provide any imported prelude signatures explicitly. Keeping
/// this path separate from [`compile_with_externals`] prevents a future
/// precompiled-prelude cache from changing source offsets or metadata RIDs in
/// the ordinary source compiler.
pub fn compile_without_prelude(
    source: &str,
    external_functions: &[zun_sema::check::ExternalFunction],
) -> Result<Module, Vec<Diagnostic>> {
    compile_source_with_externals(source, external_functions)
}

/// Compile the standard prelude as an independent module artifact.
///
/// The fingerprint travels with the artifact so a future R2R/cache loader can
/// reject stale prelude code before it is linked with user modules. The normal
/// [`compile`] path intentionally remains concatenated until module metadata
/// linking can preserve the existing prelude bindings and RID contracts.
pub fn compile_prelude() -> Result<(u64, Module), Vec<Diagnostic>> {
    let module = compile_without_prelude(zun_prelude::PRELUDE, &[])?;
    Ok((zun_prelude::fingerprint(), module))
}

/// Compile a standalone source module against an already compiled prelude.
///
/// The prelude is registered as a real loader dependency and is not
/// concatenated into the user's source.  The checked-in prelude source is used
/// only for its stable public signatures; the supplied module remains the
/// executable artifact.
pub fn compile_with_cached_prelude(
    source: &str,
    prelude: &Module,
) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>> {
    compile_with_cached_module(source, prelude, "zun:prelude")
}

/// Compile a standalone source module against the default-installed standard
/// library artifact. The `zun:std` identity is the module-boundary form of the
/// legacy cached-prelude API; the prelude remains available above for callers
/// that still target the compatibility identity.
pub fn compile_with_cached_std(
    source: &str,
    standard: &Module,
) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>> {
    compile_with_cached_module(source, standard, "zun:std")
}

/// Compile against the built-in default `zun:std` artifact without
/// concatenating standard-library source into the user's module. Hosts that
/// persist the artifact should use [`compile_with_cached_std`] instead.
pub fn compile_with_default_std(source: &str) -> Result<Module, Vec<Diagnostic>> {
    let (_, standard) = compile_prelude()?;
    let loader = compile_with_cached_std(source, &standard)?;
    let scope = loader
        .get("app")
        .ok_or_else(|| vec![Diagnostic::error("default std compilation did not produce app module")])?;
    Module::read(&scope.module().write())
        .map_err(|error| vec![Diagnostic::error(error.to_string())])
}

fn compile_with_cached_module(
    source: &str,
    prelude: &Module,
    module_name: &str,
) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>> {
    let prelude_scope = zun_loader::ModuleScope::from_bytes(module_name, &prelude.write())
        .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    let mut externals = collect_function_signatures(zun_prelude::PRELUDE);
    for function in &mut externals {
        function.module = module_name.to_owned();
    }
    for function in &externals {
        if prelude_scope.export(&function.name).is_none() {
            return Err(vec![Diagnostic::error(format!(
                "cached prelude is missing export `{}`",
                function.name
            ))]);
        }
    }
    let imports = externals
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let linked_source = format!("{source}\nimport {{ {imports} }} from \"{module_name}\";\n");
    let module = compile_source_with_externals(&linked_source, &externals)?;
    let mut loader = zun_loader::ModuleLoader::new();
    let app_scope = zun_loader::ModuleScope::from_bytes("app", &module.write())
        .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    loader
        .insert(prelude_scope)
        .and_then(|_| loader.insert(app_scope))
        .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    Ok(loader)
}

/// Errors returned by the standalone prelude cache envelope.
#[derive(Debug)]
pub enum PreludeCacheError {
    /// The cache file could not be read or written.
    Io(std::io::Error),
    /// The cache envelope or embedded `.zm` module is invalid.
    Invalid(String),
    /// The cache was built from a different prelude source.
    Stale { expected: u64, found: u64 },
}

impl core::fmt::Display for PreludeCacheError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "prelude cache I/O failed: {error}"),
            Self::Invalid(detail) => write!(f, "invalid prelude cache: {detail}"),
            Self::Stale { expected, found } => write!(
                f,
                "stale prelude cache: expected {expected:#x}, found {found:#x}"
            ),
        }
    }
}

impl std::error::Error for PreludeCacheError {}

const PRELUDE_CACHE_MAGIC: [u8; 4] = *b"ZPRC";
const PRELUDE_CACHE_VERSION: u16 = 1;

/// Persist the compiled prelude as a versioned cache envelope.
pub fn write_prelude_cache(
    path: impl AsRef<std::path::Path>,
) -> Result<u64, PreludeCacheError> {
    let (fingerprint, module) = compile_prelude().map_err(|diagnostics| {
        PreludeCacheError::Invalid(
            diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.message)
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    let payload = module.write();
    let mut bytes = Vec::with_capacity(4 + 2 + 8 + 4 + payload.len());
    bytes.extend_from_slice(&PRELUDE_CACHE_MAGIC);
    bytes.extend_from_slice(&PRELUDE_CACHE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&fingerprint.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    std::fs::write(path, bytes).map_err(PreludeCacheError::Io)?;
    Ok(fingerprint)
}

/// Read a prelude cache and reject it unless it matches the current source.
pub fn read_prelude_cache(
    path: impl AsRef<std::path::Path>,
) -> Result<Module, PreludeCacheError> {
    let bytes = std::fs::read(path).map_err(PreludeCacheError::Io)?;
    if bytes.len() < 18 || bytes[..4] != PRELUDE_CACHE_MAGIC {
        return Err(PreludeCacheError::Invalid("bad cache header".into()));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version > PRELUDE_CACHE_VERSION {
        return Err(PreludeCacheError::Invalid(format!(
            "unsupported cache version {version}"
        )));
    }
    let found = u64::from_le_bytes(bytes[6..14].try_into().unwrap());
    let expected = zun_prelude::fingerprint();
    if found != expected {
        return Err(PreludeCacheError::Stale { expected, found });
    }
    let length = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    let payload = bytes
        .get(18..18 + length)
        .ok_or_else(|| PreludeCacheError::Invalid("truncated module payload".into()))?;
    if bytes.len() != 18 + length {
        return Err(PreludeCacheError::Invalid(
            "trailing bytes after module payload".into(),
        ));
    }
    zun_module::Module::read(payload)
        .map_err(|error| PreludeCacheError::Invalid(error.to_string()))
}

fn compile_source_with_externals(
    source: &str,
    external_functions: &[zun_sema::check::ExternalFunction],
) -> Result<Module, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let ast = zun_syntax::parse(source, &mut diagnostics);
    if diagnostics.iter().any(is_error) {
        return Err(diagnostics);
    }
    let hir = zun_sema::check_with_externals(source, &ast, &mut diagnostics, external_functions);
    if diagnostics.iter().any(is_error) {
        return Err(diagnostics);
    }
    Ok(zun_ilgen::lower(&hir))
}

/// Compile one named source into a loadable module scope.
pub fn compile_scope(
    name: impl Into<String>,
    source: &str,
) -> Result<zun_loader::ModuleScope, Vec<Diagnostic>> {
    let module = compile(source)?;
    zun_loader::ModuleScope::from_bytes(name, &module.write())
        .map_err(|error| vec![Diagnostic::error(error.to_string())])
}

/// Compile and register a set of named source modules.
pub fn compile_modules<'a, I>(sources: I) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let sources: Vec<_> = sources.into_iter().collect();
    validate_imports(sources.iter().copied())?;
    let mut loader = zun_loader::ModuleLoader::new();
    for (name, source) in sources {
        let scope = compile_scope(name, source)?;
        loader
            .insert(scope)
            .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    }
    Ok(loader)
}

/// Compile a source-module set with user imports lowered to `MemberRef`s.
pub fn compile_linked_modules<'a, I>(
    sources: I,
) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let sources: Vec<_> = sources.into_iter().collect();
    validate_imports(sources.iter().copied())?;

    let mut exports = HashMap::new();
    for (module, source) in &sources {
        for mut function in collect_function_signatures(source) {
            function.module = (*module).to_owned();
            exports.insert(((*module).to_owned(), function.name.clone()), function);
        }
    }

    let mut loader = zun_loader::ModuleLoader::new();
    for (module, source) in sources {
        let mut externals = Vec::new();
        let mut parse_diagnostics = Vec::new();
        let ast = zun_syntax::parse(source, &mut parse_diagnostics);
        for import in ast.imports {
            for binding in import.names {
                let imported = &source[binding.imported.span.start as usize..binding.imported.span.end as usize];
                if let Some(function) = exports.get(&(import.module.clone(), imported.to_owned())) {
                    externals.push(function.clone());
                }
            }
        }
        let module_bytes = compile_with_externals(source, &externals)?;
        let scope = zun_loader::ModuleScope::from_bytes(module, &module_bytes.write())
            .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
        loader
            .insert(scope)
            .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    }
    Ok(loader)
}

fn collect_function_signatures(source: &str) -> Vec<zun_sema::check::ExternalFunction> {
    let source = zun_prelude::with_prelude(source);
    let mut parse_diagnostics = Vec::new();
    let ast = zun_syntax::parse(&source, &mut parse_diagnostics);
    let mut check_diagnostics = Vec::new();
    let hir = zun_sema::check_with_externals(&source, &ast, &mut check_diagnostics, &[]);
    hir.functions
        .into_iter()
        .map(|function| zun_sema::check::ExternalFunction {
            module: String::new(),
            name: function.name,
            params: function.locals[..function.param_count as usize].to_vec(),
            return_ty: function.return_ty,
        })
        .collect()
}

/// Compile the checked-in Zun standard-library modules into a runtime loader.
///
/// The source remains ordinary Zun code; this convenience entry point only
/// supplies the stable module identities used by hosts and tests.
pub fn compile_standard_library() -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>> {
    compile_modules([
        ("zun:std:math", include_str!("../../../std/math.zun")),
        ("zun:std:string", include_str!("../../../std/string.zun")),
        ("zun:std:number", include_str!("../../../std/number.zun")),
        ("zun:std:bigint", include_str!("../../../std/bigint.zun")),
        ("zun:std:array", include_str!("../../../std/array.zun")),
        ("zun:std:map", include_str!("../../../std/map.zun")),
        ("zun:std:set", include_str!("../../../std/set.zun")),
        (
            "zun:std:weak-map",
            include_str!("../../../std/weak_map.zun"),
        ),
        (
            "zun:std:finalization-registry",
            include_str!("../../../std/finalization_registry.zun"),
        ),
        ("zun:std:regexp", include_str!("../../../std/regexp.zun")),
        ("zun:std:date", include_str!("../../../std/date.zun")),
        ("zun:std:error", include_str!("../../../std/error.zun")),
    ])
}

/// Compile named modules and produce the flattened manifest consumed by the
/// runtime loader. The input order is retained for deterministic dependency
/// order, with the entry module stored separately at the end of the manifest.
pub fn compile_modules_with_manifest<'a, I>(
    entry: &str,
    sources: I,
) -> Result<(zun_loader::ModuleLoader, zun_loader::ModuleManifest), Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let sources: Vec<_> = sources.into_iter().collect();
    let names: Vec<String> = sources.iter().map(|(name, _)| (*name).to_owned()).collect();
    let loader = compile_modules(sources.iter().copied())?;
    let dependencies = names.into_iter().filter(|name| name != entry);
    let manifest = zun_loader::ModuleManifest::new(entry, dependencies)
        .with_prelude_fingerprint(zun_prelude::fingerprint());
    manifest
        .validate(&loader)
        .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    Ok((loader, manifest))
}

/// Compile named modules, produce the flattened manifest, and persist it for
/// a host or packaging step.
pub fn compile_modules_with_manifest_file<'a, I>(
    entry: &str,
    sources: I,
    path: impl AsRef<std::path::Path>,
) -> Result<(zun_loader::ModuleLoader, zun_loader::ModuleManifest), Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let (loader, manifest) = compile_modules_with_manifest(entry, sources)?;
    manifest
        .write_file(path)
        .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    Ok((loader, manifest))
}

/// Validate imports against the complete source set before module emission.
///
/// This is the compiler-side half of module linking: native `zun:*` exports
/// are checked through the intrinsic registry, while user-module exports are
/// collected from all source files. The later HIR/linker pass will attach the
/// resolved member-reference tokens to imported calls.
pub fn validate_imports<'a, I>(sources: I) -> Result<(), Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut modules: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut parsed = Vec::new();
    let mut diagnostics = Vec::new();
    for (module, source) in sources {
        if modules.contains_key(module) {
            diagnostics.push(Diagnostic::error(format!(
                "duplicate source module `{module}`"
            )));
            continue;
        }
        let mut parse_diagnostics = Vec::new();
        let program = zun_syntax::parse(source, &mut parse_diagnostics);
        diagnostics.extend(parse_diagnostics);
        let exports = program
            .functions
            .iter()
            .map(|function| {
                source[function.name.span.start as usize..function.name.span.end as usize]
                    .to_owned()
            })
            .chain(program.classes.iter().map(|class| {
                source[class.name.span.start as usize..class.name.span.end as usize].to_owned()
            }))
            .collect();
        modules.insert(module, exports);
        parsed.push((module, source, program));
    }
    if diagnostics.iter().any(is_error) {
        return Err(diagnostics);
    }

    for (module, source, program) in parsed {
        for import in program.imports {
            for binding in import.names {
                let imported = &source[binding.imported.span.start as usize..binding.imported.span.end as usize];
                let found = if import.module.starts_with("zun:") {
                    zun_natives::resolve(&import.module, imported).is_some()
                } else {
                    modules
                        .get(import.module.as_str())
                        .is_some_and(|exports| exports.contains(imported))
                };
                if !found {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "module `{}` has no export named `{imported}`",
                            import.module
                        ))
                        .with_span(import.span),
                    );
                }
            }
        }
        let _ = module;
    }
    if diagnostics.iter().any(is_error) {
        Err(diagnostics)
    } else {
        Ok(())
    }
}

fn is_error(diagnostic: &Diagnostic) -> bool {
    diagnostic.severity == Severity::Error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_a_minimal_entry_point() {
        let module = compile("function main(): int { return 7; }").expect("compile");
        assert!(module.entry_point.is_some());
        assert!(!module.code.is_empty());
    }

    #[test]
    fn compiles_an_independent_module_without_the_implicit_prelude() {
        let module = compile_without_prelude("function main(): int { return 42; }", &[])
            .expect("standalone module should compile");
        assert!(module.entry_point.is_some());
        assert!(module
            .tables
            .method_defs
            .iter()
            .all(|method| module.strings.get(method.name) != "clamp"));
    }

    #[test]
    fn compiles_a_standalone_module_importing_a_linked_prelude_function() {
        let source = "function main(): int { return abs(-7); }\nimport { abs } from \"zun:prelude\";";
        let module = compile_without_prelude(
            source,
            &[zun_sema::check::ExternalFunction {
                module: "zun:prelude".into(),
                name: "abs".into(),
                params: vec![zun_sema::Ty::INT],
                return_ty: zun_sema::Ty::INT,
            }],
        )
        .expect("linked prelude import should compile");
        assert_eq!(module.tables.member_refs.len(), 1);
    }

    #[test]
    fn compiles_against_a_cached_prelude_module_without_concatenation() {
        let (_, prelude) = compile_prelude().expect("prelude should compile");
        let loader = compile_with_cached_prelude(
            "function main(): int { return abs(-7); }",
            &prelude,
        )
        .expect("cached prelude should link");
        let app = loader.get("app").expect("app scope");
        assert_eq!(app.module().tables.member_refs.len(), 1);
        assert!(loader.get("zun:prelude").is_some());
    }

    #[test]
    fn compiles_against_the_default_installed_std_module() {
        let (_, standard) = compile_prelude().expect("standard library should compile");
        let loader = compile_with_cached_std(
            "function main(): int { return abs(-7); }",
            &standard,
        )
        .expect("zun:std should link");
        assert!(loader.get("zun:std").is_some());
        assert_eq!(loader.get("app").unwrap().module().tables.member_refs.len(), 1);
    }

    #[test]
    fn default_std_entry_point_does_not_concatenate_prelude_source() {
        let module = compile_with_default_std("function main(): int { return abs(-7); }")
            .expect("default std should compile");
        assert_eq!(module.tables.member_refs.len(), 1);
        assert!(module.sequence_points.iter().all(|point| point.source_start < 50));
    }

    #[test]
    fn rejects_a_cached_prelude_with_missing_exports() {
        let error = compile_with_cached_prelude(
            "function main(): int { return abs(-7); }",
            &Module::new(),
        )
        .expect_err("an incomplete prelude must not link");
        assert!(error.iter().any(|diagnostic| diagnostic
            .message
            .contains("missing export `abs`")));
    }

    #[test]
    fn emits_the_prelude_as_a_fingerprinted_independent_artifact() {
        let (fingerprint, module) = compile_prelude().expect("prelude should compile alone");
        assert_eq!(fingerprint, zun_prelude::fingerprint());
        assert!(module
            .tables
            .method_defs
            .iter()
            .any(|method| module.strings.get(method.name) == "clamp"));
    }

    #[test]
    fn prelude_cache_roundtrips_and_rejects_stale_fingerprints() {
        let path = std::env::temp_dir().join(format!(
            "zun-prelude-cache-{}-{}.zprc",
            std::process::id(),
            zun_prelude::fingerprint()
        ));
        write_prelude_cache(&path).expect("write prelude cache");
        let module = read_prelude_cache(&path).expect("read prelude cache");
        assert!(module
            .tables
            .method_defs
            .iter()
            .any(|method| module.strings.get(method.name) == "clamp"));
        let mut stale = std::fs::read(&path).expect("read cache bytes");
        stale[6] ^= 1;
        std::fs::write(&path, stale).expect("write stale cache");
        assert!(matches!(
            read_prelude_cache(&path),
            Err(PreludeCacheError::Stale { .. })
        ));
        write_prelude_cache(&path).expect("restore valid cache");
        let mut trailing = std::fs::read(&path).expect("read cache bytes");
        trailing.push(0);
        std::fs::write(&path, trailing).expect("write trailing cache");
        assert!(matches!(
            read_prelude_cache(&path),
            Err(PreludeCacheError::Invalid(detail)) if detail.contains("trailing")
        ));
        std::fs::remove_file(path).expect("remove cache");
    }

    #[test]
    fn reachability_excludes_unused_methods() {
        let module = compile(
            "function used(): int { return 7; }\n\
             function unused(): int { return 9; }\n\
             function main(): int { return used(); }",
        )
        .expect("compile");
        let reachable = reachable_methods(&module).expect("analyze reachability");
        assert!(reachable.contains(&1));
        assert!(reachable.contains(&3));
        assert!(!reachable.contains(&2));
        let report = trim_report(&module).expect("trim report");
        assert!(report.reachable_methods.contains(&1));
        assert!(report.reachable_methods.contains(&3));
        assert!(report.removed_methods.contains(&2));
        assert!(!report.removed_methods.contains(&1));
        assert!(!report.removed_methods.contains(&3));
    }

    #[test]
    fn preserve_roots_keep_dynamic_methods_during_trimming() {
        let module = compile_without_prelude(
            "function dynamic(): int { return 9; }\n\
             function main(): int { return 7; }",
            &[],
        )
        .expect("compile");
        let report = trim_report_with_roots(&module, &[1]).expect("rooted trim report");
        assert!(report.reachable_methods.contains(&1));
        assert!(report.reachable_methods.contains(&2));
        assert!(report.removed_methods.is_empty());
        let trimmed = trim_module_with_roots(&module, &[1]).expect("rooted trim");
        assert_ne!(trimmed.tables.method_defs[0].body_offset, u32::MAX);
    }

    #[test]
    fn trimming_removes_unreachable_bodies_and_preserves_tokens() {
        let module = compile(
            "function used(): int { return 7; }\n\
             function unused(): int { return 9; }\n\
             function main(): int { return used(); }",
        )
        .expect("compile");
        let trimmed = trim_module(&module).expect("trim");
        assert!(trimmed.code.len() < module.code.len());
        assert_ne!(trimmed.tables.method_defs[0].body_offset, u32::MAX);
        assert_eq!(trimmed.tables.method_defs[1].body_offset, u32::MAX);
        assert_ne!(trimmed.tables.method_defs[2].body_offset, u32::MAX);
        assert_eq!(trimmed.entry_point, module.entry_point);
        for method in &trimmed.tables.method_defs {
            if method.body_offset == u32::MAX {
                continue;
            }
            let body = read_body(&trimmed.code, method.body_offset).expect("trimmed body");
            let info = zun_verify::MethodInfo {
                local_count: body.local_count,
                returns_value: body.returns_value,
            };
            zun_verify::verify(body.code, info, &|opcode, _| {
                matches!(opcode, zun_il::Opcode::Call | zun_il::Opcode::CallVirt).then_some((0, 1))
            })
            .expect("trimmed module remains verifiable");
        }
    }

    #[test]
    fn compiles_external_call_to_member_reference() {
        let module = compile_with_externals(
            "import { answer } from \"core\"; function main(): int { return answer(41); }",
            &[zun_sema::check::ExternalFunction {
                module: "core".into(),
                name: "answer".into(),
                params: vec![zun_sema::Ty::INT],
                return_ty: zun_sema::Ty::INT,
            }],
        )
        .expect("compile external call");
        assert_eq!(module.tables.member_refs.len(), 1);
        assert_eq!(module.tables.module_refs.len(), 1);
    }

    #[test]
    fn compiles_source_extern_function_to_typed_member_reference() {
        let module = compile_with_externals(
            "extern function answer(value: i64): i64 from \"core\"; function main(): i64 { return answer(41); }",
            &[zun_sema::check::ExternalFunction {
                module: "core".into(),
                name: "answer".into(),
                params: vec![zun_sema::Ty::Int(zun_sema::hir::IntKind::I64)],
                return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
            }],
        )
        .expect("compile source extern function");
        assert_eq!(module.tables.member_refs.len(), 2);
        let signature = module
            .tables
            .member_refs
            .iter()
            .map(|member| module.blobs.get(member.signature))
            .find(|signature| *signature == [1, 1, 1])
            .expect("typed external signature blob");
        assert_eq!(signature, &[1, 1, 1]);
    }

    #[test]
    fn rejects_source_extern_function_with_wrong_library() {
        let diagnostics = compile_with_externals(
            "extern function answer(): i64 from \"other\"; function main(): i64 { return answer(); }",
            &[zun_sema::check::ExternalFunction {
                module: "core".into(),
                name: "answer".into(),
                params: Vec::new(),
                return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
            }],
        )
        .expect_err("source extern module mismatch must be rejected");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "SEM0001"));
    }

    #[test]
    fn rejects_ambiguous_external_symbol_bindings() {
        let diagnostics = compile_with_externals(
            "import { answer } from \"core\"; function main(): i64 { return answer(); }",
            &[
                zun_sema::check::ExternalFunction {
                    module: "core".into(),
                    name: "answer".into(),
                    params: Vec::new(),
                    return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
                },
                zun_sema::check::ExternalFunction {
                    module: "other".into(),
                    name: "answer".into(),
                    params: Vec::new(),
                    return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
                },
            ],
        )
        .expect_err("ambiguous external symbols must be rejected");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "SEM0001"));
    }

    #[test]
    fn rejects_external_call_with_mismatched_signature() {
        let diagnostics = compile_with_externals(
            "import { answer } from \"core\"; function main(): int { return answer(\"wrong\"); }",
            &[zun_sema::check::ExternalFunction {
                module: "core".into(),
                name: "answer".into(),
                params: vec![zun_sema::Ty::INT],
                return_ty: zun_sema::Ty::INT,
            }],
        )
        .expect_err("mismatched external signatures must be rejected");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SEM0001"));
    }

    #[test]
    fn compiles_extern_method_to_builtin_call() {
        let module = compile(
            "import { receiverValue } from \"zun:math\";\n\
             class Service { extern receiverValue(value: number): number; }\n\
             function main(): number { return 0.0; }",
        )
        .expect("extern method should compile");
        let method = module
            .tables
            .method_defs
            .iter()
            .find(|method| module.strings.get(method.name) == "receiverValue")
            .expect("extern method metadata");
        let body = zun_module::read_body(&module.code, method.body_offset).expect("method body");
        let text = zun_il::disassemble(&body.code).expect("disassemble method body");
        assert!(text.contains("call.builtin"), "{text}");
    }

    #[test]
    fn returns_frontend_diagnostics() {
        let diagnostics = compile("function main( { }").expect_err("must reject malformed source");
        assert!(diagnostics.iter().any(|d| d.severity == Severity::Error));
    }

    #[test]
    fn compiles_named_modules_into_one_registry() {
        let loader = compile_modules([
            ("core", "function core(): int { return 1; }"),
            ("app", "function main(): int { return 0; }"),
        ])
        .expect("compile modules");
        assert_eq!(loader.len(), 2);
        assert!(loader.get("core").is_some());
        assert!(loader.get("app").is_some());
    }

    #[test]
    fn compiles_linked_user_module_call() {
        let loader = compile_linked_modules([
            (
                "core",
                "function answer(value: int): int { return value + 1; }",
            ),
            (
                "app",
                "import { answer } from \"core\"; function main(): int { return answer(41); }",
            ),
        ])
        .expect("linked compilation");
        let app = loader.get("app").expect("app");
        assert_eq!(app.module().tables.member_refs.len(), 1);
        assert_eq!(app.module().tables.module_refs.len(), 1);
    }

    #[test]
    fn validates_user_module_exports_before_linking() {
        validate_imports([
            ("core", "function answer(): int { return 42; }"),
            (
                "app",
                "import { answer } from \"core\";\nfunction main(): int { return 0; }",
            ),
        ])
        .expect("import should resolve");
        let error = validate_imports([
            ("core", "function answer(): int { return 42; }"),
            (
                "app",
                "import { missing } from \"core\";\nfunction main(): int { return 0; }",
            ),
        ])
        .expect_err("missing export should diagnose");
        assert!(error.iter().any(|d| d.message.contains("missing")));
    }

    #[test]
    fn standard_library_source_is_compilable_by_zun() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../std/math.zun"))
                .expect("std/math.zun");
        compile(&source).expect("standard library module should compile");
        let loader = compile_standard_library().expect("standard library should load");
        assert!(loader.get("zun:std:math").is_some());
        assert!(loader.get("zun:std:string").is_some());
        assert!(loader.get("zun:std:number").is_some());
        assert!(loader.get("zun:std:array").is_some());
        assert!(loader.get("zun:std:map").is_some());
        assert!(loader.get("zun:std:set").is_some());
        assert!(loader.get("zun:std:date").is_some());
        assert!(loader.get("zun:std:error").is_some());
    }

    #[test]
    fn build_produces_a_valid_flattened_manifest() {
        let (loader, manifest) = compile_modules_with_manifest(
            "app",
            [
                ("core", "function answer(): int { return 42; }"),
                ("app", "function main(): int { return 0; }"),
            ],
        )
        .expect("build");
        assert_eq!(
            manifest.load_order(&loader).expect("order"),
            vec!["core", "app"]
        );
        assert_eq!(
            zun_loader::ModuleManifest::read(&manifest.write()).unwrap(),
            manifest
        );
    }

    #[test]
    fn build_can_persist_manifest_for_distribution() {
        let path =
            std::env::temp_dir().join(format!("zun-build-manifest-{}.zman", std::process::id()));
        let (_loader, manifest) = compile_modules_with_manifest_file(
            "app",
            [
                ("core", "function answer(): int { return 42; }"),
                ("app", "function main(): int { return 0; }"),
            ],
            &path,
        )
        .expect("build");
        assert_eq!(
            zun_loader::ModuleManifest::read_file(&path).unwrap(),
            manifest
        );
        std::fs::remove_file(path).expect("remove manifest");
    }
}
