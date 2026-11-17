//! `new Function(...)` / `Function(...)`: runtime code generation from strings.
//!
//! The interpreter never links a compiler (PLAN section 2). It instead calls an
//! injected [`EvalHost`], which turns `(parameter names, body)` into an
//! ephemeral [`Module`]. The interpreter appends that module's methods to its
//! live module ([`Module::link`]: token and string rewrite), has the host
//! re-verify the appended methods, and returns a callable function value that
//! runs on Tier 0. With no host injected (AOT, `--jitless`-style builds) the
//! intrinsic throws the ECMAScript "code generation from strings disallowed"
//! error, exactly like V8 under a CSP.

use super::{Interp, RunError, Value};
use zun_module::Module;

/// A failure reported by an [`EvalHost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError(pub String);

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for EvalError {}

/// The message thrown when `new Function` runs without an [`EvalHost`].
pub const CODEGEN_DISALLOWED: &str = "EvalError: code generation from strings disallowed";

/// A source compiled by an [`EvalHost`]: a self-contained module (free of
/// types and external references, see [`Module::link`]) and the 1-based index
/// of its entry method. Nested closures may add further methods.
#[derive(Debug)]
pub struct Generated {
    pub module: Module,
    pub entry: u32,
    /// `false`: the entry *is* the requested function. `true`: the entry is a
    /// parameterless factory that returns it, which the interpreter calls
    /// once per construction (the function is then a real closure value).
    pub factory: bool,
}

/// Compiles source strings on behalf of the interpreter.
///
/// Implemented by a crate that links the compiler (the dev/JIT driver); the
/// interpreter itself only sees this trait.
pub trait EvalHost {
    /// Compile `function(params...) { body }`. The function takes the
    /// parameters in order.
    fn compile_function(&self, params: &[String], body: &str) -> Result<Generated, EvalError>;

    /// Compile `source` as global code (an indirect `eval`) into a
    /// self-contained module holding one parameterless function that returns
    /// the source's completion value, with the same contract as
    /// [`EvalHost::compile_function`]. A source whose completion value the
    /// host cannot produce faithfully must be an error, never a guess.
    fn compile_eval(&self, source: &str) -> Result<Generated, EvalError>;

    /// Verify the methods `first_rid..first_rid + count` of the freshly linked
    /// `module` before they can run.
    fn verify_linked(&self, module: &Module, first_rid: u32, count: u32) -> Result<(), EvalError>;
}

impl Interp<'_> {
    /// `functionConstruct(argsArray)`: all elements are strings; the last is the
    /// body, the preceding ones are parameter names (each may itself be a
    /// comma-separated list, as in ECMAScript).
    pub(super) fn function_construct(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let array = match args.first() {
            Some(Value::Object(array)) => *array,
            _ => return Err(RunError::BadToken),
        };
        let values = self.read_array_values(array)?;
        self.function_from_strings(values).map(Value::Function)
    }

    /// The function `Function(p1, ..., body)` denotes, for arguments that are
    /// already values: shared by the compiled `functionConstruct` site and by a
    /// call or construction of the `Function` constructor reached as a value.
    pub(crate) fn function_from_strings(
        &mut self,
        values: Vec<Value>,
    ) -> Result<zun_core::FunctionHandle, RunError> {
        if self.eval_host.is_none() {
            return Err(codegen_disallowed());
        }
        let mut strings = Vec::new();
        for value in values {
            let value = match value {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                    .unwrap_or(Value::Any(any)),
                other => other,
            };
            match value {
                Value::Str(s) => strings.push(s.to_string()),
                other => strings.push(other.display()),
            }
        }
        let body = strings.pop().unwrap_or_default();
        let params: Vec<String> = strings
            .iter()
            .flat_map(|p| p.split(','))
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let cache_key = format!("{}\u{0}{}", params.join(","), body);
        let (handle, factory) =
            self.link_generated(cache_key, |host| host.compile_function(&params, &body))?;
        if !factory {
            return Ok(handle);
        }
        // The entry builds the function: every construction is a fresh closure.
        let built = match self.invoke_function_value(handle, Vec::new())? {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                .unwrap_or(Value::Void),
            other => other,
        };
        match zun_gc::with_current_heap(|heap| crate::collections::unbox_function_box(built, heap)) {
            Value::Function(function) => Ok(function),
            _ => Err(RunError::Thrown(Value::Str(zun_core::ZStr::from(
                "EvalError: compiled function is empty",
            )))),
        }
    }

    /// `indirectEval(source)`: the `eval` function called as a value. A
    /// non-string argument is returned unchanged. A string is compiled by the
    /// host as global code and run with the global object as `this`; it sees
    /// neither the caller's locals nor the module's lexical bindings.
    pub(super) fn indirect_eval(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let argument = args.first().cloned().unwrap_or(Value::Void);
        let source = match &argument {
            Value::Str(text) => text.to_string(),
            Value::Any(any) => {
                match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
                    Some(Value::Str(text)) => text.to_string(),
                    _ => return Ok(argument),
                }
            }
            _ => return Ok(argument),
        };
        if self.eval_host.is_none() {
            return Err(codegen_disallowed());
        }
        // A leading byte no parameter list can contain keeps the two caches apart.
        let cache_key = format!("\u{1}eval\u{0}{source}");
        let (handle, _) = self.link_generated(cache_key, |host| host.compile_eval(&source))?;
        // Global code runs with `this` bound to the global object; the compiled
        // function declares that receiver only when the source mentions `this`.
        let takes_this = self
            .module
            .tables
            .method_defs
            .get((handle.method_rid() as usize).wrapping_sub(1))
            .is_some_and(|method| self.module.strings.get(method.name).contains("@this"));
        let call_args = if takes_this {
            vec![Value::Object(zun_gc::with_current_heap_mut(|heap| {
                heap.global_object()
            }))]
        } else {
            Vec::new()
        };
        self.invoke_function_value(handle, call_args)
    }

    /// Compile through the host, link the result into the live module, verify
    /// the appended methods and return the entry function, memoized by
    /// `cache_key`.
    fn link_generated(
        &mut self,
        cache_key: String,
        compile: impl FnOnce(&dyn EvalHost) -> Result<Generated, EvalError>,
    ) -> Result<(zun_core::FunctionHandle, bool), RunError> {
        let Some(host) = self.eval_host else {
            return Err(codegen_disallowed());
        };
        if let Some(cached) = self.eval_cache.get(&cache_key) {
            return Ok(*cached);
        }
        let thrown =
            |message: String| RunError::Thrown(Value::Str(zun_core::ZStr::from(message.as_str())));
        let Generated {
            module: ephemeral,
            entry,
            factory,
        } = compile(host).map_err(|e| thrown(format!("SyntaxError: {e}")))?;
        let (linked, placed) = self
            .module
            .link(&ephemeral)
            .map_err(|e| thrown(format!("EvalError: {e}")))?;
        if entry == 0 || entry > placed.count {
            return Err(thrown("EvalError: compiled function is empty".into()));
        }
        host.verify_linked(&linked, placed.first_rid, placed.count)
            .map_err(|e| thrown(format!("EvalError: {e}")))?;

        // Publish the grown module. The previous module stays alive (leaked) so
        // every borrow of it held by running frames stays valid; the leak is
        // bounded by the number of distinct sources (see `eval_cache`).
        let linked: &'static Module = Box::leak(Box::new(linked));
        self.module = linked;
        self.global_code
            .first_generated_rid
            .get_or_insert(placed.first_rid);
        let handle = zun_core::FunctionHandle::new(placed.first_rid + entry - 1, None);
        self.eval_cache.insert(cache_key, (handle, factory));
        Ok((handle, factory))
    }
}

/// The error every string-compiling entry point throws without an [`EvalHost`].
fn codegen_disallowed() -> RunError {
    RunError::Thrown(Value::Str(zun_core::ZStr::from(CODEGEN_DISALLOWED)))
}

/// Prelude functions that are built-in *function* globals: they read as native
/// code and are not constructors.
const NATIVE_GLOBAL_FUNCTIONS: &[&str] = &["eval"];

impl Interp<'_> {
    /// Whether `value` is the `Function` constructor (the realm descriptor
    /// behind the global `Function`).
    pub(crate) fn is_function_constructor(&self, value: &Value) -> bool {
        match value {
            Value::Any(any) => {
                any.as_class_descriptor()
                    .and_then(Self::builtin_ctor_global_name)
                    == Some("Function")
            }
            _ => false,
        }
    }

    /// The name of the built-in function global `handle` stands for, when it is
    /// one of [`NATIVE_GLOBAL_FUNCTIONS`] (a free prelude function, not a class
    /// member, bound function or built-in method value).
    pub(crate) fn native_global_function_name(
        &self,
        handle: zun_core::FunctionHandle,
    ) -> Option<&'static str> {
        if handle.builtin_method().is_some()
            || handle.is_bound()
            || handle.generator_method().is_some()
        {
            return None;
        }
        let rid = handle.method_rid();
        let method = self
            .module
            .tables
            .method_defs
            .get((rid as usize).wrapping_sub(1))?;
        let name = self.module.strings.get(method.name);
        let found = NATIVE_GLOBAL_FUNCTIONS.iter().copied().find(|n| *n == name)?;
        let class_member = self
            .module
            .tables
            .type_defs
            .iter()
            .any(|def| (def.method_start..def.method_end).contains(&rid));
        (!class_member).then_some(found)
    }

    /// `function <name>() { [native code] }` for a built-in function value: a
    /// realm constructor descriptor, a coercion constructor (`String`), or a
    /// built-in function global (`eval`). `None` for every other value.
    pub(crate) fn native_function_text(&self, value: &Value) -> Option<String> {
        let name = match value {
            Value::Any(any) => {
                match self.builtin_ctor_own_value(any.as_class_descriptor()?, "name")? {
                    Value::Str(name) => name.to_string(),
                    _ => return None,
                }
            }
            Value::Function(handle) => self
                .coerce_ctor_name(*handle)
                .or_else(|| self.native_global_function_name(*handle))?
                .to_string(),
            _ => return None,
        };
        Some(format!("function {name}() {{ [native code] }}"))
    }
}

impl Interp<'_> {
    /// [`Interp::function_from_strings`] for a dynamic call site, whose result
    /// travels as `any`: the function rides the managed cell a function takes
    /// when it is stored in an `any` slot.
    pub(crate) fn function_from_strings_boxed(
        &mut self,
        values: Vec<Value>,
    ) -> Result<Value, RunError> {
        let handle = self.function_from_strings(values)?;
        let cell = self.call_managed_collection("functionBox", &[Value::Function(handle)])?;
        let boxed = zun_gc::with_current_heap_mut(|heap| cell.box_any(heap))
            .unwrap_or_else(zun_core::Any::void);
        Ok(Value::Any(boxed))
    }
}
