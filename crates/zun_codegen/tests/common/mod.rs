#![allow(dead_code)]

use zun_codegen::RetVal;
use zun_interp::value::Value;
use zun_interp::CaptureOutput;

pub fn compile(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true)
        .unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

/// Run the entry point through the interpreter, returning its value.
pub fn interp_entry(module: &zun_module::Module) -> Value {
    let mut out = CaptureOutput::default();
    zun_interp::run(module, &mut out).unwrap()
}

/// Assert the JIT and interpreter agree on the entry point's result.
pub fn assert_equivalent(src: &str) {
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();

    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(
        jit.is_compiled(entry),
        "entry should be JIT-eligible:\n{src}"
    );

    let jit_result = jit.call_entry(entry).expect("jit call");
    let interp_result = interp_entry(&module);

    let matches = match (&jit_result, &interp_result) {
        (RetVal::Float(a), Value::Float(b, _)) => a == b,
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        (RetVal::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    };
    assert!(
        matches,
        "JIT {jit_result:?} vs interpreter {interp_result:?} for:\n{src}"
    );
}

pub fn compile_oxc(src: &str) -> zun_module::Module {
    let ast = zun_syntax::oxc::parse_subset(src, true).expect("oxc parse");
    let mut diags = Vec::new();
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

/// Run `body` as `main(): int` through both frontends and expect `expected`.
pub fn assert_regex_program(body: &str, expected: i64) {
    let src = format!("function main(): int {{ {body} }}");
    for (name, module) in [("hand", compile(&src)), ("oxc", compile_oxc(&src))] {
        match interp_entry(&module) {
            Value::Int(v, _) => assert_eq!(v, expected, "{name}: {src}"),
            other => panic!("{name}: unexpected result {other:?} for {src}"),
        }
    }
}

/// One argument of a string-method call driven through both tiers.
pub enum CallArg<'a> {
    Str(&'a str),
    U64(u64),
}

/// What the method under test returns.
#[derive(Clone, Copy)]
pub enum CallRet {
    Str,
    Int,
}

/// Rendered result of a call, comparable across tiers.
#[derive(Debug, PartialEq, Eq)]
pub enum CallOut {
    Str(String),
    Int(i32),
}

fn method_rid(module: &zun_module::Module, name: &str) -> u32 {
    module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == name)
        .unwrap_or_else(|| panic!("method `{name}` not found")) as u32
        + 1
}

/// Run `method` of `src` in Tier 0 and return its rendered result.
pub fn interp_call(
    module: &zun_module::Module,
    method: &str,
    args: &[CallArg],
    ret: CallRet,
) -> Result<CallOut, String> {
    let values = args
        .iter()
        .map(|arg| match arg {
            CallArg::Str(text) => Value::Str(text.to_string().into()),
            CallArg::U64(number) => Value::Int(*number as i64, zun_il::NumKind::U64),
        })
        .collect();
    let mut out = CaptureOutput::default();
    match zun_interp::run_method(module, method_rid(module, method), values, &mut out) {
        Ok(Value::Str(text)) if matches!(ret, CallRet::Str) => Ok(CallOut::Str(text.to_string())),
        Ok(Value::Int(number, _)) if matches!(ret, CallRet::Int) => {
            Ok(CallOut::Int(number as i32))
        }
        Ok(other) => panic!("unexpected interpreter result {other:?}"),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// Assert the JIT-compiled `method` and Tier 0 return the same result for
/// `args`. Entry points with string literals are not JIT-eligible, so the
/// strings enter as parameters through the managed string ABI instead.
pub fn assert_string_call_equivalent(src: &str, method: &str, args: &[CallArg], ret: CallRet) {
    let module = compile(src);
    let rid = method_rid(&module, method);
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid), "`{method}` should be JIT-eligible:\n{src}");

    let mut heap = zun_gc::Heap::new();
    // Occupy handle 0 so a thunk returning the null handle cannot alias an argument.
    heap.intern_string(std::rc::Rc::from("<handle-0-sentinel>"));
    let packed: Vec<u64> = args
        .iter()
        .map(|arg| match arg {
            CallArg::Str(text) => u64::from(heap.intern_string(std::rc::Rc::from(*text))),
            CallArg::U64(number) => *number,
        })
        .collect();
    let raw = jit
        .call_packed_with_heap(rid, &packed, &mut heap)
        .expect("jit call");
    let jit_out = match ret {
        CallRet::Str => CallOut::Str(
            heap.try_string(raw as u32)
                .map(|text| text.to_string())
                .unwrap_or_else(|| format!("<null string handle {raw}>")),
        ),
        CallRet::Int => CallOut::Int(raw as u32 as i32),
    };
    let interp_out = interp_call(&module, method, args, ret).expect("interpreter call");
    assert_eq!(
        jit_out,
        interp_out,
        "JIT vs interpreter for `{method}` with {}",
        args.iter()
            .map(|arg| match arg {
                CallArg::Str(text) => format!("{text:?}"),
                CallArg::U64(number) => number.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    );
}
