//! The tail of the typed-array surface: `byteOffset`/`buffer` and iteration.
//!
//! `byteOffset` and `buffer` come from the master table in `zun_core`, so
//! every kind gets them at once; a Uint8Array and a Float64Array are spot
//! checks that the generated wiring reaches each element width. Iteration
//! builds an element sequence and reuses the array iterator, the same shape
//! the string iterator uses.

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
fn byte_offset_reflects_the_construction() {
    // `new Uint32Array(b, 8, 4)` starts eight bytes in; the offset counts
    // bytes, not elements, so it is the raw `8` rather than two elements.
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(64);
            let a = new Uint32Array(b, 8, 4);
            print(a.byteOffset());
            print(a.length());
            print(a.byteLength());
        }",
    );
    assert_eq!(out, vec!["8", "4", "16"]);
}

#[test]
fn buffer_refers_back_to_the_backing_array_buffer() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(64);
            let a = new Float64Array(b, 16, 4);
            print(a.buffer().byteLength());
        }",
    );
    assert_eq!(out, vec!["64"]);
}

#[test]
fn a_subarray_keeps_the_same_backing_buffer_and_shifts_the_offset() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(64);
            let a = new Uint16Array(b, 4, 8);
            let sub = a.subarray(2, 6);
            // The window moved two elements (four bytes) past `a`'s start.
            print(sub.byteOffset());
            print(sub.buffer().byteLength());
        }",
    );
    assert_eq!(out, vec!["8", "64"]);
}

#[test]
fn for_of_visits_every_element_in_order() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(8);
            let a = new Uint8Array(b, 0, 4);
            a.set(0, 10);
            a.set(1, 20);
            a.set(2, 30);
            a.set(3, 40);
            for (let x of a) {
                print(x);
            }
        }",
    );
    assert_eq!(out, vec!["10", "20", "30", "40"]);
}

#[test]
fn for_of_works_over_floats_too() {
    let out = run_source(
        "function main() {
            let b = new ArrayBuffer(32);
            let a = new Float64Array(b, 0, 3);
            a.set(0, 1.5);
            a.set(1, 2.5);
            a.set(2, 3.5);
            for (let x of a) {
                print(x);
            }
        }",
    );
    assert_eq!(out, vec!["1.5", "2.5", "3.5"]);
}

#[test]
fn symbol_iterator_is_present_and_drives_iteration() {
    // The explicit `[Symbol.iterator]()` form mints an iterator whose
    // `next` yields the elements, the same one `for...of` desugars to.
    let out = run_source(
        "import { arrayIteratorNext } from \"zun:core\";
         function main() {
            let b = new ArrayBuffer(8);
            let a = new Uint8Array(b, 0, 2);
            a.set(0, 7);
            a.set(1, 9);
            let it = a[Symbol.iterator]();
            print(arrayIteratorNext(it));
            print(arrayIteratorNext(it));
        }",
    );
    assert_eq!(out, vec!["7", "9"]);
}
