mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn f64_array_allocation_and_access_are_jit_equivalent() {
    let module = zun_compiler::compile(
        "function main(): number { let values = new Array(1); values[0] = 42.0; return values[0] as number; }",
    )
    .expect("compile generic Array access");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles f64 Array access");
    assert!(jit.is_compiled(entry), "typed f64 Array access should be eligible");
    assert!(matches!(
        interp_entry(&module),
        Value::Float(42.0, zun_il::NumKind::F64)
    ));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(
        f64::from_bits(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap()),
        42.0
    );
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
fn scalar_u64_array_allocation_and_access_are_jit_eligible() {
    let module = compile(
        "function main(): u64 { let values = new Array(1); values[0] = 42 as u64; return values[0] as u64; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles scalar array access");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(42, _)));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap(), 42);
}

#[test]
fn scalar_bool_array_allocation_and_access_are_jit_eligible() {
    let module = compile(
        "function main(): boolean { let values = new Array(1); values[0] = true; return values[0] as boolean; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles boolean array access");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap(), 1);
}

#[test]
fn f64_array_allocation_and_pop_are_jit_equivalent() {
    let module = compile(
        "function main(): number { let values = new Array(1); values[0] = 42.0; return values.pop() as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles f64 array pop");
    assert!(jit.is_compiled(entry));
    assert!(matches!(
        interp_entry(&module),
        Value::Float(42.0, zun_il::NumKind::F64)
    ));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(
        f64::from_bits(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap()),
        42.0
    );
}

#[test]
fn f64_array_allocation_and_push_are_jit_equivalent() {
    let module = compile(
        "function main(): u64 { let values = new Array(0); return values.push(42.0) as u64; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles f64 array push");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap(), 1);
}

#[test]
fn u64_array_allocation_and_push_are_jit_equivalent() {
    let module = compile(
        "function main(): u64 { let values = new Array(0); return values.push(42 as u64) as u64; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles u64 array push");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap(), 1);
}

#[test]
fn bool_array_allocation_and_push_are_jit_equivalent() {
    let module = compile(
        "function main(): u64 { let values = new Array(0); return values.push(true) as u64; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles boolean array push");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap(), 1);
}

#[test]
fn bool_array_allocation_and_pop_are_jit_equivalent() {
    let module = compile(
        "function main(): boolean { let values = new Array(1); values[0] = true; return values.pop() as boolean; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles boolean array pop");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let mut heap = zun_gc::Heap::new();
    assert_eq!(jit.call_packed_with_heap(entry, &[], &mut heap).unwrap(), 1);
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
            if (x > (1 as u32)) { return 1; }\n\
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

