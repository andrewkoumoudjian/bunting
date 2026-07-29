#[cfg(not(target_arch = "wasm32"))]
#[cxx::bridge(namespace = "bunting")]
mod ffi {
    struct ReplaySummary {
        json: String,
    }

    extern "Rust" {
        type Bunting;
        fn new_bunting() -> Box<Bunting>;
        fn replay_archive(self: &Bunting, archive_json: &str) -> Result<ReplaySummary>;
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Bunting;

#[cfg(not(target_arch = "wasm32"))]
pub fn replay_contract(archive_json: &str) -> Result<String, String> {
    if archive_json.len() > 64 * 1_024 * 1_024 {
        return Err("archive exceeds 67108864 bytes".to_owned());
    }
    bunting_rs::BuntingHandle::replay_archive_json(archive_json)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn new_bunting() -> Box<Bunting> {
    Box::new(Bunting)
}

#[cfg(not(target_arch = "wasm32"))]
impl Bunting {
    fn replay_archive(&self, archive_json: &str) -> Result<ffi::ReplaySummary, String> {
        replay_contract(archive_json).map(|json| ffi::ReplaySummary { json })
    }
}
