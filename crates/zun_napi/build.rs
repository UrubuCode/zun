fn main() {
    if std::env::var_os("CARGO_FEATURE_NODE_API").is_none() {
        return;
    }

    println!("cargo:rerun-if-env-changed=ZUN_NODE_LIB");
    if let Some(path) = std::env::var_os("ZUN_NODE_LIB") {
        let path = std::path::Path::new(&path);
        let directory = path.parent().expect("ZUN_NODE_LIB must name a file");
        let file = path.file_stem().expect("ZUN_NODE_LIB must name a file");
        println!("cargo:rustc-link-search=native={}", directory.display());
        println!("cargo:rustc-link-lib=dylib={}", file.to_string_lossy());
    } else {
        println!(
            "cargo:warning=node-api enabled without ZUN_NODE_LIB; set it to node.lib (Windows) or libnode.so/libnode.dylib"
        );
    }
}
