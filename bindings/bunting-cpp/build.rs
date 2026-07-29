fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    cxx_build::bridge("src/lib.rs")
        .std("c++17")
        .compile("bunting-cpp");
    println!("cargo:rerun-if-changed=src/lib.rs");
}
