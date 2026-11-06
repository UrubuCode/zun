mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

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
        "import { receiverValue } from \"zun:core\";\n\
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
        "import { length, contains } from \"zun:core\";\n\
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
        "import { includes } from \"zun:core\";\n\
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
        "import { indexOf } from \"zun:core\";\n\
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
        "import { lastIndexOf } from \"zun:core\";\n\
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
        "import { replace } from \"zun:core\";\n\
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
        "import { substring } from \"zun:core\";\n\
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
        "import { padStart } from \"zun:core\";\n\
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
        "import { padEnd } from \"zun:core\";\n\
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
        "import { toLowerCase, toUpperCase } from \"zun:core\";\n\
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
        "import { trimStart, trimEnd } from \"zun:core\";\n\
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
        "import { trim, length } from \"zun:core\";\n\
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
        "import { length } from \"zun:core\";\n\
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
        "import { reverse } from \"zun:core\";\n\
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
fn string_receiver_loop_method_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:core\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAscii(): boolean { let index: u64 = 0; while (index < this.length()) { if (this.charCodeAt(index) > 127) { return false; } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"zun\"; return value.isAscii(); }",
    )
    .expect("compile String isAscii");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    let is_ascii = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "isAscii")
        .map(|index| index as u32 + 1)
        .expect("isAscii method");
    assert!(jit.is_compiled(is_ascii), "string-receiver method should be JIT-eligible");
    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun"));
    assert_eq!(jit.call_packed_with_heap(is_ascii, &[value as u64], &mut heap).expect("jit isAscii"), 1);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_includes_with_managed_parameters_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { includes } from \"zun:core\";\n\
         class String { extern includes(needle: string): boolean; }\n\
         function contains(value: string, needle: string): boolean { return value.includes(needle); }\n\
         function main(): boolean { return true; }",
    )
    .expect("compile parameterized String includes");
    let contains = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "contains")
        .map(|index| index as u32 + 1)
        .expect("contains method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds parameterized String includes");
    assert!(jit.is_compiled(contains), "managed string parameters should use the direct JIT ABI");

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let needle = heap.intern_string(std::rc::Rc::from("runtime"));
    assert_eq!(
        jit.call_packed_with_heap(contains, &[value as u64, needle as u64], &mut heap)
            .expect("jit includes"),
        1
    );
}

#[test]
fn string_index_of_with_managed_parameters_preserves_utf16_indices() {
    let module = zun_compiler::compile(
        "import { indexOf } from \"zun:core\";\n\
         class String { extern indexOf(needle: string): int; }\n\
         function locate(value: string, needle: string): int { return value.indexOf(needle); }\n\
         function main(): int { return 0; }",
    )
    .expect("compile parameterized String indexOf");
    let locate = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "locate")
        .map(|index| index as u32 + 1)
        .expect("locate method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds parameterized String indexOf");
    assert!(jit.is_compiled(locate));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("😀zun"));
    let needle = heap.intern_string(std::rc::Rc::from("zun"));
    assert_eq!(
        jit.call_packed_with_heap(locate, &[value as u64, needle as u64], &mut heap)
            .expect("jit indexOf"),
        2
    );
}

#[test]
fn string_prefix_helpers_with_managed_parameters_are_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { startsWith } from \"zun:core\";\n\
         class String { extern startsWith(needle: string): boolean; }\n\
         function check(value: string, prefix: string): boolean { return value.startsWith(prefix); }\n\
         function main(): boolean { return true; }",
    )
    .expect("compile parameterized String prefix helpers");
    let check = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "check")
        .map(|index| index as u32 + 1)
        .expect("check method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds String prefix helpers");
    assert!(jit.is_compiled(check));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let prefix = heap.intern_string(std::rc::Rc::from("zun"));
    assert_eq!(
        jit.call_packed_with_heap(check, &[value as u64, prefix as u64], &mut heap)
            .expect("jit prefix helpers"),
        1
    );
}

#[test]
fn string_suffix_helper_with_managed_parameters_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { endsWith } from \"zun:core\";\n\
         class String { extern endsWith(needle: string): boolean; }\n\
         function check(value: string, suffix: string): boolean { return value.endsWith(suffix); }\n\
         function main(): boolean { return true; }",
    )
    .expect("compile parameterized String suffix helper");
    let check = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "check")
        .map(|index| index as u32 + 1)
        .expect("check method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds String suffix helper");
    assert!(jit.is_compiled(check));

    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun runtime"));
    let suffix = heap.intern_string(std::rc::Rc::from("runtime"));
    assert_eq!(
        jit.call_packed_with_heap(check, &[value as u64, suffix as u64], &mut heap)
            .expect("jit suffix helper"),
        1
    );
}

#[test]
fn string_count_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { count } from \"zun:core\";\n\
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
        "import { length, charCodeAt } from \"zun:core\";\n\
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
        "import { compare } from \"zun:core\";\n\
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
        "import { length, trim } from \"zun:core\";\n\
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
        "import { length, charCodeAt } from \"zun:core\";\n\
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
        "import { length, charCodeAt } from \"zun:core\";\n\
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
        "import { length, charCodeAt } from \"zun:core\";\n\
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
        "import { toInt } from \"zun:core\";\n\
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
        "import { toFloat } from \"zun:core\";\n\
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
        "import { toBoolean } from \"zun:core\";\n\
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
fn typed_string_conversions_use_native_lowering_when_receiver_is_string() {
    let module = zun_compiler::compile(
        "import { toInt, toFloat, toBoolean } from \"zun:core\";\n\
         class String { extern toInt(): int; extern toFloat(): number; extern toBoolean(): boolean; }\n\
         function convertInt(value: string): int { return value.toInt(); }\n\
         function convertFloat(value: string): number { return value.toFloat(); }\n\
         function convertBoolean(value: string): boolean { return value.toBoolean(); }\n\
         function main(): int { return 0; }",
    )
    .expect("compile typed String conversions");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds String conversions");
    for name in ["convertInt", "convertFloat", "convertBoolean"] {
        let rid = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .map(|index| index as u32 + 1)
            .expect("conversion method");
        assert!(jit.is_compiled(rid), "{name} should use native lowering");
    }
    let convert_int = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "convertInt")
        .map(|index| index as u32 + 1)
        .expect("integer conversion method");
    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from(" -42 "));
    assert_eq!(
        jit.call_packed_with_heap(convert_int, &[value as u64], &mut heap)
            .unwrap(),
        (-42i64) as u64
    );
    let convert_float = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "convertFloat")
        .map(|index| index as u32 + 1)
        .expect("float conversion method");
    assert_eq!(
        f64::from_bits(
            jit.call_packed_with_heap(convert_float, &[value as u64], &mut heap)
                .unwrap()
        ),
        -42.0
    );
    let truth = heap.intern_string(std::rc::Rc::from(" true "));
    let convert_boolean = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "convertBoolean")
        .map(|index| index as u32 + 1)
        .expect("boolean conversion method");
    assert_eq!(
        jit.call_packed_with_heap(convert_boolean, &[truth as u64], &mut heap)
            .unwrap(),
        1
    );
}

