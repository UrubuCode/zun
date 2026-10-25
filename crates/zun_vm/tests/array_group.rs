//! `Array.prototype.group` / `groupToMap` (NUCLEUS #7).

use zun_interp::CaptureOutput;

fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut out).expect("program should run");
    out.lines
}

#[test]
fn group_buckets_by_string_key() {
    let out = run_source(
        "function main() { \
            let xs = [1.0, 2.0, 3.0, 4.0, 5.0]; \
            let g: any = xs.group((x: any) => (x as number) % 2.0 == 0.0 ? \"even\" : \"odd\"); \
            let even = g.even as Array; \
            let odd = g.odd as Array; \
            print(even.length); print(odd.length); print(odd[2] as number); }",
    );
    assert_eq!(out, ["2", "3", "5"]);
}

#[test]
fn group_to_map_buckets_by_callback_result() {
    let out = run_source(
        "function main() { \
            let xs = [1.0, 2.0, 3.0, 4.0]; \
            let m = xs.groupToMap((x: any) => (x as number) % 2.0); \
            let odd = m.get(1.0) as Array; \
            let even = m.get(0.0) as Array; \
            print(m.size); print(odd.length); print(even[1] as number); }",
    );
    assert_eq!(out, ["2", "2", "4"]);
}
