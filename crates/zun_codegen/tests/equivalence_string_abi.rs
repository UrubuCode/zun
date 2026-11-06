mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn primitive_string_length_callvirt_uses_the_direct_handle_path() {
    let module = compile(
        "import { length } from \"zun:core\"; class String { extern length(): u64; } function measure(value: string): u64 { return value.length(); } function main(): u64 { return 0; }",
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
        "import { contains } from \"zun:core\"; class String { extern contains(needle: string): boolean; } function main(): boolean { return \"zun\".contains(\"u\"); }",
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
        "import { contains } from \"zun:core\"; class String { extern contains(needle: string): boolean; } function measure(value: string, needle: string): boolean { return value.contains(needle); } function main(): boolean { return false; }",
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
fn managed_string_result_remains_rooted_across_a_following_helper() {
    let module = compile(
        "import { concat, contains } from \"zun:core\"; class String { extern concat(other: string): string; extern contains(needle: string): boolean; } function measure(value: string, suffix: string, needle: string): boolean { return value.concat(suffix).contains(needle); } function main(): boolean { return false; }",
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
    let suffix = heap.intern_string(std::rc::Rc::from(" runtime"));
    let needle = heap.intern_string(std::rc::Rc::from("runtime"));
    assert!(matches!(
        jit.call_packed_with_heap(
            rid,
            &[value as u64, suffix as u64, needle as u64],
            &mut heap,
        ),
        Ok(1)
    ));
}

#[test]
fn primitive_string_char_code_at_uses_the_managed_string_abi() {
    let module = compile(
        "import { charCodeAt } from \"zun:core\"; class String { extern charCodeAt(index: u64): u32; } function measure(value: string, index: u64): u32 { return value.charCodeAt(index); } function main(): u32 { return 0; }",
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
        "import { startsWith } from \"zun:core\"; class String { extern startsWith(prefix: string): boolean; } function measure(value: string, prefix: string): boolean { return value.startsWith(prefix); } function main(): boolean { return false; }",
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
        "import { endsWith } from \"zun:core\"; class String { extern endsWith(suffix: string): boolean; } function measure(value: string, suffix: string): boolean { return value.endsWith(suffix); } function main(): boolean { return false; }",
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
        "import { indexOf } from \"zun:core\"; class String { extern indexOf(needle: string): int; } function measure(value: string, needle: string): int { return value.indexOf(needle); } function main(): int { return 0; }",
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
        "import { lastIndexOf } from \"zun:core\"; class String { extern lastIndexOf(needle: string): int; } function measure(value: string, needle: string): int { return value.lastIndexOf(needle); } function main(): int { return 0; }",
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
        "import { count } from \"zun:core\"; class String { extern count(needle: string): int; } function measure(value: string, needle: string): int { return value.count(needle); } function main(): int { return 0; }",
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
        "import { compare } from \"zun:core\"; class String { extern compare(other: string): int; } function measure(value: string, other: string): int { return value.compare(other); } function main(): int { return 0; }",
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
        "import { concat } from \"zun:core\"; class String { extern concat(other: string): string; } function measure(value: string, other: string): string { return value.concat(other); } function main(): int { return 0; }",
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
        "import { substring } from \"zun:core\"; class String { extern substring(start: u64, end: u64): string; } function measure(value: string): string { return value.substring(1, 4); } function main(): int { return 0; }",
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
        "import { trim } from \"zun:core\"; class String { extern trim(): string; } function measure(value: string): string { return value.trim(); } function main(): int { return 0; }",
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
        "import { trimStart, trimEnd } from \"zun:core\"; class String { extern trimStart(): string; extern trimEnd(): string; } function measure(value: string): string { return value.trimStart().trimEnd(); } function main(): int { return 0; }",
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
        "import { toLowerCase, toUpperCase } from \"zun:core\"; class String { extern toLowerCase(): string; extern toUpperCase(): string; } function lower(value: string): string { return value.toLowerCase(); } function upper(value: string): string { return value.toUpperCase(); } function main(): int { return 0; }",
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
        "import { repeat } from \"zun:core\"; class String { extern repeat(count: u64): string; } function measure(value: string): string { return value.repeat(3); } function main(): int { return 0; }",
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
        "import { reverse } from \"zun:core\"; class String { extern reverse(): string; } function measure(value: string): string { return value.reverse(); } function main(): int { return 0; }",
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
        "import { padStart, padEnd } from \"zun:core\"; class String { extern padStart(target: u64, fill: string): string; extern padEnd(target: u64, fill: string): string; } function left(value: string, fill: string): string { return value.padStart(5, fill); } function right(value: string, fill: string): string { return value.padEnd(5, fill); } function main(): int { return 0; }",
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
        "import { replace } from \"zun:core\"; class String { extern replace(needle: string, replacement: string): string; } function measure(value: string, needle: string, replacement: string): string { return value.replace(needle, replacement); } function main(): int { return 0; }",
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
        "import { toInt, toFloat, toBoolean } from \"zun:core\"; function asInt(value: string): int { return toInt(value); } function asFloat(value: string): number { return toFloat(value); } function asBool(value: string): boolean { return toBoolean(value); } function main(): int { return 0; }",
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
        "import { toInt, toFloat, toBoolean } from \"zun:core\"; class String { extern toInt(): int; extern toFloat(): number; extern toBoolean(): boolean; } function asInt(value: string): int { return value.toInt(); } function asFloat(value: string): number { return value.toFloat(); } function asBool(value: string): boolean { return value.toBoolean(); } function main(): int { return 0; }",
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

