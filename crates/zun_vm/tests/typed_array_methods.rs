//! The second-generation typed-array surface: bulk `set(source, offset)`,
//! copying `slice`, in-place `copyWithin`, the `from`/`of` statics, and the
//! functional methods. These route to kind-agnostic `typedArray*` natives on
//! the Tier-0 interpreter (callbacks re-enter it), so the element kind is a
//! runtime property of the view or a discriminant argument.

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn of_builds_a_view_from_its_arguments() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(10, 20, 30);
            print(a.length());
            print(a.get(0));
            print(a.get(2));
        }",
    );
    assert_eq!(out, vec!["3", "10", "30"]);
}

#[test]
fn from_copies_an_array_into_a_view() {
    let out = run_source(
        "function main() {
            let a = Float64Array.from([1.5, 2.5, 3.5]);
            print(a.length());
            print(a.get(1));
        }",
    );
    assert_eq!(out, vec!["3", "2.5"]);
}

#[test]
fn from_with_a_map_function_transforms_each_element() {
    let out = run_source(
        "function main() {
            let a = Int32Array.from([1, 2, 3], (v: any, i: any) => (v as number) * 10.0 + (i as number));
            print(a.get(0));
            print(a.get(1));
            print(a.get(2));
        }",
    );
    assert_eq!(out, vec!["10", "21", "32"]);
}

#[test]
fn from_accepts_another_typed_array() {
    let out = run_source(
        "function main() {
            let src = Uint8Array.of(7, 8, 9);
            let a = Int32Array.from(src);
            print(a.get(0));
            print(a.get(2));
        }",
    );
    assert_eq!(out, vec!["7", "9"]);
}

#[test]
fn bulk_set_copies_a_source_at_an_offset() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(32);
            let a = new Int32Array(b, 0, 8);
            let src = Int32Array.of(5, 6, 7);
            print(a.set(src, 2));
            print(a.get(2));
            print(a.get(3));
            print(a.get(4));
            print(a.get(0));
        }",
    );
    assert_eq!(out, vec!["true", "5", "6", "7", "0"]);
}

#[test]
fn bulk_set_from_a_plain_array() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(32);
            let a = new Int32Array(b, 0, 8);
            print(a.set([100, 200], 1));
            print(a.get(1));
            print(a.get(2));
        }",
    );
    assert_eq!(out, vec!["true", "100", "200"]);
}

#[test]
fn bulk_set_out_of_range_reports_false() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(32);
            let a = new Int32Array(b, 0, 8);
            let src = Int32Array.of(1, 2, 3);
            print(a.set(src, 7));
        }",
    );
    assert_eq!(out, vec!["false"]);
}

#[test]
fn indexed_set_still_works() {
    // The overload must not have broken the indexed `set(index, value)`.
    let out = run_source(
        "function main() {
            let a = Int32Array.of(0, 0, 0);
            a.set(1, 42);
            print(a.get(1));
        }",
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn slice_clamps_and_handles_negative_indices() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(10, 20, 30, 40, 50);
            let tail = a.slice(-2);
            print(tail.length());
            print(tail.get(0));
            print(tail.get(1));
            let mid = a.slice(1, 3);
            print(mid.length());
            print(mid.get(0));
            print(mid.get(1));
        }",
    );
    assert_eq!(out, vec!["2", "40", "50", "2", "20", "30"]);
}

#[test]
fn slice_is_a_copy_not_a_view() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3);
            let s = a.slice(0, 2);
            a.set(0, 99);
            print(s.get(0));
        }",
    );
    assert_eq!(out, vec!["1"]);
}

#[test]
fn copy_within_moves_a_range_in_place() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3, 4, 5);
            a.copyWithin(0, 3);
            print(a.get(0));
            print(a.get(1));
            print(a.get(2));
            print(a.get(3));
            print(a.get(4));
        }",
    );
    assert_eq!(out, vec!["4", "5", "3", "4", "5"]);
}

#[test]
fn copy_within_with_negative_bounds() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3, 4, 5);
            a.copyWithin(1, -2);
            print(a.get(0));
            print(a.get(1));
            print(a.get(2));
        }",
    );
    // Copies [4, 5] starting at index 1.
    assert_eq!(out, vec!["1", "4", "5"]);
}

#[test]
fn map_returns_a_new_view_of_the_same_kind() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3);
            let b = a.map((v: any, i: any, arr: any) => (v as number) * 2.0 + (i as number));
            print(b.get(0));
            print(b.get(1));
            print(b.get(2));
            print(a.get(0));
        }",
    );
    assert_eq!(out, vec!["2", "5", "8", "1"]);
}

#[test]
fn filter_keeps_matching_elements() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3, 4);
            let b = a.filter((v: any, i: any, arr: any) => (v as number) > 2.0);
            print(b.length());
            print(b.get(0));
            print(b.get(1));
        }",
    );
    assert_eq!(out, vec!["2", "3", "4"]);
}

#[test]
fn for_each_visits_every_element_with_index() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(10, 20, 30);
            let total = 0.0;
            a.forEach((v: any, i: any, arr: any) => { total = total + (v as number) + (i as number); });
            print(total as int);
        }",
    );
    // (10+0) + (20+1) + (30+2) = 63.
    assert_eq!(out, vec!["63"]);
}

#[test]
fn reduce_folds_with_an_initial_value() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3, 4);
            let sum = a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number), 100.0);
            print(sum as number);
        }",
    );
    assert_eq!(out, vec!["110"]);
}

#[test]
fn reduce_without_initial_value_seeds_from_first_element() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3);
            let sum = a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number));
            print(sum as number);
        }",
    );
    assert_eq!(out, vec!["6"]);
}

#[test]
fn reduce_right_without_initial_value_seeds_from_last_element() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3);
            let r = a.reduceRight((acc: any, v: any, i: any, arr: any) => (acc as number) * 10.0 + (v as number));
            print(r as number);
        }",
    );
    // Seeds with 3, then 3*10+2=32, 32*10+1=321.
    assert_eq!(out, vec!["321"]);
}

#[test]
fn reduce_empty_without_initial_value_throws() {
    let src = zun_prelude::with_prelude(
        "function main(): number {
            let a = Int32Array.of();
            return a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number)) as number;
        }",
    );
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    let error = zun_vm::execute_with(&bytes, &mut out)
        .expect_err("empty reduce without initial value should throw");
    assert!(
        format!("{error:?}").contains("Reduce of empty array"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn find_and_find_index() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(5, 10, 15);
            print(a.find((v: any, i: any, arr: any) => (v as number) > 8.0));
            print(a.findIndex((v: any, i: any, arr: any) => (v as number) > 8.0));
            print(a.findIndex((v: any, i: any, arr: any) => (v as number) > 100.0));
        }",
    );
    assert_eq!(out, vec!["10", "1", "-1"]);
}

#[test]
fn some_and_every() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(2, 4, 6);
            print(a.some((v: any, i: any, arr: any) => (v as number) > 5.0));
            print(a.every((v: any, i: any, arr: any) => (v as number) % 2.0 == 0.0));
            print(a.every((v: any, i: any, arr: any) => (v as number) > 3.0));
        }",
    );
    assert_eq!(out, vec!["true", "true", "false"]);
}

#[test]
fn index_of_and_includes() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(10, 20, 30);
            print(a.indexOf(20));
            print(a.indexOf(99));
            print(a.includes(30));
            print(a.includes(31));
        }",
    );
    assert_eq!(out, vec!["1", "-1", "true", "false"]);
}

#[test]
fn join_renders_elements_with_a_separator() {
    let out = run_source(
        "function main() {
            let a = Int32Array.of(1, 2, 3);
            print(a.join(\"-\"));
            print(a.join(\",\"));
        }",
    );
    assert_eq!(out, vec!["1-2-3", "1,2,3"]);
}

#[test]
fn data_view_endianness_defaults_to_big_endian() {
    // The flag is now optional: `getUint32(0)` must type-check and read
    // big-endian, so the high byte sits at offset 0.
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(8);
            let dv = new DataView(b, 0, 8);
            dv.setUint32(0, 1);
            print(dv.getUint32(0));
            print(dv.getUint8(0));
            print(dv.getUint8(3));
        }",
    );
    assert_eq!(out, vec!["1", "0", "1"]);
}

#[test]
fn data_view_little_endian_flag_still_honored() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(8);
            let dv = new DataView(b, 0, 8);
            dv.setUint32(0, 1, true);
            print(dv.getUint8(0));
            print(dv.getUint32(0, true));
        }",
    );
    assert_eq!(out, vec!["1", "1"]);
}
