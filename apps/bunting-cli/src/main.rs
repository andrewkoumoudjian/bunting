#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    native::run().await;
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Native-only executable. The stub keeps platform dependencies out of the
    // Worker graph while allowing the workspace Wasm gate to compile every app.
}
