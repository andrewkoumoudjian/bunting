use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let output = PathBuf::from(env::var("OUT_DIR").expect("Cargo sets OUT_DIR")).join("bunting.h");
    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(cbindgen::Config::from_file("cbindgen.toml").expect("valid cbindgen config"))
        .generate()
        .expect("cbindgen generation succeeds")
        .write_to_file(output);
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
}
