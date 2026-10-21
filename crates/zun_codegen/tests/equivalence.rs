//! Tier 0 vs Tier 1 equivalence for the typed JIT.
//!
//! Each test compiles a program whose `main` is a nullary numeric entry, runs
//! it through both the interpreter and the Cranelift JIT, and requires matching
//! results — now across integer widths, signedness, and f32, not just f64.

use zun_codegen::RetVal;
use zun_interp::value::Value;
use zun_interp::CaptureOutput;

fn compile(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

/// Run the entry point through the interpreter, returning its value.
fn interp_entry(module: &zun_module::Module) -> Value {
    let mut out = CaptureOutput::default();
    zun_interp::run(module, &mut out).unwrap()
}

/// Assert the JIT and interpreter agree on the entry point's result.
fn assert_equivalent(src: &str) {
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

#[test]
fn array_for_of_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let values = new Array(2); values[0] = 20.0; values[1] = 22.0; for (let value of values) { print(value); } return 42.0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "array iteration requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn interned_string_length_uses_the_typed_jit_abi() {
    let module = compile(
        "import { length } from \"zun:string\"; function measure(value: string): u64 { return length(value); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid), "measure RID {rid}, methods: {:?}", module.tables.method_defs);

    let mut heap = zun_gc::Heap::new();
    let handle = heap.intern_string(std::rc::Rc::from("hé"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[handle as u64], &mut heap),
        Ok(2)
    ));
}

#[test]
fn primitive_string_length_callvirt_uses_the_direct_handle_path() {
    let module = compile(
        "import { length } from \"zun:string\"; class String { extern length(): u64; } function measure(value: string): u64 { return value.length(); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let handle = heap.intern_string(std::rc::Rc::from("zun"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[handle as u64], &mut heap),
        Ok(3)
    ));
}

#[test]
fn string_literal_callvirt_uses_the_runtime_literal_abi() {
    let module = compile(
        "import { contains } from \"zun:string\"; class String { extern contains(needle: string): boolean; } function main(): boolean { return \"zun\".contains(\"u\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    let jit_result = jit
        .call_entry_with_heap(entry, &mut heap)
        .expect("jit call");
    assert_eq!(jit_result, RetVal::Bool(true));
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn primitive_string_contains_uses_the_managed_string_abi() {
    let module = compile(
        "import { contains } from \"zun:string\"; class String { extern contains(needle: string): boolean; } function measure(value: string, needle: string): boolean { return value.contains(needle); } function main(): boolean { return false; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let needle = heap.intern_string(std::rc::Rc::from("run"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, needle as u64], &mut heap),
        Ok(1)
    ));
}

#[test]
fn primitive_string_char_code_at_uses_the_managed_string_abi() {
    let module = compile(
        "import { charCodeAt } from \"zun:string\"; class String { extern charCodeAt(index: u64): u32; } function measure(value: string, index: u64): u32 { return value.charCodeAt(index); } function main(): u32 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("Az"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, 1], &mut heap),
        Ok(122)
    ));
}

#[test]
fn primitive_string_prefix_uses_the_managed_string_abi() {
    let module = compile(
        "import { startsWith } from \"zun:string\"; class String { extern startsWith(prefix: string): boolean; } function measure(value: string, prefix: string): boolean { return value.startsWith(prefix); } function main(): boolean { return false; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let prefix = heap.intern_string(std::rc::Rc::from("zun"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, prefix as u64], &mut heap),
        Ok(1)
    ));
}

#[test]
fn primitive_string_suffix_uses_the_managed_string_abi() {
    let module = compile(
        "import { endsWith } from \"zun:string\"; class String { extern endsWith(suffix: string): boolean; } function measure(value: string, suffix: string): boolean { return value.endsWith(suffix); } function main(): boolean { return false; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let suffix = heap.intern_string(std::rc::Rc::from("time"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, suffix as u64], &mut heap),
        Ok(1)
    ));
}

#[test]
fn primitive_string_index_of_uses_the_managed_string_abi() {
    let module = compile(
        "import { indexOf } from \"zun:string\"; class String { extern indexOf(needle: string): int; } function measure(value: string, needle: string): int { return value.indexOf(needle); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let needle = heap.intern_string(std::rc::Rc::from("run"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, needle as u64], &mut heap),
        Ok(4)
    ));
}

#[test]
fn primitive_string_last_index_of_uses_the_managed_string_abi() {
    let module = compile(
        "import { lastIndexOf } from \"zun:string\"; class String { extern lastIndexOf(needle: string): int; } function measure(value: string, needle: string): int { return value.lastIndexOf(needle); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime runtime"));
    let needle = heap.intern_string(std::rc::Rc::from("run"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, needle as u64], &mut heap),
        Ok(12)
    ));
}

#[test]
fn primitive_string_count_uses_the_managed_string_abi() {
    let module = compile(
        "import { count } from \"zun:string\"; class String { extern count(needle: string): int; } function measure(value: string, needle: string): int { return value.count(needle); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("aaaaa"));
    let needle = heap.intern_string(std::rc::Rc::from("aa"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[value as u64, needle as u64], &mut heap),
        Ok(2)
    ));
}

#[test]
fn primitive_string_compare_uses_the_managed_string_abi() {
    let module = compile(
        "import { compare } from \"zun:string\"; class String { extern compare(other: string): int; } function measure(value: string, other: string): int { return value.compare(other); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let left = heap.intern_string(std::rc::Rc::from("alpha"));
    let right = heap.intern_string(std::rc::Rc::from("beta"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[left as u64, right as u64], &mut heap),
        Ok(u64::MAX)
    ));
}

#[test]
fn primitive_string_concat_uses_the_managed_string_abi() {
    let module = compile(
        "import { concat } from \"zun:string\"; class String { extern concat(other: string): string; } function measure(value: string, other: string): string { return value.concat(other); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let left = heap.intern_string(std::rc::Rc::from("zun "));
    let right = heap.intern_string(std::rc::Rc::from("runtime"));
    let handle = jit
        .call_packed_with_heap(rid, &[left as u64, right as u64], &mut heap)
        .expect("concat call");
    assert_eq!(heap.string(handle as u32).as_ref(), "zun runtime");
}

#[test]
fn primitive_string_substring_uses_the_managed_string_abi() {
    let module = compile(
        "import { substring } from \"zun:string\"; class String { extern substring(start: u64, end: u64): string; } function measure(value: string): string { return value.substring(1, 4); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let handle = jit
        .call_packed_with_heap(rid, &[value as u64], &mut heap)
        .expect("substring call");
    assert_eq!(heap.string(handle as u32).as_ref(), "un ");
}

#[test]
fn primitive_string_trim_uses_the_managed_string_abi() {
    let module = compile(
        "import { trim } from \"zun:string\"; class String { extern trim(): string; } function measure(value: string): string { return value.trim(); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("  zun runtime  "));
    let handle = jit
        .call_packed_with_heap(rid, &[value as u64], &mut heap)
        .expect("trim call");
    assert_eq!(heap.string(handle as u32).as_ref(), "zun runtime");
}

#[test]
fn primitive_string_trim_boundaries_use_the_managed_string_abi() {
    let module = compile(
        "import { trimStart, trimEnd } from \"zun:string\"; class String { extern trimStart(): string; extern trimEnd(): string; } function measure(value: string): string { return value.trimStart().trimEnd(); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("  zun runtime  "));
    let handle = jit
        .call_packed_with_heap(rid, &[value as u64], &mut heap)
        .expect("trim boundary call");
    assert_eq!(heap.string(handle as u32).as_ref(), "zun runtime");
}

#[test]
fn primitive_string_case_conversion_uses_the_managed_string_abi() {
    let module = compile(
        "import { toLowerCase, toUpperCase } from \"zun:string\"; class String { extern toLowerCase(): string; extern toUpperCase(): string; } function lower(value: string): string { return value.toLowerCase(); } function upper(value: string): string { return value.toUpperCase(); } function main(): int { return 0; }",
    );
    let lower_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "lower")
        .expect("lower method") as u32
        + 1;
    let upper_rid = lower_rid + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(lower_rid));
    assert!(jit.is_compiled(upper_rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("ZuN"));
    let lower = jit
        .call_packed_with_heap(lower_rid, &[value as u64], &mut heap)
        .expect("lowercase call");
    let upper = jit
        .call_packed_with_heap(upper_rid, &[value as u64], &mut heap)
        .expect("uppercase call");
    assert_eq!(heap.string(lower as u32).as_ref(), "zun");
    assert_eq!(heap.string(upper as u32).as_ref(), "ZUN");
}

#[test]
fn primitive_string_repeat_uses_the_managed_string_abi() {
    let module = compile(
        "import { repeat } from \"zun:string\"; class String { extern repeat(count: u64): string; } function measure(value: string): string { return value.repeat(3); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun"));
    let handle = jit
        .call_packed_with_heap(rid, &[value as u64], &mut heap)
        .expect("repeat call");
    assert_eq!(heap.string(handle as u32).as_ref(), "zunzunzun");
}

#[test]
fn primitive_string_reverse_uses_the_managed_string_abi() {
    let module = compile(
        "import { reverse } from \"zun:string\"; class String { extern reverse(): string; } function measure(value: string): string { return value.reverse(); } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun"));
    let handle = jit
        .call_packed_with_heap(rid, &[value as u64], &mut heap)
        .expect("reverse call");
    assert_eq!(heap.string(handle as u32).as_ref(), "nuz");
}

#[test]
fn primitive_string_padding_uses_the_managed_string_abi() {
    let module = compile(
        "import { padStart, padEnd } from \"zun:string\"; class String { extern padStart(target: u64, fill: string): string; extern padEnd(target: u64, fill: string): string; } function left(value: string, fill: string): string { return value.padStart(5, fill); } function right(value: string, fill: string): string { return value.padEnd(5, fill); } function main(): int { return 0; }",
    );
    let left_rid = module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == "left").expect("left method") as u32 + 1;
    let right_rid = left_rid + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(left_rid));
    assert!(jit.is_compiled(right_rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun"));
    let zero = heap.intern_string(std::rc::Rc::from("0"));
    let bang = heap.intern_string(std::rc::Rc::from("!"));
    let left = jit.call_packed_with_heap(left_rid, &[value as u64, zero as u64], &mut heap).expect("pad start call");
    let right = jit.call_packed_with_heap(right_rid, &[value as u64, bang as u64], &mut heap).expect("pad end call");
    assert_eq!(heap.string(left as u32).as_ref(), "00zun");
    assert_eq!(heap.string(right as u32).as_ref(), "zun!!");
}

#[test]
fn primitive_string_replace_uses_the_managed_string_abi() {
    let module = compile(
        "import { replace } from \"zun:string\"; class String { extern replace(needle: string, replacement: string): string; } function measure(value: string, needle: string, replacement: string): string { return value.replace(needle, replacement); } function main(): int { return 0; }",
    );
    let rid = module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == "measure").expect("measure method") as u32 + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun vm"));
    let needle = heap.intern_string(std::rc::Rc::from("vm"));
    let replacement = heap.intern_string(std::rc::Rc::from("runtime"));
    let handle = jit.call_packed_with_heap(rid, &[value as u64, needle as u64, replacement as u64], &mut heap).expect("replace call");
    assert_eq!(heap.string(handle as u32).as_ref(), "zun runtime");
}

#[test]
fn primitive_string_builtin_conversions_use_the_managed_string_abi() {
    let module = compile(
        "import { toInt, toFloat, toBoolean } from \"zun:string\"; function asInt(value: string): int { return toInt(value); } function asFloat(value: string): number { return toFloat(value); } function asBool(value: string): boolean { return toBoolean(value); } function main(): int { return 0; }",
    );
    let rid = |name: &str| module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == name).expect("method") as u32 + 1;
    let int_rid = rid("asInt");
    let float_rid = rid("asFloat");
    let bool_rid = rid("asBool");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(int_rid));
    assert!(jit.is_compiled(float_rid));
    assert!(jit.is_compiled(bool_rid));

    let mut heap = zun_gc::Heap::new();
    let int_value = heap.intern_string(std::rc::Rc::from(" -42 "));
    let float_value = heap.intern_string(std::rc::Rc::from(" 3.5 "));
    let bool_value = heap.intern_string(std::rc::Rc::from(" true "));
    assert_eq!(jit.call_packed_with_heap(int_rid, &[int_value as u64], &mut heap).unwrap() as i32, -42);
    assert_eq!(f64::from_bits(jit.call_packed_with_heap(float_rid, &[float_value as u64], &mut heap).unwrap()), 3.5);
    assert_eq!(jit.call_packed_with_heap(bool_rid, &[bool_value as u64], &mut heap).unwrap(), 1);
}

#[test]
fn primitive_string_method_conversions_bind_to_builtins() {
    let module = compile(
        "import { toInt, toFloat, toBoolean } from \"zun:string\"; class String { extern toInt(): int; extern toFloat(): number; extern toBoolean(): boolean; } function asInt(value: string): int { return value.toInt(); } function asFloat(value: string): number { return value.toFloat(); } function asBool(value: string): boolean { return value.toBoolean(); } function main(): int { return 0; }",
    );
    let rid = |name: &str| module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == name).expect("method") as u32 + 1;
    let int_rid = rid("asInt");
    let float_rid = rid("asFloat");
    let bool_rid = rid("asBool");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(int_rid));
    assert!(jit.is_compiled(float_rid));
    assert!(jit.is_compiled(bool_rid));

    let mut heap = zun_gc::Heap::new();
    let int_value = heap.intern_string(std::rc::Rc::from(" -42 "));
    let float_value = heap.intern_string(std::rc::Rc::from(" 3.5 "));
    let bool_value = heap.intern_string(std::rc::Rc::from(" true "));
    assert_eq!(jit.call_packed_with_heap(int_rid, &[int_value as u64], &mut heap).unwrap() as i32, -42);
    assert_eq!(f64::from_bits(jit.call_packed_with_heap(float_rid, &[float_value as u64], &mut heap).unwrap()), 3.5);
    assert_eq!(jit.call_packed_with_heap(bool_rid, &[bool_value as u64], &mut heap).unwrap(), 1);
}

#[test]
fn generator_for_of_preserves_tier0_fallback_boundary() {
    let module = compile(
        "function* values(): int { yield 7; return 9; } function main(): any { for (let value of values()) { return value; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "generator suspension requires Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(7.0, _)));
}

#[test]
fn generator_arguments_preserve_tier0_fallback_boundary() {
    let module = compile(
        "function* values(seed: number): int { yield seed; yield seed + 1; return 9; } function main(): any { let total: number = 0.0; for (let value of values(2)) { total = total + (value as number); } return total; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "generator suspension requires Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(5.0, _)));
}

#[test]
fn jit_can_allocate_an_array_for_a_scalar_length_result_with_heap_scope() {
    let module = compile(
        "function main(): u64 { let values = new Array(3); return values.length; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(3))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn jit_can_allocate_an_object_and_read_a_numeric_field_with_heap_scope() {
    let module = compile(
        "class Box { value: int; }
         function read(box: Box): int { return box.value; }
         function main(): int { return read(new Box(7)); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(7))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(7, _)));
}

#[test]
fn jit_preserves_object_references_through_managed_fields() {
    let module = compile(
        "class Box { value: int; }
         class Holder { box: Box; }
         function main(): int { let value = new Box(7); let holder = new Holder(value); return holder.box.value; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(7))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(7, _)));
}

#[test]
fn jit_stores_and_reloads_managed_object_references() {
    let module = compile(
        "class Box { value: int; }
         class Holder { box: Box; }
         function main(): int { let first = new Box(1); let second = new Box(9); let holder = new Holder(first); holder.box = second; return holder.box.value; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(9))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(9, _)));
}

#[test]
fn managed_collections_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { mapNew, mapSet, mapHas } from \"zun:collections\"; function main(): int { let map = mapNew(); mapSet(map, 7.0, 42.0); if (mapHas(map, 7.0)) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "managed collections require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_map_and_set_size_methods_use_typed_object_receivers() {
    let module = compile(
        "function mapSize(value: Map): u64 { return value.size(); } function setSize(value: Set): u64 { return value.size(); } function main(): u64 { return 0; }",
    );
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapSize")
        .expect("mapSize method") as u32
        + 1;
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setSize")
        .expect("setSize method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));

    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(2));
    heap.set_add(set, zun_core::Any::int(3));
    assert!(matches!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64], &mut heap),
        Ok(1)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64], &mut heap),
        Ok(1)
    ));
}

#[test]
fn numeric_map_and_set_has_use_typed_jit_abis() {
    let module = compile(
        "import { mapHasF64, setHasF64, setAddF64, mapDeleteF64, setDeleteF64, mapSetF64, mapGetF64Or, mapHasI64, setHasI64, setAddI64, mapDeleteI64, setDeleteI64, mapSetI64, mapGetI64Or } from \"zun:collections\"; function mapContains(value: Map, key: number): boolean { return mapHasF64(value, key); } function setContains(value: Set, key: number): boolean { return setHasF64(value, key); } function setInsert(value: Set, key: number): boolean { return setAddF64(value, key); } function mapRemove(value: Map, key: number): boolean { return mapDeleteF64(value, key); } function setRemove(value: Set, key: number): boolean { return setDeleteF64(value, key); } function mapInsert(value: Map, key: number, item: number): boolean { return mapSetF64(value, key, item); } function mapRead(value: Map, key: number, fallback: number): number { return mapGetF64Or(value, key, fallback); } function mapContainsI64(value: Map, key: i64): boolean { return mapHasI64(value, key); } function setContainsI64(value: Set, key: i64): boolean { return setHasI64(value, key); } function setInsertI64(value: Set, key: i64): boolean { return setAddI64(value, key); } function mapRemoveI64(value: Map, key: i64): boolean { return mapDeleteI64(value, key); } function setRemoveI64(value: Set, key: i64): boolean { return setDeleteI64(value, key); } function mapInsertI64(value: Map, key: i64, item: i64): boolean { return mapSetI64(value, key, item); } function mapReadI64(value: Map, key: i64, fallback: i64): i64 { return mapGetI64Or(value, key, fallback); } function main(): u64 { return 0; }",
    );
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapContains")
        .expect("mapContains method") as u32
        + 1;
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setContains")
        .expect("setContains method") as u32
        + 1;
    let insert_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setInsert")
        .expect("setInsert method") as u32
        + 1;
    let map_remove_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapRemove")
        .expect("mapRemove method") as u32
        + 1;
    let set_remove_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setRemove")
        .expect("setRemove method") as u32
        + 1;
    let map_insert_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapInsert")
        .expect("mapInsert method") as u32
        + 1;
    let map_read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapRead")
        .expect("mapRead method") as u32
        + 1;
    let method_rid = |name: &str| -> u32 {
        module.tables.method_defs.iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("typed collection method") as u32 + 1
    };
    let map_i64_rid = method_rid("mapContainsI64");
    let set_i64_rid = method_rid("setContainsI64");
    let add_i64_rid = method_rid("setInsertI64");
    let map_remove_i64_rid = method_rid("mapRemoveI64");
    let set_remove_i64_rid = method_rid("setRemoveI64");
    let map_insert_i64_rid = method_rid("mapInsertI64");
    let map_read_i64_rid = method_rid("mapReadI64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));
    assert!(jit.is_compiled(insert_rid));
    assert!(jit.is_compiled(map_remove_rid));
    assert!(jit.is_compiled(set_remove_rid));
    assert!(jit.is_compiled(map_insert_rid));
    assert!(jit.is_compiled(map_read_rid));
    assert!(jit.is_compiled(map_i64_rid));
    assert!(jit.is_compiled(set_i64_rid));
    assert!(jit.is_compiled(add_i64_rid));
    assert!(jit.is_compiled(map_remove_i64_rid));
    assert!(jit.is_compiled(set_remove_i64_rid));
    assert!(jit.is_compiled(map_insert_i64_rid));
    assert!(jit.is_compiled(map_read_i64_rid));

    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::float(7.5), zun_core::Any::float(42.0));
    heap.set_add(set, zun_core::Any::float(7.5));
    let key = 7.5f64.to_bits();
    assert_eq!(jit.call_packed_with_heap(map_rid, &[map.addr() as u64, key], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(set_rid, &[set.addr() as u64, key], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(insert_rid, &[set.addr() as u64, 8.5f64.to_bits()], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(set_rid, &[set.addr() as u64, 8.5f64.to_bits()], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(map_insert_rid, &[map.addr() as u64, 8.5f64.to_bits(), 9.5f64.to_bits()], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(map_read_rid, &[map.addr() as u64, 8.5f64.to_bits(), 0.0f64.to_bits()], &mut heap).unwrap(), 9.5f64.to_bits());
    assert_eq!(jit.call_packed_with_heap(map_read_rid, &[map.addr() as u64, 99.0f64.to_bits(), 4.5f64.to_bits()], &mut heap).unwrap(), 4.5f64.to_bits());
    assert_eq!(jit.call_packed_with_heap(map_rid, &[map.addr() as u64, 8.5f64.to_bits()], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(map_remove_rid, &[map.addr() as u64, key], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(set_remove_rid, &[set.addr() as u64, key], &mut heap).unwrap(), 1);
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(42));
    heap.set_add(set, zun_core::Any::int(7));
    assert_eq!(jit.call_packed_with_heap(map_i64_rid, &[map.addr() as u64, 7], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(set_i64_rid, &[set.addr() as u64, 7], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(add_i64_rid, &[set.addr() as u64, 8], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(map_insert_i64_rid, &[map.addr() as u64, 8, 9], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(map_read_i64_rid, &[map.addr() as u64, 8, 0], &mut heap).unwrap(), 9);
    assert_eq!(jit.call_packed_with_heap(map_remove_i64_rid, &[map.addr() as u64, 8], &mut heap).unwrap(), 1);
    assert_eq!(jit.call_packed_with_heap(set_remove_i64_rid, &[set.addr() as u64, 8], &mut heap).unwrap(), 1);
}

#[test]
fn array_buffer_length_uses_the_managed_object_abi() {
    let module = compile(
        "import { bufferLength } from \"zun:buffer\"; function bufferSize(value: ArrayBuffer): u64 { return bufferLength(value); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "bufferSize")
        .expect("bufferSize method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(17);
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[buffer.addr() as u64], &mut heap),
        Ok(17)
    ));
}

#[test]
fn managed_view_lengths_use_typed_object_abis() {
    let module = compile(
        "import { dataViewLength, dataViewByteLength, u8ArrayLength, u8ArrayByteLength, u16ArrayLength, u16ArrayByteLength, u32ArrayLength, u32ArrayByteLength, f64ArrayLength, f64ArrayByteLength } from \"zun:buffer\"; function data(view: DataView): u64 { return dataViewLength(view); } function dataBytes(view: DataView): u64 { return dataViewByteLength(view); } function u8(view: Uint8Array): u64 { return u8ArrayLength(view); } function u8Bytes(view: Uint8Array): u64 { return u8ArrayByteLength(view); } function u16(view: Uint16Array): u64 { return u16ArrayLength(view); } function u16Bytes(view: Uint16Array): u64 { return u16ArrayByteLength(view); } function u32(view: Uint32Array): u64 { return u32ArrayLength(view); } function u32Bytes(view: Uint32Array): u64 { return u32ArrayByteLength(view); } function f64(view: Float64Array): u64 { return f64ArrayLength(view); } function f64Bytes(view: Float64Array): u64 { return f64ArrayByteLength(view); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("view length method") as u32
            + 1
    };
    let data = find("data");
    let data_bytes = find("dataBytes");
    let u8 = find("u8");
    let u8_bytes = find("u8Bytes");
    let u16 = find("u16");
    let u16_bytes = find("u16Bytes");
    let u32 = find("u32");
    let u32_bytes = find("u32Bytes");
    let f64 = find("f64");
    let f64_bytes = find("f64Bytes");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(data));
    assert!(jit.is_compiled(data_bytes));
    assert!(jit.is_compiled(u8));
    assert!(jit.is_compiled(u8_bytes));
    assert!(jit.is_compiled(u16));
    assert!(jit.is_compiled(u16_bytes));
    assert!(jit.is_compiled(u32));
    assert!(jit.is_compiled(u32_bytes));
    assert!(jit.is_compiled(f64));
    assert!(jit.is_compiled(f64_bytes));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(32);
    let data_view = heap.alloc_data_view(buffer, 0, 12).expect("data view");
    let u8_view = heap.alloc_u8_array(buffer, 0, 5).expect("u8 view");
    let u16_view = heap.alloc_u16_array(buffer, 0, 8).expect("u16 view");
    let u32_view = heap.alloc_u32_array(buffer, 0, 8).expect("u32 view");
    let f64_view = heap.alloc_f64_array(buffer, 0, 16).expect("f64 view");
    assert!(matches!(jit.call_packed_with_heap(data, &[data_view.addr() as u64], &mut heap), Ok(12)));
    assert!(matches!(jit.call_packed_with_heap(data_bytes, &[data_view.addr() as u64], &mut heap), Ok(12)));
    assert!(matches!(jit.call_packed_with_heap(u8, &[u8_view.addr() as u64], &mut heap), Ok(5)));
    assert!(matches!(jit.call_packed_with_heap(u8_bytes, &[u8_view.addr() as u64], &mut heap), Ok(5)));
    assert!(matches!(jit.call_packed_with_heap(u16, &[u16_view.addr() as u64], &mut heap), Ok(4)));
    assert!(matches!(jit.call_packed_with_heap(u16_bytes, &[u16_view.addr() as u64], &mut heap), Ok(8)));
    assert!(matches!(jit.call_packed_with_heap(u32, &[u32_view.addr() as u64], &mut heap), Ok(2)));
    assert!(matches!(jit.call_packed_with_heap(u32_bytes, &[u32_view.addr() as u64], &mut heap), Ok(8)));
    assert!(matches!(jit.call_packed_with_heap(f64, &[f64_view.addr() as u64], &mut heap), Ok(2)));
    assert!(matches!(jit.call_packed_with_heap(f64_bytes, &[f64_view.addr() as u64], &mut heap), Ok(16)));
}

#[test]
fn array_buffer_u8_read_uses_the_scalar_managed_abi() {
    let module = compile(
        "import { bufferGetU8 } from \"zun:buffer\"; function read(buffer: ArrayBuffer, index: u64): u8 { return bufferGetU8(buffer, index); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("read method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(2);
    heap.buffer_set(buffer, 1, 0xab);
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[buffer.addr() as u64, 1], &mut heap),
        Ok(0xab)
    ));
}

#[test]
fn array_buffer_numeric_reads_use_width_specific_managed_abis() {
    let module = compile(
        "import { bufferReadU32, bufferReadF64 } from \"zun:buffer\"; function readU32(buffer: ArrayBuffer, offset: u64): u32 { return bufferReadU32(buffer, offset); } function readF64(buffer: ArrayBuffer, offset: u64): number { return bufferReadF64(buffer, offset); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("buffer reader method") as u32
            + 1
    };
    let u32_rid = find("readU32");
    let f64_rid = find("readF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u32_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    heap.buffer_write_u32(buffer, 0, 0xfeed_beef);
    heap.buffer_write_f64(buffer, 8, 2.5);
    assert!(matches!(
        jit.call_packed_with_heap(u32_rid, &[buffer.addr() as u64, 0], &mut heap),
        Ok(0xfeed_beef)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(f64_rid, &[buffer.addr() as u64, 8], &mut heap),
        Ok(value) if f64::from_bits(value) == 2.5
    ));
}

#[test]
fn array_buffer_numeric_writes_use_scalar_managed_abis() {
    let module = compile(
        "import { bufferSetU8, bufferWriteU32, bufferWriteF64 } from \"zun:buffer\"; function setU8(buffer: ArrayBuffer, index: u64, value: u8): boolean { return bufferSetU8(buffer, index, value); } function writeU32(buffer: ArrayBuffer, offset: u64, value: u32): boolean { return bufferWriteU32(buffer, offset, value); } function writeF64(buffer: ArrayBuffer, offset: u64, value: number): boolean { return bufferWriteF64(buffer, offset, value); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("buffer writer method") as u32
            + 1
    };
    let u8_rid = find("setU8");
    let u32_rid = find("writeU32");
    let f64_rid = find("writeF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u8_rid));
    assert!(jit.is_compiled(u32_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    assert!(matches!(
        jit.call_packed_with_heap(u8_rid, &[buffer.addr() as u64, 1, 0xab], &mut heap),
        Ok(1)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(u32_rid, &[buffer.addr() as u64, 4, 0xfeed_beef], &mut heap),
        Ok(1)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(f64_rid, &[buffer.addr() as u64, 8, 2.5f64.to_bits()], &mut heap),
        Ok(1)
    ));
    assert_eq!(heap.buffer_get(buffer, 1), Some(0xab));
    assert_eq!(heap.buffer_read_u32(buffer, 4), Some(0xfeed_beef));
    assert_eq!(heap.buffer_read_f64(buffer, 8), Some(2.5));
}

#[test]
fn data_view_u32_read_uses_the_typed_view_abi() {
    let module = compile(
        "import { dataViewGetU32 } from \"zun:buffer\"; function read(view: DataView, offset: u64, little: boolean): u32 { return dataViewGetU32(view, offset, little); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("data view reader") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(4);
    heap.buffer_write_u32(buffer, 0, 0xfeed_beef);
    let view = heap.alloc_data_view(buffer, 0, 4).expect("data view");
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[view.addr() as u64, 0, 1], &mut heap),
        Ok(0xfeed_beef)
    ));
}

#[test]
fn data_view_u16_and_f64_reads_use_width_specific_abis() {
    let module = compile(
        "import { dataViewGetU16, dataViewGetF64 } from \"zun:buffer\"; function readU16(view: DataView, offset: u64, little: boolean): u16 { return dataViewGetU16(view, offset, little); } function readF64(view: DataView, offset: u64, little: boolean): number { return dataViewGetF64(view, offset, little); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("data view reader") as u32
            + 1
    };
    let u16_rid = find("readU16");
    let f64_rid = find("readF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u16_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    heap.buffer_write_u32(buffer, 0, 0xfeed_beef);
    heap.buffer_write_f64(buffer, 8, 2.5);
    let view = heap.alloc_data_view(buffer, 0, 16).expect("data view");
    assert!(matches!(
        jit.call_packed_with_heap(u16_rid, &[view.addr() as u64, 0, 1], &mut heap),
        Ok(0xbeef)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(f64_rid, &[view.addr() as u64, 8, 1], &mut heap),
        Ok(value) if f64::from_bits(value) == 2.5
    ));
}

#[test]
fn data_view_scalar_writes_use_width_specific_abis() {
    let module = compile(
        "import { dataViewSetU32, dataViewSetU16, dataViewSetF64 } from \"zun:buffer\"; function setU32(view: DataView, offset: u64, value: u32, little: boolean): boolean { return dataViewSetU32(view, offset, value, little); } function setU16(view: DataView, offset: u64, value: u16, little: boolean): boolean { return dataViewSetU16(view, offset, value, little); } function setF64(view: DataView, offset: u64, value: number, little: boolean): boolean { return dataViewSetF64(view, offset, value, little); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("data view writer") as u32
            + 1
    };
    let u32_rid = find("setU32");
    let u16_rid = find("setU16");
    let f64_rid = find("setF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u32_rid));
    assert!(jit.is_compiled(u16_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    let view = heap.alloc_data_view(buffer, 0, 16).expect("data view");
    assert!(matches!(jit.call_packed_with_heap(u32_rid, &[view.addr() as u64, 0, 0xfeed_beef, 1], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(u16_rid, &[view.addr() as u64, 4, 0xbeef, 1], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(f64_rid, &[view.addr() as u64, 8, 2.5f64.to_bits(), 1], &mut heap), Ok(1)));
    assert_eq!(heap.data_view_get_u32(view, 0, true), Some(0xfeed_beef));
    assert_eq!(heap.data_view_get_u16(view, 4, true), Some(0xbeef));
    assert_eq!(heap.data_view_get_f64(view, 8, true), Some(2.5));
}

#[test]
fn u8_array_access_uses_the_typed_view_abi() {
    let module = compile(
        "import { u8ArrayGet, u8ArraySet } from \"zun:buffer\"; function read(view: Uint8Array, index: u64): u8 { return u8ArrayGet(view, index); } function write(view: Uint8Array, index: u64, value: u8): boolean { return u8ArraySet(view, index, value); } function main(): u64 { return 0; }",
    );
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("u8 reader") as u32
        + 1;
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .expect("u8 writer") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(read_rid));
    assert!(jit.is_compiled(write_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(2);
    let view = heap.alloc_u8_array(buffer, 0, 2).expect("u8 view");
    assert!(matches!(jit.call_packed_with_heap(write_rid, &[view.addr() as u64, 1, 0xcd], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(read_rid, &[view.addr() as u64, 1], &mut heap), Ok(0xcd)));
}

#[test]
fn u16_array_access_uses_the_typed_view_abi() {
    let module = compile(
        "import { u16ArrayGet, u16ArraySet } from \"zun:buffer\"; function read(view: Uint16Array, index: u64): u16 { return u16ArrayGet(view, index); } function write(view: Uint16Array, index: u64, value: u16): boolean { return u16ArraySet(view, index, value); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == name).expect("u16 method") as u32 + 1;
    let read_rid = find("read");
    let write_rid = find("write");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(read_rid));
    assert!(jit.is_compiled(write_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(4);
    let view = heap.alloc_u16_array(buffer, 0, 4).expect("u16 view");
    let write_result = jit.call_packed_with_heap(write_rid, &[view.addr() as u64, 1, 0xbeef], &mut heap);
    assert!(matches!(write_result, Ok(1)), "write result: {write_result:?}");
    assert!(matches!(jit.call_packed_with_heap(read_rid, &[view.addr() as u64, 1], &mut heap), Ok(0xbeef)));
}

#[test]
fn u32_and_f64_array_access_use_typed_view_abis() {
    let module = compile(
        "import { u32ArrayGet, u32ArraySet, f64ArrayGet, f64ArraySet } from \"zun:buffer\"; function readU32(view: Uint32Array, index: u64): u32 { return u32ArrayGet(view, index); } function writeU32(view: Uint32Array, index: u64, value: u32): boolean { return u32ArraySet(view, index, value); } function readF64(view: Float64Array, index: u64): number { return f64ArrayGet(view, index); } function writeF64(view: Float64Array, index: u64, value: number): boolean { return f64ArraySet(view, index, value); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == name).expect("typed array method") as u32 + 1;
    let read_u32 = find("readU32");
    let write_u32 = find("writeU32");
    let read_f64 = find("readF64");
    let write_f64 = find("writeF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(read_u32));
    assert!(jit.is_compiled(write_u32));
    assert!(jit.is_compiled(read_f64));
    assert!(jit.is_compiled(write_f64));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(32);
    let u32_view = heap.alloc_u32_array(buffer, 0, 8).expect("u32 view");
    let f64_view = heap.alloc_f64_array(buffer, 16, 16).expect("f64 view");
    assert!(matches!(jit.call_packed_with_heap(write_u32, &[u32_view.addr() as u64, 1, 0xdead_beef], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(write_f64, &[f64_view.addr() as u64, 1, 3.25f64.to_bits()], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(read_u32, &[u32_view.addr() as u64, 1], &mut heap), Ok(0xdead_beef)));
    assert!(matches!(jit.call_packed_with_heap(read_f64, &[f64_view.addr() as u64, 1], &mut heap), Ok(value) if f64::from_bits(value) == 3.25));
}

#[test]
fn regexp_methods_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): int { let pattern = new RegExp(\"zun-[0-9]+\"); if (pattern.isMatch(\"zun-42\")) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "RegExp requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_collection_constructors_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): u64 { let map = new Map(); map.set(7.0, 42.0); if (map.has(7.0)) { return map.size(); } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "managed constructors require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_promise_intrinsics_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { promiseNew, promiseResolve, promiseState } from \"zun:promise\"; function main(): int { let promise = promiseNew(); if (promiseState(promise) != 0) { return 1; } promiseResolve(promise, 42.0); if (promiseState(promise) == 1) { return 0; } return 2; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Promise intrinsics require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn managed_promise_class_methods_use_interpreter_fallback() {
    let module = compile(
        "function main(): int { let promise = new Promise(); if (promise.isReady()) { return 1; } if (promise.state() != 0) { return 2; } promise.resolve(42.0); if (promise.isReady()) { let value = promise.result(); return 0; } return 3; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Promise methods require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn async_function_without_await_returns_a_promise_through_tier0() {
    let module = compile(
        "async function immediate(): Promise { return new Promise(); } function main(): int { let promise = immediate(); return promise.state() as int; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "managed Promise requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn async_await_preserves_the_interpreter_fallback_boundary() {
    let module = compile(
        "async function main(): Promise { let p = new Promise(); await p; return p; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Await requires Tier 0 state-machine execution");
    let mut output = CaptureOutput::default();
    let mut session = zun_interp::ResumableSession::new(
        &module,
        entry,
        Vec::new(),
        &mut output,
        None,
        None,
    )
    .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        zun_interp::ResumeResult::Suspended
    ));
    let promise = session.awaited_promise().expect("awaited promise");
    let boxed = Value::Int(1, zun_il::NumKind::I32)
        .box_any(session.heap_mut())
        .expect("box result");
    assert_eq!(session.heap_mut().promise_resolve(promise, boxed), Some(true));
    assert!(matches!(
        session.poll().expect("resumed poll"),
        zun_interp::ResumeResult::Complete(Value::Object(_))
    ));
}

#[test]
fn map_and_set_values_are_exposed_as_instance_methods() {
    let module = compile(
        "function main(): int { let map = new Map(); map.set(7.0, 42.0); let set = new Set(); set.add(9.0); let map_values: Array = map.values(); let set_values: Array = set.values(); if ((map_values[0] as number) == 42.0) { if ((set_values[0] as number) == 9.0) { return 1; } } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn map_keys_and_entries_are_exposed_as_instance_methods() {
    let module = compile(
        "function main(): int { let map = new Map(); map.set(7.0, 42.0); let keys: Array = map.keys(); let entries: Array = map.entries(); let pair: Array = entries[0] as Array; if ((keys[0] as number) == 7.0) { if ((pair[1] as number) == 42.0) { return 1; } } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn set_entries_are_exposed_as_instance_methods() {
    let module = compile(
        "function main(): int { let set = new Set(); set.add(9.0); let entries: Array = set.entries(); let pair: Array = entries[0] as Array; if ((pair[0] as number) == 9.0) { if ((pair[1] as number) == 9.0) { return 1; } } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn set_keys_alias_values() {
    let module = compile(
        "function main(): int { let set = new Set(); set.add(9.0); let keys: Array = set.keys(); if ((keys[0] as number) == 9.0) { return 1; } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn parenthesized_function_values_use_indirect_call_equivalence() {
    let module = compile(
        "function add(value: int): int { return value + 1; } function main(): int { let callback = add; return (callback)(7); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn parenthesized_dynamic_function_values_use_indirect_call_equivalence() {
    let module = compile(
        "function add(value: number): number { return value + 1.0; } function main(): int { let callback: Function = add; return ((callback)(7.0) as int); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn parenthesized_rest_function_values_pack_arguments() {
    let module = compile(
        "function count(first: int, ...rest: Array): int { return first + (rest[0] as int); } function main(): int { let callback = count; return ((callback)(7, 1, 2) as int); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn labeled_loop_control_is_interpreter_jit_equivalent() {
    assert_equivalent(
        "function main(): int { let total: int = 0; outer: for (let i: int = 0; i < 4; i = i + 1) { for (let j: int = 0; j < 4; j = j + 1) { if (j == 2) { continue outer; } total = total + 1; } } return total; }",
    );
}

#[test]
fn labeled_break_exits_the_named_loop() {
    let module = compile(
        "function main(): int { let total: int = 0; outer: for (let i: int = 0; i < 4; i = i + 1) { for (let j: int = 0; j < 4; j = j + 1) { total = total + 1; if (i == 1) { if (j == 1) { break outer; } } } } return total; }",
    );
    assert!(matches!(interp_entry(&module), Value::Int(6, _)));
}

#[test]
fn virtual_rest_methods_match_interpreter_and_jit_fallback() {
    let module = compile(
        "class Counter { count(first: int, ...rest: Array): int { return first + (rest[0] as int); } } function main(): int { let counter = new Counter(); return counter.count(7, 1, 2); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn virtual_rest_methods_accept_array_spread() {
    let module = compile(
        "class Counter { count(first: int, ...rest: Array): int { return first + (rest[0] as int); } } function main(): int { let counter = new Counter(); let values: Array = new Array(2); values[0] = 1; values[1] = 2; return counter.count(7, ...values); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn weak_map_class_methods_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "class Key {} function main(): int { let key = new Key(); let value = new Key(); let weak = new WeakMap(); weak.set(key, value); if (weak.has(key)) { let fetched: any = weak.get(key); if (weak.delete(key)) { if (!weak.has(key)) { return 1; } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "WeakMap requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn finalization_registry_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "function cleanup(value: any): void { } class Key {} function main(): boolean { let registry = new FinalizationRegistry(); let key = new Key(); let id = registry.register(key, cleanup); return registry.unregister(id); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "FinalizationRegistry requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn reflect_dynamic_keys_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): any { let object = dynamic_new(); let key = \"answer\"; reflect_set(object, key, box_any(41)); return reflect_get(object, key); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "dynamic property keys require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_construction_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return true; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn proxy_get_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { proxyGet } from \"zun:collections\"; function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(41)); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy get requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_set_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { proxySet, proxyGet } from \"zun:collections\"; function main(): any { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); return proxyGet(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy set requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_has_and_delete_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { proxyHas, proxySet, proxyDelete } from \"zun:collections\"; function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); return proxyHas(proxy, \"answer\") && proxyDelete(proxy, \"answer\") && !proxyHas(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy property traps require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn proxy_get_trap_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { dynamicSetFunction, proxyGet } from \"zun:collections\"; function getTrap(target: any, key: string): any { return box_any(99); } function main(): any { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"get\", getTrap); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy trap invocation requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(99.0)));
}

#[test]
fn proxy_property_traps_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { dynamicSetFunction, proxySet, proxyHas, proxyDelete } from \"zun:collections\"; function setTrap(target: any, key: string, value: any): boolean { return true; } function hasTrap(target: any, key: string): boolean { return true; } function deleteTrap(target: any, key: string): boolean { return true; } function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"set\", setTrap); dynamicSetFunction(handler, \"has\", hasTrap); dynamicSetFunction(handler, \"deleteProperty\", deleteTrap); let proxy = new Proxy(target, handler); return proxySet(proxy, \"answer\", box_any(41)) && proxyHas(proxy, \"answer\") && proxyDelete(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy property traps require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn map_and_set_for_of_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let map = new Map(); map.set(1.0, 10.0); let set = new Set(); set.add(2.0); for (let value of map) { print(value); } for (let value of set) { print(value); } return 42.0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "collection iteration requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn symbol_iterator_calls_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { arrayIteratorNext } from \"zun:buffer\"; function main(): number { let values = new Array(1); values[0] = 42.0; let iterator = values[Symbol.iterator](); return arrayIteratorNext(iterator) as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Symbol.iterator requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn typed_view_instance_accessors_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { bufferNew } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(4); let values = new Uint8Array(buffer, 0, 4); values.set(1, 255); return values.get(1) as u64; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "typed views require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(255, _)));
}

#[test]
fn switch_lowering_is_jit_equivalent() {
    let module = compile(
        "function main(): u64 { let value: u64 = 2; switch (value) { case 1: return 10; case 2: return 20; default: return 30; } }",
    );
    let jit = zun_codegen::Jit::compile(&module);
    assert!(
        jit.is_err(),
        "switch must remain on the interpreter fallback until verifier joins are lowered"
    );
    assert!(matches!(interp_entry(&module), Value::Int(20, _)));
}

#[test]
fn do_while_preserves_interpreter_fallback_boundary() {
    assert_equivalent(
        "function main(): int { let count: int = 0; do { count = count + 1; } while (false); return count; }",
    );
}

#[test]
fn c_style_for_is_jit_equivalent() {
    assert_equivalent(
        "function main(): int { let total: int = 0; for (let i: int = 0; i < 4; i = i + 1) { total = total + i; } return total; }",
    );
}

#[test]
fn short_circuit_logical_operators_are_jit_equivalent() {
    let module = compile("function main(): boolean { return true || false && false; }");
    assert!(
        zun_codegen::Jit::compile(&module).is_err(),
        "short-circuit joins remain on the interpreter fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn conditional_expression_is_jit_equivalent() {
    let module = compile(
        "function main(): number { let choose: boolean = false; return choose ? 1.0 : 2.0; }",
    );
    assert!(zun_codegen::Jit::compile(&module).is_err());
    assert!(matches!(interp_entry(&module), Value::Float(2.0, _)));
}

#[test]
fn null_literal_uses_the_interpreter_fallback_boundary() {
    let module = compile("function main(): any { return null; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "null currently requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Null));
}

#[test]
fn nullish_coalescing_uses_the_interpreter_fallback_boundary() {
    let module =
        compile("function main(): number { let value: any = null; return value ?? 42.0; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "nullish coalescing requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn optional_dynamic_access_uses_the_interpreter_fallback_boundary() {
    let module = compile("function main(): any { let value: any = null; return value?.answer; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "optional access requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Null));
}

#[test]
fn optional_managed_method_uses_the_interpreter_fallback_boundary() {
    let module = compile("function main(): u64 { let value = new Map(); return value?.size(); }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "optional managed method requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn default_parameter_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function add(value: number = 2.0): number { return value; } function main(): number { return add(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "default parameter requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(2.0, _)));
}

#[test]
fn rest_parameter_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(1, 2, 3); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "rest calls require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn spread_array_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(...new Array(3)); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "spread calls require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn fixed_spread_array_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function count(prefix: u64, ...values: Array): u64 { return values.length; } function main(): u64 { return count(7, ...new Array(3)); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "fixed spread calls require Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn array_destructuring_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let values = new Array(2); values[0] = 4; values[1] = 8; let [first, second] = values; return first as number + second as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "destructuring requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Float(12.0, _)));
}

#[test]
fn object_destructuring_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); let {answer} = object; return answer as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "object destructuring requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Float(41.0, _)));
}

#[test]
fn method_default_parameter_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "class Box { value(factor: number = 3.0): number { return factor; } } function main(): number { let box_value = new Box(); return box_value.value(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "default method parameter requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(3.0, _)));
}

#[test]
fn f64_arithmetic_and_recursion() {
    assert_equivalent(
        "function fib(n: number): number {\n\
            if (n < 2) { return n; }\n\
            return fib(n - 1) + fib(n - 2);\n\
         }\n\
         function main(): number { return fib(15); }",
    );
}

#[test]
fn int_arithmetic() {
    assert_equivalent(
        "function main(): int { let a: int = 10; let b: int = 3; return a * b + a / b; }",
    );
}

#[test]
fn managed_error_hierarchy_constructs_and_preserves_fields() {
    let module = compile(
        "class Error { name: string; message: string; }\n\
         class TypeError extends Error {}\n\
         function main(): int { let error: TypeError = new TypeError(\"TypeError\", \"bad value\"); if (error.name == \"TypeError\") { if (error.message == \"bad value\") { return 1; } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "string-field error objects stay interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_error_to_string_uses_name_and_message() {
    let module = compile(
        "class Error { name: string; message: string; toString(): string { return this.name + \": \" + this.message; } }\n\
         class RangeError extends Error {}\n\
         function main(): int { let error: RangeError = new RangeError(\"RangeError\", \"outside\"); let text: string = error.toString(); if (text == \"RangeError: outside\") { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "string operations stay interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn virtual_override_dispatch_is_jit_equivalent() {
    let module = compile(
        "class Base { value(): int { return 1; } }\n\
         class Derived extends Base { value(): int { return 2; } }\n\
         function main(): int { let item: Base = new Derived(); return item.value(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles virtual override");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("run JIT virtual override")
    };
    assert!(matches!(jit_result, RetVal::Int(2)));
    assert!(matches!(interp_entry(&module), Value::Int(2, _)));
}

#[test]
fn date_surface_decomposes_utc_timestamps() {
    let module = compile(
        "import { year, month, day } from \"zun:date\";\n\
         class Date { millis: i64; utcYear(): i32 { return year(this.millis); } utcMonth(): i32 { return month(this.millis); } utcDay(): i32 { return day(this.millis); } }\n\
         function main(): int { let value: Date = new Date(0); if (value.utcYear() == 1970) { if (value.utcMonth() == 1) { if (value.utcDay() == 1) { return 1; } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_validity_is_jit_equivalent() {
    assert_equivalent(
        "import { isValid } from \"zun:date\"; function main(): boolean { return isValid(0); }",
    );
}

#[test]
fn date_numeric_intrinsics_are_equivalent_in_jit() {
    let module = compile(
        "import { year, month, day, hour, minute, second, millisecond, weekday, dayOfYear } from \"zun:date\";\n\
         class Date { millis: i64; all(): int { return year(this.millis) + month(this.millis) + day(this.millis) + hour(this.millis) + minute(this.millis) + second(this.millis) + millisecond(this.millis) + weekday(this.millis) + dayOfYear(this.millis); } }\n\
         function main(): int { return new Date(3723004).all(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1987));
}

#[test]
fn date_numeric_intrinsics_handle_pre_epoch_timestamps() {
    let module = compile(
        "import { year, month, day, hour, minute, second, millisecond } from \"zun:date\";\n\
         class Date { millis: i64; signature(): int { return year(this.millis) + month(this.millis) + day(this.millis) + hour(this.millis) + minute(this.millis) + second(this.millis) + millisecond(this.millis); } }\n\
         function main(): int { return new Date(-1).signature(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1969 + 12 + 31 + 23 + 59 + 59 + 999));
}

#[test]
fn date_surface_exposes_time_components_and_string_form() {
    let module = compile(
        "import { hour, minute, second, millisecond, isoDateTime } from \"zun:date\";\n\
         class Date { millis: i64; utcHour(): i32 { return hour(this.millis); } utcMinute(): i32 { return minute(this.millis); } utcSecond(): i32 { return second(this.millis); } utcMillisecond(): i32 { return millisecond(this.millis); } isoDateTime(): string { return isoDateTime(this.millis); } toString(): string { return this.isoDateTime(); } }\n\
         function main(): int { let value: Date = new Date(3723004); if (value.utcHour() == 1) { if (value.utcMinute() == 2) { if (value.utcSecond() == 3) { if (value.utcMillisecond() == 4) { if (value.toString() == \"1970-01-01T01:02:03.004Z\") { return 1; } } } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "date string operations remain interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_updates_and_returns_timestamp_values() {
    let module = compile(
        "class Date { millis: i64; setMillis(value: i64): Date { this.millis = value; return this; } valueOf(): i64 { return this.millis; } }\n\
         function main(): int { let value: Date = new Date(0); let same: Date = value.setMillis(42); if (same.valueOf() == 42) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_supports_time_aliases_and_comparisons() {
    let module = compile(
        "class Date { millis: i64; getTime(): i64 { return this.millis; } setTime(value: i64): i64 { this.millis = value; return this.millis; } isBefore(other: Date): boolean { return this.millis < other.millis; } isAfter(other: Date): boolean { return this.millis > other.millis; } isEqual(other: Date): boolean { return this.millis == other.millis; } }\n\
         function main(): int { let first: Date = new Date(10); let second: Date = new Date(20); first.setTime(15); if (first.getTime() == 15) { if (first.isBefore(second)) { if (second.isAfter(first)) { if (!first.isEqual(second)) { return 1; } } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_exposes_ecmascript_utc_aliases() {
    let module = compile(
        "import { year, month, day, hour, minute, second, millisecond, isoDateTime } from \"zun:date\";\n\
         class Date { millis: i64; utcYear(): i32 { return year(this.millis); } utcMonth(): i32 { return month(this.millis); } utcDay(): i32 { return day(this.millis); } utcHour(): i32 { return hour(this.millis); } utcMinute(): i32 { return minute(this.millis); } utcSecond(): i32 { return second(this.millis); } utcMillisecond(): i32 { return millisecond(this.millis); } isoDateTime(): string { return isoDateTime(this.millis); } getUTCFullYear(): i32 { return this.utcYear(); } getUTCMonth(): i32 { return this.utcMonth(); } getUTCDate(): i32 { return this.utcDay(); } getUTCHours(): i32 { return this.utcHour(); } getUTCMinutes(): i32 { return this.utcMinute(); } getUTCSeconds(): i32 { return this.utcSecond(); } getUTCMilliseconds(): i32 { return this.utcMillisecond(); } toISOString(): string { return this.isoDateTime(); } }\n\
         function main(): int { let value: Date = new Date(3723004); if (value.getUTCFullYear() == 1970) { if (value.getUTCMonth() == 1) { if (value.getUTCDate() == 1) { if (value.getUTCHours() == 1) { if (value.getUTCMinutes() == 2) { if (value.getUTCSeconds() == 3) { if (value.getUTCMilliseconds() == 4) { if (value.toISOString() == \"1970-01-01T01:02:03.004Z\") { return 1; } } } } } } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "date string aliases remain interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_provides_timestamp_factories() {
    let module = compile(
        "\n\
         class Date { millis: i64; valueOf(): i64 { return this.millis; } }\n\
         function dateFromMillis(value: i64): Date { return new Date(value); }\n\
         function main(): int { let fixed: Date = dateFromMillis(42); if (fixed.valueOf() == 42) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_parses_iso_datetime_values() {
    let module = compile(
        "import { parseIsoDateTime } from \"zun:date\";\n\
         class Date { millis: i64; valueOf(): i64 { return this.millis; } }\n\
         function dateParse(value: string): Date { return new Date(parseIsoDateTime(value)); }\n\
         function main(): int { let value: Date = dateParse(\"1970-01-01T00:00:00.042Z\"); if (value.valueOf() == 42) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "ISO parsing remains interpreter-only until native JIT calls are lowered"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_json_representation_matches_iso() {
    let module = compile(
        "import { isoDateTime } from \"zun:date\";\n\
         class Date { millis: i64; isoDateTime(): string { return isoDateTime(this.millis); } toISOString(): string { return this.isoDateTime(); } toJSON(): string { return this.toISOString(); } }\n\
         function main(): int { let value: Date = new Date(42); if (value.toJSON() == \"1970-01-01T00:00:00.042Z\") { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Date JSON strings remain interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_exposes_utc_weekday_alias() {
    let module = compile(
        "import { weekday } from \"zun:date\";\n\
         class Date { millis: i64; utcWeekday(): i32 { return weekday(this.millis); } getUTCWeekday(): i32 { return this.utcWeekday(); } }\n\
         function main(): int { let value: Date = new Date(0); if (value.getUTCWeekday() == 4) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_converts_iso_weekday_to_ecmascript_day() {
    let module = compile(
        "import { weekday } from \"zun:date\";\n\
         class Date { millis: i64; utcWeekday(): i32 { return weekday(this.millis); } getUTCDay(): i32 { return this.utcWeekday(); } }\n\
         function main(): int { let value: Date = new Date(0); if (value.getUTCDay() == 4) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn int_recursion() {
    std::thread::Builder::new()
        .name("int-recursion-equivalence".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            assert_equivalent(
                "function sumTo(n: int): int { if (n == 0) { return 0; } return n + sumTo(n - 1); }\n\
                 function main(): int { return sumTo(100); }",
            );
        })
        .expect("spawn recursion test thread")
        .join()
        .expect("recursion test thread completed");
}

#[test]
fn extern_method_uses_interpreter_fallback_until_native_jit_lowering_exists() {
    let module = zun_compiler::compile(
        "import { receiverValue } from \"zun:math\";\n\
         class Service { extern receiverValue(value: number): number; }\n\
         function main(): number { let service = new Service(); return service.receiverValue(7.5); }",
    )
    .expect("compile extern method");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with fallback methods");
    assert!(
        !jit.is_compiled(entry),
        "native builtin call must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 7.5));
}

#[test]
fn string_primitive_methods_use_static_class_binding() {
    let module = zun_compiler::compile(
        "import { length, contains } from \"zun:string\";\n\
         class String { extern length(): u64; extern contains(needle: string): boolean; }\n\
         function main(): boolean { let value: string = \"zun\"; if (value.length() == 3) { return value.contains(\"u\"); } return false; }",
    )
    .expect("compile String methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(
        !jit.is_compiled(entry),
        "native String methods must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_includes_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { includes } from \"zun:string\";\n\
         class String { extern includes(needle: string): boolean; }\n\
         function main(): boolean { let value: string = \"zun runtime\"; return value.includes(\"runtime\"); }",
    )
    .expect("compile String includes");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_index_of_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { indexOf } from \"zun:string\";\n\
         class String { extern indexOf(needle: string): int; }\n\
         function main(): int { let value: string = \"😀zun\"; return value.indexOf(\"zun\"); }",
    )
    .expect("compile String indexOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(
        interp_entry(&module),
        Value::Int(2, zun_il::NumKind::I32)
    ));
}

#[test]
fn string_last_index_of_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { lastIndexOf } from \"zun:string\";\n\
         class String { extern lastIndexOf(needle: string): int; }\n\
         function main(): int { let value: string = \"zunzun\"; return value.lastIndexOf(\"zun\"); }",
    )
    .expect("compile String lastIndexOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(
        interp_entry(&module),
        Value::Int(3, zun_il::NumKind::I32)
    ));
}

#[test]
fn string_replace_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { replace } from \"zun:string\";\n\
         class String { extern replace(needle: string, replacement: string): string; }\n\
         function main(): string { let value: string = \"zun vm\"; return value.replace(\"vm\", \"runtime\"); }",
    )
    .expect("compile String replace");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun runtime"));
}

#[test]
fn string_substring_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { substring } from \"zun:string\";\n\
         class String { extern substring(start: u64, end: u64): string; }\n\
         function main(): string { let value: string = \"😀zun\"; return value.substring(2, 5); }",
    )
    .expect("compile String substring");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn string_pad_start_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { padStart } from \"zun:string\";\n\
         class String { extern padStart(targetLength: u64, fill: string): string; }\n\
         function main(): string { let value: string = \"zun\"; return value.padStart(5, \"0\"); }",
    )
    .expect("compile String padStart");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "00zun"));
}

#[test]
fn string_pad_end_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { padEnd } from \"zun:string\";\n\
         class String { extern padEnd(targetLength: u64, fill: string): string; }\n\
         function main(): string { let value: string = \"zun\"; return value.padEnd(5, \"0\"); }",
    )
    .expect("compile String padEnd");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun00"));
}

#[test]
fn string_case_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toLowerCase, toUpperCase } from \"zun:string\";\n\
         class String { extern toLowerCase(): string; extern toUpperCase(): string; }\n\
         function main(): string { let value: string = \"ZuN\"; return value.toLowerCase() + value.toUpperCase(); }",
    )
    .expect("compile String case methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zunZUN"));
}

#[test]
fn string_trim_boundary_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { trimStart, trimEnd } from \"zun:string\";\n\
         class String { extern trimStart(): string; extern trimEnd(): string; }\n\
         function main(): string { let value: string = \"  zun  \"; return value.trimStart().trimEnd(); }",
    )
    .expect("compile String trim boundary methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn string_is_blank_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { trim, length } from \"zun:string\";\n\
         class String { extern trim(): string; extern length(): u64; isEmpty(): boolean { return this.length() == 0; } isBlank(): boolean { return this.trim().isEmpty(); } }\n\
         function main(): boolean { let value: string = \"   \"; return value.isBlank(); }",
    )
    .expect("compile String isBlank");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_not_empty_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length } from \"zun:string\";\n\
         class String { extern length(): u64; isNotEmpty(): boolean { return this.length() != 0; } }\n\
         function main(): boolean { let value: string = \"zun\"; return value.isNotEmpty(); }",
    )
    .expect("compile String isNotEmpty");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_reverse_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { reverse } from \"zun:string\";\n\
         class String { extern reverse(): string; }\n\
         function main(): string { let value: string = \"zun\"; return value.reverse(); }",
    )
    .expect("compile String reverse");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "nuz"));
}

#[test]
fn string_is_ascii_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAscii(): boolean { let index: u64 = 0; while (index < this.length()) { if (this.charCodeAt(index) > 127) { return false; } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"zun\"; return value.isAscii(); }",
    )
    .expect("compile String isAscii");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_count_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { count } from \"zun:string\";\n\
         class String { extern count(needle: string): int; }\n\
         function main(): int { let value: string = \"aaaa\"; return value.count(\"aa\"); }",
    )
    .expect("compile String count");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 2));
}

#[test]
fn string_is_numeric_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isNumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"12345\"; return value.isNumeric(); }",
    )
    .expect("compile String isNumeric");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_compare_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { compare } from \"zun:string\";\n\
         class String { extern compare(other: string): int; }\n\
         function main(): int { let value: string = \"abc\"; return value.compare(\"abd\"); }",
    )
    .expect("compile String compare");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == -1));
}

#[test]
fn string_is_whitespace_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, trim } from \"zun:string\";\n\
         class String { extern length(): u64; extern trim(): string; isEmpty(): boolean { return this.length() == 0; } isWhitespace(): boolean { if (this.isEmpty()) { return false; } return this.trim().isEmpty(); } }\n\
         function main(): boolean { let value: string = \"   \"; return value.isWhitespace(); }",
    )
    .expect("compile String isWhitespace");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_alpha_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlpha(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"ZunRuntime\"; return value.isAlpha(); }",
    )
    .expect("compile String isAlpha");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_alphanumeric_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlphanumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"Zun2026\"; return value.isAlphanumeric(); }",
    )
    .expect("compile String isAlphanumeric");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_case_predicates_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isLowerCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 65) { if (code <= 90) { return false; } if (code >= 97) { if (code <= 122) { hasLetter = true; } } } index = index + 1; } return hasLetter; } isUpperCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 97) { if (code <= 122) { return false; } } if (code >= 65) { if (code <= 90) { hasLetter = true; } } index = index + 1; } return hasLetter; } }\n\
         function main(): boolean { let lower: string = \"hello\"; let upper: string = \"WORLD\"; if (!lower.isLowerCase()) { return false; } return upper.isUpperCase(); }",
    )
    .expect("compile String case predicates");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_to_int_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toInt } from \"zun:string\";\n\
         class String { extern toInt(): int; }\n\
         function main(): int { let value: string = \" -42 \"; return value.toInt(); }",
    )
    .expect("compile String toInt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == -42));
}

#[test]
fn string_to_float_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toFloat } from \"zun:string\";\n\
         class String { extern toFloat(): number; }\n\
         function main(): number { let value: string = \" 3.5 \"; return value.toFloat(); }",
    )
    .expect("compile String toFloat");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.5));
}

#[test]
fn string_to_boolean_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toBoolean } from \"zun:string\";\n\
         class String { extern toBoolean(): boolean; }\n\
         function main(): boolean { let value: string = \" true \"; return value.toBoolean(); }",
    )
    .expect("compile String toBoolean");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_primitive_methods_use_static_class_binding() {
    let module = zun_compiler::compile(
        "import { toFixed } from \"zun:number\";\n\
         class Number { extern toFixed(digits: int): string; }\n\
         function main(): string { let value: number = 12.5; return value.toFixed(1); }",
    )
    .expect("compile Number methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "native Number methods must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn number_sign_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { sign(): number { if (this < 0.0) { return -1.0; } if (this > 0.0) { return 1.0; } return 0.0; } }\n\
         function main(): number { let value: number = -42.0; return value.sign(); }",
    )
    .expect("compile Number sign");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == -1.0));
}

#[test]
fn number_finiteness_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isNaN(): boolean { return this != this; } isFinite(): boolean { return this == this; } }\n\
         function main(): boolean { let value: number = 42.0; if (!value.isFinite()) { return false; } return !value.isNaN(); }",
    )
    .expect("compile Number finiteness methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_integer_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isInteger(): boolean { return (this as int) as number == this; } }\n\
         function main(): boolean { let value: number = 42.0; return value.isInteger(); }",
    )
    .expect("compile Number isInteger");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_parity_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isInteger(): boolean { return (this as int) as number == this; } isEven(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 == 0; } isOdd(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 != 0; } }\n\
         function main(): boolean { let value: number = 42.0; if (!value.isEven()) { return false; } return !value.isOdd(); }",
    )
    .expect("compile Number parity methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_to_int_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { toInt(): int { return this as int; } }\n\
         function main(): int { let value: number = 12.75; return value.toInt(); }",
    )
    .expect("compile Number toInt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 12));
}

#[test]
fn number_to_boolean_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { toBoolean(): boolean { return this != 0.0; } }\n\
         function main(): boolean { let value: number = 1.0; return value.toBoolean(); }",
    )
    .expect("compile Number toBoolean");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_to_string_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toString } from \"zun:number\";\n\
         class Number { extern toString(): string; }\n\
         function main(): string { let value: number = 12.5; return value.toString(); }",
    )
    .expect("compile Number toString");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn number_source_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } isZero(): boolean { return this == 0.0; } isNegative(): boolean { return this < 0.0; } }\n\
         function main(): number { let value: number = -7.5; if (value.isNegative()) { return value.abs(); } return 0.0; }",
    )
    .expect("compile Number source methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "Number method calls must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(7.5, zun_il::NumKind::F64)
    ));
}

#[test]
fn number_clamp_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } }\n\
         function main(): number { let value: number = 12.0; return value.clamp(0.0, 10.0); }",
    )
    .expect("compile Number clamp");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "Number method calls must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(10.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn number_min_and_max_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { min(other: number): number { if (this < other) { return this; } return other; } max(other: number): number { if (this > other) { return this; } return other; } }\n\
         function main(): number { let value: number = 7.0; return value.min(10.0) + value.max(3.0); }",
    )
    .expect("compile Number min max");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "Number method calls must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(14.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn string_source_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class String { concat(other: string): string { return this + other; } withSeparator(separator: string, other: string): string { return this + separator + other; } }\n\
         function main(): string { let value: string = \"zun\"; return value.withSeparator(\":\", \"vm\"); }",
    )
    .expect("compile String source methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(
        !jit.is_compiled(entry),
        "String method calls must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun:vm"));
}

#[test]
fn string_is_empty_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length } from \"zun:string\";\n\
         class String { extern length(): u64; isEmpty(): boolean { return this.length() == 0; } }\n\
         function main(): boolean { let value: string = \"\"; return value.isEmpty(); }",
    )
    .expect("compile String isEmpty");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_to_string_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class String { toString(): string { return this; } }\n\
         function main(): string { let value: string = \"zun\"; return value.toString(); }",
    )
    .expect("compile String toString");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn array_source_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array {\n\
             contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; }\n\
         }\n\
         function main(): boolean { let values = new Array(1); values[0] = 41; return values.contains(values[0]); }",
    )
    .expect("compile Array source method");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_higher_order_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { map(callback: Function): Array { let result = new Array(this.length); let index: u64 = 0; while (index < this.length) { result[index] = callback(this[index]); index = index + 1; } return result; } filter(callback: Predicate): Array { let result = new Array(0); let index: u64 = 0; while (index < this.length) { let value: any = this[index]; if (callback(value)) { result.push(value); } index = index + 1; } return result; } reduce(callback: Reducer, initial: any): any { let accumulator: any = initial; let index: u64 = 0; while (index < this.length) { accumulator = callback(accumulator, this[index]); index = index + 1; } return accumulator; } }\n\
         function add_one(value: any): any { return (value as number) + 1.0; }\n\
         function positive(value: any): boolean { return (value as number) > 0.0; }\n\
         function add(accumulator: any, value: any): any { return (accumulator as number) + (value as number); }\n\
         function main(): any { let values = new Array(2); values[0] = -1.0; values[1] = 41.0; let mapped = values.map(add_one); let filtered = mapped.filter(positive); return filtered.reduce(add, 0.0); }",
    )
    .expect("compile Array higher-order methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with callback fallback");
    assert!(
        !jit.is_compiled(entry),
        "indirect callback methods must remain Tier 0"
    );
    assert!(
        matches!(interp_entry(&module), Value::Float(value, zun_il::NumKind::F64) if value == 42.0)
    );
}

#[test]
fn array_count_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { count(value: any): u64 { let total: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { total = total + 1; } index = index + 1; } return total; } }\n\
         function main(): u64 { let values = new Array(4); values[0] = 7; values[1] = 3; values[2] = 7; values[3] = 7; let needle: any = values[0]; return values.count(needle); }",
    )
    .expect("compile Array count");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 3));
}

#[test]
fn array_clear_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { clear(): void { while (this.length > 0) { this.pop(); } } }\n\
         function main(): u64 { let values = new Array(2); values[0] = 7; values[1] = 3; values.clear(); return values.length; }",
    )
    .expect("compile Array clear");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 0));
}

#[test]
fn array_average_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { sum(): number { let total: number = 0.0; let index: u64 = 0; while (index < this.length) { total = total + (this[index] as number); index = index + 1; } return total; } average(): number { if (this.length == 0) { return 0.0; } return this.sum() / (this.length as number); } }\n\
         function main(): number { let values = new Array(2); values[0] = 2.0; values[1] = 6.0; return values.average(); }",
    )
    .expect("compile Array average");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 4.0));
}

#[test]
fn array_min_and_max_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { min(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value < result) { result = value; } index = index + 1; } return result; } max(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value > result) { result = value; } index = index + 1; } return result; } }\n\
         function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; return values.min() + values.max(); }",
    )
    .expect("compile Array min and max");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 11.0));
}

#[test]
fn array_sort_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { sort(): Array { let index: u64 = 0; while (index < this.length) { let smallest: u64 = index; let scan: u64 = index + 1; while (scan < this.length) { let candidate: number = this[scan] as number; let currentSmallest: number = this[smallest] as number; if (candidate < currentSmallest) { smallest = scan; } scan = scan + 1; } let temporary: any = this[index]; this[index] = this[smallest]; this[smallest] = temporary; index = index + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; values.sort(); return values[0] as number; }",
    )
    .expect("compile Array sort");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_find_index_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; } findIndex(value: any): int { return this.indexOf(value); } }\n\
         function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 15; let needle: any = values[1]; return values.findIndex(needle); }",
    )
    .expect("compile Array findIndex");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 1));
}

#[test]
fn array_last_index_of_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { lastIndexOf(value: any): int { let index: int = this.length as int; while (index > 0) { index = index - 1; if (this[index as u64] == value) { return index; } } return -1; } }\n\
         function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 8; let needle: any = values[1]; return values.lastIndexOf(needle); }",
    )
    .expect("compile Array lastIndexOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 2));
}

#[test]
fn array_swap_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { swap(first: u64, second: u64): Array { let temporary: any = this[first]; this[first] = this[second]; this[second] = temporary; return this; } }\n\
         function main(): number { let values = new Array(2); values[0] = 4.0; values[1] = 8.0; values.swap(0, 1); return values[0] as number; }",
    )
    .expect("compile Array swap");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 8.0));
}

#[test]
fn array_rotate_left_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { rotateLeft(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let first: any = this[0]; let index: u64 = 1; while (index < this.length) { this[index - 1] = this[index]; index = index + 1; } this[this.length - 1] = first; rotation = rotation + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateLeft(1); return values[0] as number; }",
    )
    .expect("compile Array rotateLeft");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_rotate_right_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { rotateRight(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let last: any = this[this.length - 1]; let index: u64 = this.length - 1; while (index > 0) { this[index] = this[index - 1]; index = index - 1; } this[0] = last; rotation = rotation + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateRight(1); return values[0] as number; }",
    )
    .expect("compile Array rotateRight");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_take_and_drop_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } take(count: u64): Array { let limit: u64 = count; if (limit > this.length) { limit = this.length; } return this.slice(0, limit); } drop(count: u64): Array { let start: u64 = count; if (start > this.length) { start = this.length; } return this.slice(start, this.length); } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let prefix = values.take(2); let suffix = values.drop(1); return prefix[1] as number + suffix[0] as number; }",
    )
    .expect("compile Array take/drop");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    let result = interp_entry(&module);
    assert!(matches!(result, Value::Float(value, _) if value == 40.0));
}

#[test]
fn array_remove_at_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let removed: any = values.removeAt(1); return (removed as number) + (values[1] as number); }",
    )
    .expect("compile Array removeAt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 50.0));
}

#[test]
fn array_insert_at_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { insertAt(index: u64, value: any): Array { let cursor: u64 = this.length; this.push(value); while (cursor > index) { this[cursor] = this[cursor - 1]; cursor = cursor - 1; } this[index] = value; return this; } } function main(): number { let values = new Array(2); values[0] = 10.0; values[1] = 30.0; values.insertAt(1, values[0]); return (values[1] as number) + (values[2] as number); }",
    )
    .expect("compile Array insertAt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 40.0));
}

#[test]
fn array_remove_first_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeFirst(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); return true; } index = index + 1; } return false; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let needle: any = values[1]; if (!values.removeFirst(needle)) { return -1.0; } return values[1] as number; }",
    )
    .expect("compile Array removeFirst");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 30.0));
}

#[test]
fn array_remove_all_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeAll(value: any): u64 { let removed: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); removed = removed + 1; } else { index = index + 1; } } return removed; } } function main(): number { let values = new Array(4); values[0] = 7.0; values[1] = 3.0; values[2] = 7.0; values[3] = 5.0; let needle: any = values[0]; return values.removeAll(needle) as number; }",
    )
    .expect("compile Array removeAll");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_equals_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { equals(other: Array): boolean { if (this.length != other.length) { return false; } let index: u64 = 0; while (index < this.length) { if (this[index] != other[index]) { return false; } index = index + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 4.0; left[1] = 8.0; let right = new Array(2); right[0] = 4.0; right[1] = 8.0; return left.equals(right); }",
    )
    .expect("compile Array equals");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_sorted_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isSorted(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current < previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 2.0; return values.isSorted(); }",
    )
    .expect("compile Array isSorted");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_strictly_increasing_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isStrictlyIncreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current <= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isStrictlyIncreasing(); }",
    )
    .expect("compile Array isStrictlyIncreasing");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_strictly_decreasing_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isStrictlyDecreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current >= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 1.0; return values.isStrictlyDecreasing(); }",
    )
    .expect("compile Array isStrictlyDecreasing");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_descending_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isDescending(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current > previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 2.0; return values.isDescending(); }",
    )
    .expect("compile Array isDescending");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_unique_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isUnique(): boolean { let outer: u64 = 0; while (outer < this.length) { let inner: u64 = outer + 1; while (inner < this.length) { if (this[outer] == this[inner]) { return false; } inner = inner + 1; } outer = outer + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isUnique(); }",
    )
    .expect("compile Array isUnique");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_transpose_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { transpose(): Array { if (this.length == 0) { return new Array(0); } let first: Array = this[0] as Array; let rows: u64 = this.length; let columns: u64 = first.length; let result = new Array(columns); let column: u64 = 0; while (column < columns) { let rowResult = new Array(rows); let row: u64 = 0; while (row < rows) { let source: Array = this[row] as Array; rowResult[row] = source[column]; row = row + 1; } result[column] = rowResult; column = column + 1; } return result; } } function main(): number { let matrix = new Array(2); let first = new Array(2); first[0] = 1.0; first[1] = 2.0; let second = new Array(2); second[0] = 3.0; second[1] = 4.0; matrix[0] = first; matrix[1] = second; let result = matrix.transpose(); let column = result[1] as Array; return column[0] as number; }",
    )
    .expect("compile Array transpose");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_chunk_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { chunk(size: u64): Array { if (size == 0) { return new Array(0); } let groups: u64 = this.length / size; if (this.length % size != 0) { groups = groups + 1; } let result = new Array(groups); let group: u64 = 0; let start: u64 = 0; while (group < groups) { let remaining: u64 = this.length - start; let count: u64 = size; if (remaining < count) { count = remaining; } let part = new Array(count); let index: u64 = 0; while (index < count) { part[index] = this[start + index]; index = index + 1; } result[group] = part; start = start + count; group = group + 1; } return result; } } function main(): number { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; values[4] = 5.0; let groups = values.chunk(2); let last = groups[2] as Array; return last[0] as number; }",
    )
    .expect("compile Array chunk");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 5.0));
}

#[test]
fn array_windowed_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { windowed(size: u64): Array { if (size == 0) { return new Array(0); } if (size > this.length) { return new Array(0); } let count: u64 = this.length - size + 1; let result = new Array(count); let start: u64 = 0; while (start < count) { let window = new Array(size); let index: u64 = 0; while (index < size) { window[index] = this[start + index]; index = index + 1; } result[start] = window; start = start + 1; } return result; } } function main(): number { let values = new Array(4); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; let windows = values.windowed(3); let first = windows[0] as Array; return first[0] as number; }",
    )
    .expect("compile Array windowed");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn array_product_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { product(): number { let total: number = 1.0; let index: u64 = 0; while (index < this.length) { total = total * (this[index] as number); index = index + 1; } return total; } }\n\
         function main(): number { let values = new Array(3); values[0] = 2.0; values[1] = 3.0; values[2] = 4.0; return values.product(); }",
    )
    .expect("compile Array product");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 24.0));
}

#[test]
fn string_is_palindrome_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { reverse } from \"zun:string\"; class String { extern reverse(): string; isPalindrome(): boolean { return this == this.reverse(); } } function main(): boolean { let value: string = \"level\"; return value.isPalindrome(); }",
    )
    .expect("compile String isPalindrome");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_hexadecimal_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexadecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 70) { if (code < 97) { return false; } if (code > 102) { return false; } } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"a1F0\"; return value.isHexadecimal(); }",
    )
    .expect("compile String isHexadecimal");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_hex_digit_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 70) { valid = true; } } if (code >= 97) { if (code <= 102) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"aF09\"; return value.isHexDigit(); }",
    )
    .expect("compile String isHexDigit");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_binary_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isBinary(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 49) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"101001\"; return value.isBinary(); }",
    )
    .expect("compile String isBinary");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_octal_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isOctal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 55) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"701234\"; return value.isOctal(); }",
    )
    .expect("compile String isOctal");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_printable_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPrintable(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 32) { return false; } if (code > 126) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Zun 2026!\"; return value.isPrintable(); }",
    )
    .expect("compile String isPrintable");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_punctuation_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPunctuation(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let punctuation: boolean = false; if (code >= 33) { if (code <= 47) { punctuation = true; } } if (code >= 58) { if (code <= 64) { punctuation = true; } } if (code >= 91) { if (code <= 96) { punctuation = true; } } if (code >= 123) { if (code <= 126) { punctuation = true; } } if (!punctuation) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"!?.,\"; return value.isPunctuation(); }",
    )
    .expect("compile String isPunctuation");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_digit_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"2048\"; return value.isDigit(); }",
    )
    .expect("compile String isDigit");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_vowel_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isVowel(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let vowel: boolean = false; if (code == 65) { vowel = true; } if (code == 69) { vowel = true; } if (code == 73) { vowel = true; } if (code == 79) { vowel = true; } if (code == 85) { vowel = true; } if (code == 97) { vowel = true; } if (code == 101) { vowel = true; } if (code == 105) { vowel = true; } if (code == 111) { vowel = true; } if (code == 117) { vowel = true; } if (!vowel) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"AeIOu\"; return value.isVowel(); }",
    )
    .expect("compile String isVowel");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_consonant_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAlpha(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } isConsonant(): boolean { if (!this.isAlpha()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 65) { return false; } if (code == 69) { return false; } if (code == 73) { return false; } if (code == 79) { return false; } if (code == 85) { return false; } if (code == 97) { return false; } if (code == 101) { return false; } if (code == 105) { return false; } if (code == 111) { return false; } if (code == 117) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Rhythm\"; return value.isConsonant(); }",
    )
    .expect("compile String isConsonant");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_quoted_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isQuoted(): boolean { if (this.length() < 2) { return false; } let first: u32 = this.charCodeAt(0); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } } function main(): boolean { let value: string = \"\\\"zun\\\"\"; return value.isQuoted(); }",
    )
    .expect("compile String isQuoted");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_unquote_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt, substring } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; extern substring(start: u64, end: u64): string; isQuoted(): boolean { if (this.length() < 2) { return false; } let first: u32 = this.charCodeAt(0); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } unquote(): string { if (!this.isQuoted()) { return this; } return this.substring(1, this.length() - 1); } } function main(): string { let value: string = \"\\\"zun\\\"\"; return value.unquote(); }",
    )
    .expect("compile String unquote");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value == "zun".into()));
}

#[test]
fn string_is_wrapped_by_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { startsWith, endsWith } from \"zun:string\"; class String { extern startsWith(prefix: string): boolean; extern endsWith(suffix: string): boolean; isWrappedBy(prefix: string, suffix: string): boolean { if (!this.startsWith(prefix)) { return false; } return this.endsWith(suffix); } } function main(): boolean { let value: string = \"<zun>\"; return value.isWrappedBy(\"<\", \">\"); }",
    )
    .expect("compile String isWrappedBy");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_quote_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class String { quote(): string { return \"\\\"\" + this + \"\\\"\"; } } function main(): string { let value: string = \"zun\"; return value.quote(); }",
    )
    .expect("compile String quote");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value == "\"zun\"".into()));
}

#[test]
fn string_is_control_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isControl(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code > 31) { if (code != 127) { return false; } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"\\n\\t\"; return value.isControl(); }",
    )
    .expect("compile String isControl");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_signed_integer_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedInteger(): boolean { if (this.isEmpty()) { return false; } let start: u64 = 0; let first: u32 = this.charCodeAt(0); if (first == 43) { start = 1; } if (first == 45) { start = 1; } if (start == this.length()) { return false; } let index: u64 = start; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"-2048\"; return value.isSignedInteger(); }",
    )
    .expect("compile String isSignedInteger");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_signed_decimal_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedDecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let digits: u64 = 0; let dots: u64 = 0; let first: u32 = this.charCodeAt(0); if (first == 43) { index = 1; } if (first == 45) { index = 1; } while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 46) { dots = dots + 1; if (dots > 1) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } digits = digits + 1; } index = index + 1; } return digits > 0; } } function main(): boolean { let value: string = \"-12.50\"; return value.isSignedDecimal(); }",
    )
    .expect("compile String isSignedDecimal");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_identifier_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isIdentifier(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code == 95) { valid = true; } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (index > 0) { if (code >= 48) { if (code <= 57) { valid = true; } } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zun_core2\"; return value.isIdentifier(); }",
    )
    .expect("compile String isIdentifier");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_title_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isTitleCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let expectUpper: boolean = true; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 32) { expectUpper = true; } else { if (expectUpper) { if (code < 65) { return false; } if (code > 90) { return false; } expectUpper = false; hasLetter = true; } else { if (code >= 65) { if (code <= 90) { return false; } } if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } if (!hasLetter) { return false; } return !expectUpper; } } function main(): boolean { let value: string = \"Hello World\"; return value.isTitleCase(); }",
    )
    .expect("compile String isTitleCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_snake_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSnakeCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 95) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun_runtime2\"; return value.isSnakeCase(); }",
    )
    .expect("compile String isSnakeCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_kebab_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isKebabCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 45) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun-runtime2\"; return value.isKebabCase(); }",
    )
    .expect("compile String isKebabCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_camel_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isCamelCase(): boolean { if (this.isEmpty()) { return false; } let first: u32 = this.charCodeAt(0); if (first < 97) { return false; } if (first > 122) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zunRuntime2\"; return value.isCamelCase(); }",
    )
    .expect("compile String isCamelCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_pascal_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPascalCase(): boolean { if (this.isEmpty()) { return false; } let first: u32 = this.charCodeAt(0); if (first < 65) { return false; } if (first > 90) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"ZunRuntime2\"; return value.isPascalCase(); }",
    )
    .expect("compile String isPascalCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_sign_predicates_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isPositive(): boolean { return this > 0.0; } isNonZero(): boolean { return this != 0.0; } } function main(): boolean { let value: number = 3.0; if (!value.isPositive()) { return false; } return value.isNonZero(); }",
    )
    .expect("compile Number sign predicates");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_between_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } } function main(): boolean { let value: number = 5.0; return value.isBetween(1.0, 10.0); }",
    )
    .expect("compile Number isBetween");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_in_unit_interval_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } isInUnitInterval(): boolean { return this.isBetween(0.0, 1.0); } } function main(): boolean { let value: number = 0.75; return value.isInUnitInterval(); }",
    )
    .expect("compile Number isInUnitInterval");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_between_exclusive_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isBetweenExclusive(low: number, high: number): boolean { if (this <= low) { return false; } return this < high; } } function main(): boolean { let value: number = 5.0; return value.isBetweenExclusive(0.0, 10.0); }",
    )
    .expect("compile Number isBetweenExclusive");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_distance_to_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } distanceTo(other: number): number { return (this - other).abs(); } } function main(): number { let value: number = 3.0; return value.distanceTo(10.5); }",
    )
    .expect("compile Number distanceTo");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 7.5));
}

#[test]
fn number_inverse_lerp_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } } function main(): number { let value: number = 25.0; return value.inverseLerp(0.0, 100.0); }",
    )
    .expect("compile Number inverseLerp");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.25));
}

#[test]
fn number_smooth_step_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } smoothStep(edge0: number, edge1: number): number { let amount: number = this.inverseLerp(edge0, edge1).clamp(0.0, 1.0); return amount * amount * (3.0 - 2.0 * amount); } } function main(): number { let value: number = 50.0; return value.smoothStep(0.0, 100.0); }",
    )
    .expect("compile Number smoothStep");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.5));
}

#[test]
fn number_is_approximately_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isApproximately(other: number, epsilon: number): boolean { let difference: number = this - other; if (difference < 0.0) { difference = -difference; } return difference <= epsilon; } } function main(): boolean { let value: number = 1.001; return value.isApproximately(1.0, 0.01); }",
    )
    .expect("compile Number isApproximately");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_lerp_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { lerp(target: number, amount: number): number { return this + (target - this) * amount; } } function main(): number { let value: number = 10.0; return value.lerp(20.0, 0.25); }",
    )
    .expect("compile Number lerp");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 12.5));
}

#[test]
fn number_remap_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { remap(fromLow: number, fromHigh: number, toLow: number, toHigh: number): number { if (fromLow == fromHigh) { return toLow; } let amount: number = (this - fromLow) / (fromHigh - fromLow); return toLow + (toHigh - toLow) * amount; } } function main(): number { let value: number = 5.0; return value.remap(0.0, 10.0, 100.0, 200.0); }",
    )
    .expect("compile Number remap");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 150.0));
}

#[test]
fn number_finiteness_and_safe_integer_predicates_are_equivalent() {
    assert_equivalent(
        "import { isFinite, isNaN } from \"zun:number\"; function main(): int { let value: number = 9007199254740991.0; if (!isFinite(value)) { return 0; } if (isNaN(value)) { return 0; } if ((value as int) as number != value) { return 0; } if (value < -9007199254740991.0) { return 0; } if (value > 9007199254740991.0) { return 0; } return 1; }",
    );
}

#[test]
fn number_negative_zero_predicate_is_jit_equivalent() {
    assert_equivalent(
        "import { isNegativeZero } from \"zun:number\"; function main(): boolean { return isNegativeZero(-0.0); }",
    );
}

#[test]
fn number_safe_integer_intrinsic_is_jit_equivalent() {
    assert_equivalent(
        "import { isSafeInteger } from \"zun:number\"; function main(): boolean { return isSafeInteger(9007199254740991.0); }",
    );
}

#[test]
fn math_power_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { powNumber } from \"zun:math\"; function main(): number { return powNumber(2.0, 3.0); }",
    )
    .expect("compile math power");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 8.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 8.0));
}

#[test]
fn math_trigonometry_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { sinNumber, cosNumber } from \"zun:math\"; function main(): number { return sinNumber(0.0) + cosNumber(0.0); }",
    )
    .expect("compile math trigonometry");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 1.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn math_exponential_and_logarithm_are_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { expNumber, logNumber } from \"zun:math\"; function main(): number { return expNumber(0.0) + logNumber(1.0); }",
    )
    .expect("compile math exponential and logarithm");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 1.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn math_tangent_and_atan2_are_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { tanNumber, atan2Number } from \"zun:math\"; function main(): number { return tanNumber(0.0) + atan2Number(0.0, 1.0); }",
    )
    .expect("compile math tangent and atan2");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 0.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.0));
}

#[test]
fn math_hypot_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { hypotNumber } from \"zun:math\"; function main(): number { return hypotNumber(3.0, 4.0); }",
    )
    .expect("compile math hypot");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 5.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 5.0));
}

#[test]
fn math_scalar_aliases_are_jit_equivalent() {
    let module = compile(
        "import { sqrtNumber, minNumber, maxNumber, floorNumber, ceilNumber, roundNumber } from \"zun:math\"; function main(): number { return sqrtNumber(9.0) + minNumber(2.0, 5.0) + maxNumber(2.0, 5.0) + floorNumber(2.9) + ceilNumber(2.1) + roundNumber(2.6); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with scalar math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT scalar math"), RetVal::Float(value) if value == 18.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 18.0));
}

#[test]
fn math_sign_preserves_zero_sign_in_jit() {
    assert_equivalent(
        "import { signNumber } from \"zun:math\"; function main(): number { return signNumber(-0.0); }",
    );
}

#[test]
fn math_trunc_is_jit_equivalent_for_both_signs() {
    assert_equivalent(
        "import { truncNumber } from \"zun:math\"; function main(): number { return truncNumber(-2.9) + truncNumber(2.9); }",
    );
}

#[test]
fn math_log_bases_are_jit_equivalent() {
    assert_equivalent(
        "import { log10Number, log2Number } from \"zun:math\"; function main(): number { return log10Number(100.0) + log2Number(8.0); }",
    );
}

#[test]
fn math_cbrt_is_jit_equivalent() {
    assert_equivalent(
        "import { cbrtNumber } from \"zun:math\"; function main(): number { return cbrtNumber(27.0); }",
    );
}

#[test]
fn math_expm1_and_log1p_are_jit_equivalent() {
    assert_equivalent("import { expm1Number, log1pNumber } from \"zun:math\"; function main(): number { return expm1Number(log1pNumber(2.0)); }");
}

#[test]
fn math_hyperbolic_functions_are_jit_equivalent() {
    assert_equivalent("import { sinhNumber, coshNumber, tanhNumber } from \"zun:math\"; function main(): number { return sinhNumber(1.0) + coshNumber(1.0) + tanhNumber(1.0); }");
}

#[test]
fn math_inverse_hyperbolic_functions_are_jit_equivalent() {
    assert_equivalent("import { asinhNumber, acoshNumber, atanhNumber } from \"zun:math\"; function main(): number { return asinhNumber(1.0) + acoshNumber(2.0) + atanhNumber(0.5); }");
}

#[test]
fn break_keeps_interpreter_and_jit_paths_equivalent() {
    let module = zun_compiler::compile(
        "function main(): u64 { let index: u64 = 0; while (index < 10) { if (index == 3) { break; } index = index + 1; } return index; }",
    )
    .expect("compile break");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds break module");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 3));
    assert!(matches!(
        jit.call_entry(entry).expect("run JIT break"),
        RetVal::Int(3)
    ));
}

#[test]
fn continue_keeps_interpreter_and_jit_paths_equivalent() {
    let module = zun_compiler::compile(
        "function main(): u64 { let index: u64 = 0; let total: u64 = 0; while (index < 5) { index = index + 1; if (index == 3) { continue; } total = total + index; } return total; }",
    )
    .expect("compile continue");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds continue module");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 12));
    assert!(matches!(
        jit.call_entry(entry).expect("run JIT continue"),
        RetVal::Int(12)
    ));
}

#[test]
fn array_search_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array {\n\
             indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; }\n\
             isEmpty(): boolean { return this.length == 0; }\n\
         }\n\
         function main(): int { let values = new Array(1); values[0] = 41; if (values.isEmpty()) { return -2; } return values.indexOf(values[0]); }",
    )
    .expect("compile Array search methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Int(0, zun_il::NumKind::I32)
    ));
}

#[test]
fn array_access_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { at(index: u64): any { return this[index]; } first(): any { return this[0]; } last(): any { return this[this.length - 1]; } }\n\
         function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; return values.at(1) as number; }",
    )
    .expect("compile Array access methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(9.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_reverse_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { reverse(): Array { let left: u64 = 0; if (this.length == 0) { return this; } let right: u64 = this.length - 1; while (left < right) { let temporary: any = this[left]; this[left] = this[right]; this[right] = temporary; left = left + 1; right = right - 1; } return this; } first(): any { return this[0]; } }\n\
         function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; values.reverse(); return values.first() as number; }",
    )
    .expect("compile Array reverse");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(9.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_fill_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { fill(value: any): Array { let index: u64 = 0; while (index < this.length) { this[index] = value; index = index + 1; } return this; } first(): any { return this[0]; } }\n\
         function main(): number { let values = new Array(2); values[0] = 9.0; values.fill(values[0]); return values.first() as number; }",
    )
    .expect("compile Array fill");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(9.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_slice_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } first(): any { return this[0]; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; let selected = values.slice(1, 3); return selected.first() as number; }",
    )
    .expect("compile Array slice");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(7.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_concat_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { concat(other: Array): Array { let result = new Array(this.length + other.length); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } let otherIndex: u64 = 0; while (otherIndex < other.length) { result[index + otherIndex] = other[otherIndex]; otherIndex = otherIndex + 1; } return result; } first(): any { return this[0]; } }\n\
         function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let joined = left.concat(right); return joined[1] as number; }",
    )
    .expect("compile Array concat");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(9.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_copy_within_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } copyWithin(target: u64, start: u64, end: u64): Array { let copied = this.slice(start, end); let index: u64 = 0; while (index < copied.length) { this[target + index] = copied[index]; index = index + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; values.copyWithin(0, 1, 3); return values[0] as number; }",
    )
    .expect("compile Array copyWithin");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(7.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_flat_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { flat(): Array { let total: u64 = 0; let outer: u64 = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; total = total + nested.length; outer = outer + 1; } let result = new Array(total); let output: u64 = 0; outer = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; let inner: u64 = 0; while (inner < nested.length) { result[output] = nested[inner]; output = output + 1; inner = inner + 1; } outer = outer + 1; } return result; } }\n\
         function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let outer = new Array(2); outer[0] = left; outer[1] = right; let flattened = outer.flat(); return flattened[1] as number; }",
    )
    .expect("compile Array flat");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(9.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_includes_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } includes(value: any): boolean { return this.contains(value); } }\n\
         function main(): boolean { let values = new Array(1); values[0] = 41; return values.includes(values[0]); }",
    )
    .expect("compile Array includes");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_all_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { all(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] != value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 4.0; values[1] = 4.0; values[2] = 4.0; let needle: any = values[0]; return values.all(needle); }",
    )
    .expect("compile Array all");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_none_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { none(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[0]; return values.none(needle); }",
    )
    .expect("compile Array none");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(false)));
}

#[test]
fn array_any_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { any(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[1]; return values.any(needle); }",
    )
    .expect("compile Array any");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_constant_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isConstant(): boolean { if (this.length < 2) { return true; } let first: any = this[0]; let index: u64 = 1; while (index < this.length) { if (this[index] != first) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 9.0; values[1] = 9.0; values[2] = 9.0; return values.isConstant(); }",
    )
    .expect("compile Array isConstant");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_palindrome_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isPalindrome(): boolean { if (this.length < 2) { return true; } let left: u64 = 0; let right: u64 = this.length - 1; while (left < right) { if (this[left] != this[right]) { return false; } left = left + 1; right = right - 1; } return true; } } function main(): boolean { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 2.0; values[4] = 1.0; return values.isPalindrome(); }",
    )
    .expect("compile Array isPalindrome");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_disjoint_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isDisjoint(other: Array): boolean { let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { return false; } right = right + 1; } left = left + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 3.0; right[1] = 4.0; return left.isDisjoint(right); }",
    )
    .expect("compile Array isDisjoint");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_intersection_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { intersection(other: Array): Array { let matches: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { matches = matches + 1; break; } right = right + 1; } left = left + 1; } let result = new Array(matches); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { result[output] = this[left]; output = output + 1; break; } right = right + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let common = left.intersection(right); return common[0] as number; }",
    )
    .expect("compile Array intersection");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_difference_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let remaining = left.difference(right); return remaining[1] as number; }",
    )
    .expect("compile Array difference");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_union_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let combined = left.union(right); return combined[2] as number; }",
    )
    .expect("compile Array union");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_symmetric_difference_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { if (!other.contains(this[left])) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { if (!other.contains(this[left])) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } symmetricDifference(other: Array): Array { let leftOnly = this.difference(other); let rightOnly = other.difference(this); return leftOnly.union(rightOnly); } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let result = left.symmetricDifference(right); return result[1] as number; }",
    )
    .expect("compile Array symmetricDifference");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_is_subset_of_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return subset.isSubsetOf(full); }",
    )
    .expect("compile Array isSubsetOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_superset_of_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } isSupersetOf(other: Array): boolean { return other.isSubsetOf(this); } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return full.isSupersetOf(subset); }",
    )
    .expect("compile Array isSupersetOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_operations_use_interpreter_fallback_until_array_jit_lowering_exists() {
    let module = zun_compiler::compile(
        "function main(): any { let values = new Array(1); values[0] = 41.0; return values[0]; }",
    )
    .expect("compile array program");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with array fallback");
    assert!(
        !jit.is_compiled(entry),
        "array instructions must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn managed_array_length_is_jit_eligible_while_allocation_stays_tier0() {
    let module = compile(
        "function arrayLength(values: Array): u64 { return values.length; } function main(): u64 { return 7; }",
    );
    let length = module
        .tables
        .method_defs
        .iter()
        .enumerate()
        .find(|(_, method)| module.strings.get(method.name) == "arrayLength")
        .expect("arrayLength method");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles array length");

    assert!(jit.is_compiled(length.0 as u32 + 1));
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(7, _)));
    assert!(matches!(jit.call_entry(entry), Ok(RetVal::Int(7))));
}

#[test]
fn unsigned_wrap_u8() {
    assert_equivalent("function main(): u8 { let b: u8 = 250; return b + 10 as u8; }");
}

#[test]
fn signed_overflow_i8() {
    assert_equivalent("function main(): i8 { let s: i8 = 127; return s + 1 as i8; }");
}

#[test]
fn unsigned_division_u32() {
    assert_equivalent(
        "function main(): u32 { let x: u32 = 4000000000 as u32; return x / 2 as u32; }",
    );
}

#[test]
fn unsigned_comparison() {
    // 4e9 as u32 is > 1 unsigned but would be negative as i32; the JIT must use
    // an unsigned compare to match the interpreter.
    assert_equivalent(
        "function main(): int {\n\
            let x: u32 = 4000000000 as u32;\n\
            if (x > 1 as u32) { return 1; }\n\
            return 0;\n\
         }",
    );
}

#[test]
fn f32_precision() {
    assert_equivalent("function main(): f32 { let x: f32 = 1.0 as f32; return x / 3.0 as f32; }");
}

#[test]
fn int_to_float_conversion() {
    assert_equivalent("function main(): number { let n: int = 7; return n as number / 2.0; }");
}

#[test]
fn i64_arithmetic() {
    assert_equivalent("function main(): i64 { let x: i64 = 1000000 as i64; return x * x; }");
}

#[test]
fn boolean_return() {
    assert_equivalent("function main(): boolean { let a: int = 3; return a < 5; }");
}

#[test]
fn named_function_values_remain_on_interpreter_fallback() {
    let src = "function add_one(value: number): number { return value + 1.0; }\n\
               function main(): number { let f = add_one; return f(41.0); }";
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with fallback");
    assert!(
        !jit.is_compiled(entry),
        "indirect calls require Tier 0 until JIT lowering exists"
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 42.0));
}

#[test]
fn object_alloc_and_field_access_in_jit() {
    // Helpers that build a Point, pass it through a JIT call, and read/write
    // fields. They use object references in parameters and return values, so
    // the precise stack maps must retain those references at call safepoints.
    let src = "class Point { x: int; y: int; }\n\
               function make(): Point { return new Point(3, 4); }\n\
               function observe(p: Point): int { return p.x; }\n\
               class Holder { point: Point; }\n\
               function hold(p: Point): Holder { return new Holder(p); }\n\
               function dist2(): int {\n\
                   let p: Point = make();\n\
                   p.x = p.x + 1;\n\
                   return observe(p) + p.y * p.y;\n\
               }\n\
               function main(): int { return dist2(); }";
    let module = compile(src);
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let dist2_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "dist2")
        .map(|index| index as u32 + 1)
        .expect("dist2 method");
    let make_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    assert!(jit.is_compiled(dist2_rid), "object method should be eligible");
    assert!(jit.is_compiled(make_rid), "object-returning method should be eligible");
    let hold_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "hold")
        .map(|index| index as u32 + 1)
        .expect("hold method");
    let hold_maps = jit.stack_maps(hold_rid).expect("nested object method stack maps");
    assert!(
        hold_maps.iter().any(|map| !map.reference_slots.is_empty()),
        "object constructor arguments must be represented across allocation"
    );
    let maps = jit.stack_maps(dist2_rid).expect("object method stack maps");
    assert!(
        maps.iter().any(|map| !map.reference_slots.is_empty()),
        "object local must be represented in a precise JIT stack map"
    );
    let map = maps
        .iter()
        .find(|map| map.pc != 0)
        .expect("finalized stack-map PC");
    assert_eq!(
        jit.stack_map_registry().lookup(map.pc).map(|entry| entry.pc),
        Some(map.pc)
    );

    // The JIT needs a current heap for newobj; the VM sets one per run, so a
    // standalone call must register one too.
    let mut heap = zun_gc::Heap::new();
    let jit_result = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let observe_rid = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == "observe")
            .map(|index| index as u32 + 1)
            .expect("observe method");
        let made = jit
            .call_packed(make_rid, &[])
            .expect("direct packed object-returning call");
        let made = unsafe { zun_gc::Obj::from_addr(made as *mut u64) };
        assert_eq!(made.field(0), 3);
        let point = heap.alloc(1, 2);
        point.set_field(0, 3);
        let observed = jit
            .call_packed(observe_rid, &[point.addr() as u64])
            .expect("direct packed object call");
        assert_eq!(observed, 3);
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    let matches = match (&jit_result, &interp_result) {
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        _ => false,
    };
    assert!(matches, "JIT {jit_result:?} vs interp {interp_result:?}");
}

#[test]
fn numeric_array_load_is_jit_equivalent() {
    let src = "function read(values: Array): number { return values[1] as number; }\n\
               function main(): number { let values = new Array(2); values[0] = box_any(7.0); values[1] = box_any(41.0); return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array load");
    assert!(jit.is_compiled(read_rid), "numeric array reader should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    heap.array_set(array, 0, zun_core::Any::float(7.0));
    heap.array_set(array, 1, zun_core::Any::float(41.0));
    let jit_value = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(read_rid, &[array.addr() as u64])
            .expect("jit array load")
    };
    assert_eq!(f64::from_bits(jit_value), 41.0);
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn numeric_array_store_is_jit_equivalent() {
    let src = "function write(values: Array, value: number): number { values[1] = value; return value; }\n\
               function main(): number { let values = new Array(2); return write(values, 41.0); }";
    let module = compile(src);
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array store");
    assert!(jit.is_compiled(write_rid), "numeric array writer should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    let jit_value = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(write_rid, &[array.addr() as u64, 41.0f64.to_bits()])
            .expect("jit array store")
    };
    assert_eq!(f64::from_bits(jit_value), 41.0);
    assert_eq!(heap.array_get(array, 1).and_then(|value| value.as_float()), Some(41.0));
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn u64_array_load_is_jit_equivalent() {
    let src = "function read(values: Array): u64 { return values[1] as u64; }\n\
               function main(): u64 { let values = new Array(2); values[1] = 41; return read(values); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles u64 array load");
    assert!(jit.is_compiled(rid), "u64 array reader should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    heap.array_set(array, 1, zun_core::Any::int(41));
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64], &mut heap)
        .expect("jit u64 array load");
    assert_eq!(value, 41);
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 41));
}

#[test]
fn u64_array_store_is_jit_equivalent() {
    let src = "function write(values: Array, value: u64): u64 { values[1] = value; return value; }\n\
               function main(): u64 { let values = new Array(2); return write(values, 41); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles u64 array store");
    assert!(jit.is_compiled(rid), "u64 array writer should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, 41], &mut heap)
        .expect("jit u64 array store");
    assert_eq!(value, 41);
    assert_eq!(heap.array_get(array, 1).and_then(|item| item.as_int()), Some(41));
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 41));
}

#[test]
fn numeric_array_pop_is_jit_equivalent() {
    let src = "function pop_value(values: Array): number { return values.pop() as number; }\n\
               function main(): number { let values = new Array(1); values[0] = box_any(41.0); return pop_value(values); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "pop_value")
        .map(|index| index as u32 + 1)
        .expect("pop method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array pop");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::float(41.0));
    let bits = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(rid, &[array.addr() as u64]).expect("jit array pop")
    };
    assert_eq!(f64::from_bits(bits), 41.0);
    assert_eq!(heap.array_len(array), Some(0));
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn numeric_array_push_is_jit_equivalent() {
    let src = "function push_value(values: Array, value: number): number { return values.push(value) as number; }\n\
               function main(): number { let values = new Array(0); return push_value(values, 41.0); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "push_value")
        .map(|index| index as u32 + 1)
        .expect("push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array push");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(0);
    let bits = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(rid, &[array.addr() as u64, 41.0f64.to_bits()])
            .expect("jit array push")
    };
    assert_eq!(f64::from_bits(bits), 1.0);
    assert_eq!(heap.array_get(array, 0).and_then(|value| value.as_float()), Some(41.0));
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn object_array_push_and_pop_are_jit_equivalent() {
    let src = "class Item {} function pop_value(values: Array): Item { return values.pop() as Item; }\n\
               function push_value(values: Array, value: Item): u64 { return push(values, value) as u64; }\n\
               function push(values: Array, value: Item): u64 { return values.push(value) as u64; }\n\
               function main(): u64 { return 0; }";
    let module = compile(src);
    let pop_rid = module.tables.method_defs.iter()
        .position(|method| module.strings.get(method.name) == "pop_value")
        .map(|index| index as u32 + 1).expect("object pop method");
    let push_rid = module.tables.method_defs.iter()
        .position(|method| module.strings.get(method.name) == "push_value")
        .map(|index| index as u32 + 1).expect("object push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object array methods");
    assert!(!jit.is_compiled(pop_rid), "checked CastClass must remain Tier 0");
    assert!(jit.is_compiled(push_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(0);
    let item = heap.alloc(1, 0);
    assert_eq!(jit.call_packed_with_heap(push_rid, &[array.addr() as u64, item.addr() as u64], &mut heap).unwrap(), 1);
    assert_eq!(heap.array_len(array), Some(1));
}

#[test]
fn checked_string_array_unbox_keeps_interpreter_fallback() {
    let src = "function read(values: Array): string { return values[0] as string; }\n\
               function main(): string { let values = new Array(1); values[0] = box_any(\"ok\"); return read(values); }";
    let module = compile(src);
    let read_rid = module.tables.method_defs.iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1).expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles with fallback");
    assert!(!jit.is_compiled(read_rid), "checked UnboxAny must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(_)));
}

#[test]
fn proven_numeric_array_unbox_remains_jit_eligible() {
    let src = "function read(values: Array): u64 { return values[0] as u64; }\n\
               function main(): u64 { let values = new Array(1); values[0] = 37; return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric unbox");
    assert!(jit.is_compiled(read_rid), "proven numeric UnboxAny should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::int(37));
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit numeric unbox");
    assert_eq!(jit_value, 37);
    assert!(matches!(interp_entry(&module), Value::Int(37, _)));
}

#[test]
fn direct_any_array_load_preserves_the_two_word_return_abi() {
    let module = compile(
        "function read(values: Array): any { return values[0]; } function main(): u64 { return 0; }",
    );
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles Any return");
    assert!(jit.is_compiled(read_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::int(37));
    let carrier = jit
        .call_packed_any_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit Any return");
    assert_eq!(carrier.into_any(), Some(zun_core::Any::int(37)));

    let object = heap.alloc(1, 0);
    heap.array_set(array, 0, zun_core::Any::object(object.addr() as usize));
    let carrier = jit
        .call_packed_any_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit object Any return");
    assert_eq!(carrier.into_any(), Some(zun_core::Any::object(object.addr() as usize)));

    let string = heap.intern_string(std::rc::Rc::from("carrier"));
    heap.array_set(array, 0, zun_core::Any::string(string));
    let carrier = jit
        .call_packed_any_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit string Any return");
    assert_eq!(carrier.into_any(), Some(zun_core::Any::string(string)));
}

#[test]
fn direct_any_parameter_roundtrips_through_the_two_word_shim() {
    let module = compile(
        "function echo(value: any): any { return value; } function main(): u64 { return 0; }",
    );
    let echo_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "echo")
        .map(|index| index as u32 + 1)
        .expect("echo method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles Any parameter");
    assert!(jit.is_compiled(echo_rid));
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(91));
    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_any_with_heap(echo_rid, &[carrier.tag, carrier.payload], &mut heap)
        .expect("jit Any parameter");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(91)));
}

#[test]
fn direct_any_call_preserves_the_two_word_carrier() {
    let module = compile(
        "function echo(value: any): any { return value; }\n\
         function forward(value: any): any { return echo(value); }\n\
         function main(): u64 { return 0; }",
    );
    let forward_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "forward")
        .map(|index| index as u32 + 1)
        .expect("forward method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles direct Any call");
    assert!(jit.is_compiled(forward_rid));
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(123));
    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_any_with_heap(forward_rid, &[carrier.tag, carrier.payload], &mut heap)
        .expect("jit direct Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(123)));

    let object = heap.alloc(1, 0);
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::object(object.addr() as usize));
    let result = jit
        .call_packed_any_with_heap(forward_rid, &[carrier.tag, carrier.payload], &mut heap)
        .expect("jit direct managed Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::object(object.addr() as usize)));
}

#[test]
fn direct_any_call_expands_multiple_carriers_without_shifting_arguments() {
    let module = compile(
        "function choose(first: any, second: any): any { return second; }\n\
         function forward(first: any, second: any): any { return choose(first, second); }\n\
         function main(): u64 { return 0; }",
    );
    let forward_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "forward")
        .map(|index| index as u32 + 1)
        .expect("forward method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles multi-carrier call");
    assert!(jit.is_compiled(forward_rid));
    let first = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(1));
    let second = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(2));
    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_any_with_heap(
            forward_rid,
            &[first.tag, first.payload, second.tag, second.payload],
            &mut heap,
        )
        .expect("jit multi-carrier call");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(2)));
}

#[test]
fn direct_any_virtual_call_preserves_the_carrier() {
    let module = compile(
        "class Box { get(value: any): any { return value; } }\n\
         function forward(box: Box, value: any): any { return box.get(value); }\n\
         function main(): u64 { return 0; }",
    );
    let forward_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "forward")
        .map(|index| index as u32 + 1)
        .expect("forward method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles virtual Any call");
    assert!(jit.is_compiled(forward_rid));
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(321));
    let mut heap = zun_gc::Heap::new();
    let receiver = heap.alloc(1, 0);
    let result = jit
        .call_packed_any_with_heap(
            forward_rid,
            &[receiver.addr() as u64, carrier.tag, carrier.payload],
            &mut heap,
        )
        .expect("jit virtual Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(321)));

    let payload = heap.alloc(1, 0);
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::object(payload.addr() as usize));
    let result = jit
        .call_packed_any_with_heap(
            forward_rid,
            &[receiver.addr() as u64, carrier.tag, carrier.payload],
            &mut heap,
        )
        .expect("jit virtual managed Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::object(payload.addr() as usize)));
}

#[test]
fn generic_map_get_preserves_the_two_word_carrier() {
    let module = compile(
        "import { mapGet } from \"zun:collections\";\n\
         function read(map: Map, key: any): any { return mapGet(map, key); }\n\
         function main(): u64 { return 0; }",
    );
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles generic mapGet");
    assert!(jit.is_compiled(read_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(42));
    let key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(7));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, key.tag, key.payload], &mut heap)
        .expect("jit generic mapGet");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(42)));

    let value = heap.alloc(1, 0);
    heap.map_insert(map, zun_core::Any::int(8), zun_core::Any::object(value.addr() as usize));
    let key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(8));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, key.tag, key.payload], &mut heap)
        .expect("jit generic managed mapGet");
    assert_eq!(result.into_any(), Some(zun_core::Any::object(value.addr() as usize)));

    let missing_key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(99));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, missing_key.tag, missing_key.payload], &mut heap)
        .expect("jit generic mapGet missing key");
    assert_eq!(result.into_any(), Some(zun_core::Any::void()));
}

#[test]
fn proven_float_array_unbox_is_jit_equivalent() {
    let src = "function read(values: Array): number { return values[0] as number; }\n\
               function main(): number { let values = new Array(1); values[0] = 2.5; return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles float unbox");
    assert!(jit.is_compiled(read_rid), "proven float UnboxAny should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::float(2.5));
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit float unbox");
    assert_eq!(f64::from_bits(jit_value), 2.5);
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.5));
}

#[test]
fn proven_bool_array_unbox_is_jit_equivalent() {
    let src = "function read(values: Array): boolean { return values[0] as boolean; }\n\
               function main(): boolean { let values = new Array(1); values[0] = true; return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles bool unbox");
    assert!(jit.is_compiled(read_rid), "proven bool UnboxAny should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::bool(true));
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit bool unbox");
    assert_eq!(jit_value, 1);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn proven_bool_array_store_is_jit_equivalent() {
    let src = "function write(values: Array, value: boolean): boolean { values[0] = value; return values[0] as boolean; }\n\
               function main(): boolean { let values = new Array(1); return write(values, true); }";
    let module = compile(src);
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles bool store");
    assert!(jit.is_compiled(write_rid), "proven bool array store should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let jit_value = jit
        .call_packed_with_heap(write_rid, &[array.addr() as u64, 1], &mut heap)
        .expect("jit bool store");
    assert_eq!(jit_value, 1);
    assert_eq!(heap.array_get(array, 0).and_then(|value| value.as_bool()), Some(true));
}

#[test]
fn proven_bool_array_pop_and_push_are_jit_equivalent() {
    let src = "function pop_value(values: Array): boolean { return values.pop() as boolean; }\n\
               function push_value(values: Array, value: boolean): u64 { return values.push(value) as u64; }\n\
               function main(): boolean { return true; }";
    let module = compile(src);
    let pop_rid = module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == "pop_value").map(|index| index as u32 + 1).expect("pop method");
    let push_rid = module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == "push_value").map(|index| index as u32 + 1).expect("push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles bool stack methods");
    assert!(jit.is_compiled(pop_rid));
    assert!(jit.is_compiled(push_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::bool(true));
    assert_eq!(jit.call_packed_with_heap(pop_rid, &[array.addr() as u64], &mut heap).unwrap(), 1);
    assert_eq!(heap.array_len(array), Some(0));
    assert_eq!(jit.call_packed_with_heap(push_rid, &[array.addr() as u64, 1], &mut heap).unwrap(), 1);
    assert_eq!(heap.array_get(array, 0).and_then(|value| value.as_bool()), Some(true));
}

#[test]
fn numeric_array_allocation_is_jit_equivalent() {
    let src = "function make(length: u64): Array { return new Array(length); }\n\
               function main(): Array { return make(3); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array allocation");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[3], &mut heap)
        .expect("jit array allocation");
    let array = unsafe { zun_gc::Obj::from_addr(bits as *mut u64) };
    assert_eq!(heap.array_len(array), Some(3));
    assert!(matches!(interp_entry(&module), Value::Object(_)));
}

#[test]
fn jit_reads_length_from_an_existing_managed_array() {
    let src = "function length(values: Array): u64 { return values.length; }\n\
               function main(): u64 { let values = new Array(3); return length(values); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "length")
        .map(|index| index as u32 + 1)
        .expect("length method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles array length");
    assert!(jit.is_compiled(rid), "array length reader should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(3);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64], &mut heap)
        .expect("jit array length");
    assert_eq!(value, 3);
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 3));
}

#[test]
fn gc_preserves_and_reclaims_a_jit_allocated_object_by_root_contract() {
    let module = compile(
        "class Box { value: int; }
         function make(): Box { return new Box(7); }
         function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object allocation");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[], &mut heap)
        .expect("jit object allocation");
    let object = unsafe { zun_gc::Obj::from_addr(bits as *mut u64) };
    assert_eq!(object.field(0), 7);
    assert_eq!(unsafe { heap.collect(&[object], &|_| Vec::new()) }, 0);
    assert_eq!(object.field(0), 7);
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
}

#[test]
fn gc_traces_references_inside_a_jit_allocated_object() {
    let module = compile(
        "class Box { value: int; }
         class Holder { box: Box; }
         function make(): Holder { let value = new Box(7); return new Holder(value); }
         function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles nested allocation");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[], &mut heap)
        .expect("jit nested allocation");
    let holder = unsafe { zun_gc::Obj::from_addr(bits as *mut u64) };
    let box_object = unsafe { zun_gc::Obj::from_addr(holder.field(0) as *mut u64) };
    assert_eq!(box_object.field(0), 7);
    let holder_type = holder.type_rid();
    assert_eq!(
        unsafe { heap.collect(&[holder], &|type_rid| {
            (type_rid == holder_type).then_some(vec![0]).unwrap_or_default()
        }) },
        0
    );
    assert_eq!(box_object.field(0), 7);
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 2);
}

#[test]
fn generator_session_yields_and_resumes_in_tier0() {
    let module = zun_compiler::compile(
        "function* count(): int { yield 1; yield 2; return 3; }",
    )
    .expect("compile generator");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "count")
        .map(|index| index as u32 + 1)
        .expect("generator method");
    let mut output = CaptureOutput::default();
    let mut session = zun_interp::GeneratorSession::new(
        &module,
        rid,
        Vec::new(),
        &mut output,
    )
    .expect("create generator session");
    assert!(!session.is_done());
    assert!(matches!(session.next().expect("first generator next"), zun_interp::GeneratorResult::Yielded(Value::Float(value, _)) if value == 1.0));
    assert!(matches!(session.current(), Some(Value::Float(value, _)) if *value == 1.0));
    assert!(matches!(
        session.next().expect("second generator next"),
        zun_interp::GeneratorResult::Yielded(Value::Float(value, _)) if value == 2.0
    ));
    let complete = session.next().expect("generator completion");
    assert!(matches!(complete, zun_interp::GeneratorResult::Complete(Value::Int(value, _)) if value == 3), "{complete:?}");
    assert!(session.is_done());
    assert!(session.current().is_none());
    assert!(matches!(
        session.next().expect("sticky generator completion"),
        zun_interp::GeneratorResult::Complete(Value::Int(value, _)) if value == 3
    ));
}

#[test]
fn generator_session_implements_host_iterator_protocol() {
    let module = zun_compiler::compile(
        "function* count(): int { yield 4; yield 5; return 6; }",
    )
    .expect("compile generator");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "count")
        .map(|index| index as u32 + 1)
        .expect("generator method");
    let mut output = CaptureOutput::default();
    let session = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect("create generator session");
    let values: Vec<_> = session.collect();
    assert!(matches!(values.as_slice(), [
        Value::Float(first, _),
        Value::Float(second, _)
    ] if *first == 4.0 && *second == 5.0));
}

#[test]
fn generator_session_exposes_iterator_result_shape() {
    let module = zun_compiler::compile(
        "function* count(): int { yield 4; return 6; }",
    )
    .expect("compile generator");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "count")
        .map(|index| index as u32 + 1)
        .expect("generator method");
    let mut output = CaptureOutput::default();
    let mut session = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect("create generator session");

    let yielded = session.next_result().expect("yield result");
    assert!(!yielded.done);
    assert!(matches!(yielded.value, Value::Float(value, _) if value == 4.0));

    let completed = session.next_result().expect("completion result");
    assert!(completed.done);
    assert!(matches!(completed.value, Value::Int(value, _) if value == 6));

    let sticky = session.next_result().expect("sticky completion result");
    assert!(sticky.done);
    assert!(matches!(sticky.value, Value::Int(value, _) if value == 6));
}

#[test]
fn generator_session_rejects_a_regular_method() {
    let module = zun_compiler::compile("function plain(): int { return 1; }")
        .expect("compile regular method");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "plain")
        .map(|index| index as u32 + 1)
        .expect("regular method");
    let mut output = CaptureOutput::default();
    let error = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect_err("regular method must not create a generator session");
    assert!(matches!(error, zun_interp::RunError::Malformed(message) if message.contains("requires a method containing yield")));
}

#[test]
fn non_overridden_virtual_call_is_jit_eligible() {
    let src = "class Counter { n: int; get(): int { return this.n; } }\n\
               function read(c: Counter): int { return c.get(); }\n\
               function main(): int { return read(new Counter(7)); }";
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 7));
}

#[test]
fn aot_object_contains_the_eligible_entry_point() {
    let module = compile("function main(): int { return 7; }");
    let object = zun_codegen::compile_object(&module).expect("emit object");
    assert!(!object.is_empty());
    assert!(object.windows(2).any(|bytes| bytes == b"m1"));
}

#[test]
fn overridden_virtual_call_is_jit_eligible_and_dispatches_dynamically() {
    let src = "class Base { speak(): int { return 1; } }\n\
               class Derived extends Base { speak(): int { return 2; } }\n\
               function read(b: Base): int { return b.speak(); }\n\
               function main(): int { return read(new Derived()); }";
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 2));
}

#[test]
fn object_array_store_is_jit_eligible_and_preserves_pointer_abi() {
    let src = "function write(values: Array, value: Array): Array { values[0] = value; return values; }\n\
               function main(): boolean { return true; }";
    let module = compile(src);
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object array store");
    assert!(jit.is_compiled(write_rid), "object array writer should be eligible");

    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let value = heap.alloc_array(0);
    let result = jit
        .call_packed_with_heap(
            write_rid,
            &[array.addr() as u64, value.addr() as u64],
            &mut heap,
        )
        .expect("jit object array store");
    assert_eq!(result as usize, array.addr() as usize);
    assert_eq!(
        heap.array_get(array, 0).and_then(|any| any.as_object()),
        Some(value.addr() as usize)
    );
}

#[test]
fn boolean_object_field_is_jit_equivalent() {
    let src = "class Flag { value: boolean; }\n\
               function read(flag: Flag): boolean { return flag.value; }\n\
               function main(): boolean { return read(new Flag(true)); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles boolean field reader");
    assert!(jit.is_compiled(read_rid), "boolean field reader should be eligible");

    let mut heap = zun_gc::Heap::new();
    let flag = heap.alloc(1, 1);
    flag.set_field(0, 1);
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[flag.addr() as u64], &mut heap)
        .expect("jit boolean field read");
    assert_ne!(jit_value, 0);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_field_method_is_not_jit_eligible() {
    // A method touching a string field must stay on the interpreter.
    let src = "class Named { tag: string; }\n\
               function make(): int { let n: Named = new Named(\"hi\"); return 1; }\n\
               function main(): int { return make(); }";
    let module = compile(src);
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(2),
        "string-field method must be ineligible"
    );
}

#[test]
fn ineligible_methods_are_not_compiled() {
    let module = compile(r#"function main() { print("hi"); }"#);
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
}

#[test]
fn exception_regions_fall_back_to_the_interpreter() {
    let module =
        compile("function main(): int { try { return 1; } finally { let cleanup: int = 2; } }");
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn nested_exception_control_flow_keeps_interpreter_result_under_jit_fallback() {
    let module = compile(
        "class Error {} function main(): int { try { try { throw new Error(); } finally { print(1); } } catch (outer: Error) { return 42; } }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(42, _)));
}

#[test]
fn template_literals_match_interpreter_and_jit_fallback() {
    let module = compile(
        "function main(): string { let name: string = \"Zun\"; return `hello ${name}!`; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "hello Zun!"));
}

#[test]
fn template_literals_convert_numbers_through_the_native_floor() {
    let module = compile(
        "function main(): string { let value: number = 12.5; return `value=${value}`; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "value=12.5"));
}

#[test]
fn template_literals_parse_expression_interpolations() {
    let module = compile(
        "function main(): string { let value: number = 1.0; return `sum=${value + 2.0}`; }",
    );
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "sum=3"));
}

