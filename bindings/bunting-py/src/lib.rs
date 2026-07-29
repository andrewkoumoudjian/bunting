#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

#[cfg(not(target_arch = "wasm32"))]
pub fn replay_contract(archive_json: &str) -> Result<String, String> {
    if archive_json.len() > 64 * 1_024 * 1_024 {
        return Err("archive exceeds 67108864 bytes".to_owned());
    }
    bunting_rs::BuntingHandle::replay_archive_json(archive_json)
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    #[pyclass]
    pub struct Bunting;

    #[pymethods]
    impl Bunting {
        #[new]
        fn new() -> Self {
            Self
        }

        fn replay_archive(&self, archive_json: &str) -> PyResult<String> {
            if archive_json.len() > 64 * 1_024 * 1_024 {
                return Err(PyValueError::new_err("archive exceeds 67108864 bytes"));
            }
            crate::replay_contract(archive_json).map_err(PyValueError::new_err)
        }
    }

    #[pymodule]
    mod bunting {
        #[pymodule_export]
        use super::Bunting;
    }
}
