use std::ffi::{CStr, CString, c_char};
use std::ptr;

const MAX_ARCHIVE_BYTES: usize = 64 * 1_024 * 1_024;

pub fn replay_contract(archive_json: &str) -> Result<String, String> {
    if archive_json.len() > MAX_ARCHIVE_BYTES {
        return Err("archive exceeds 67108864 bytes".to_owned());
    }
    bunting_rs::BuntingHandle::replay_archive_json(archive_json)
}

pub struct BuntingFfiHandle {
    _private: u8,
}

#[repr(C)]
pub struct BuntingFfiError {
    pub code: i32,
    pub message: *mut c_char,
}

#[unsafe(no_mangle)]
pub extern "C" fn bunting_handle_new() -> *mut BuntingFfiHandle {
    Box::into_raw(Box::new(BuntingFfiHandle { _private: 0 }))
}

/// # Safety
///
/// `handle` must be null or a pointer returned by `bunting_handle_new` that
/// has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bunting_handle_free(handle: *mut BuntingFfiHandle) {
    if !handle.is_null() {
        // SAFETY: required by the function contract above.
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// # Safety
///
/// `archive_json` must be a valid NUL-terminated string, `output_json` must be
/// writable, and `error` may be null or writable. Returned strings must be
/// released with `bunting_string_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bunting_replay_archive(
    handle: *const BuntingFfiHandle,
    archive_json: *const c_char,
    output_json: *mut *mut c_char,
    error: *mut BuntingFfiError,
) -> i32 {
    if handle.is_null() || archive_json.is_null() || output_json.is_null() {
        // SAFETY: `error` is checked before it is written.
        unsafe { set_error(error, 1, "null pointer") };
        return 1;
    }
    // SAFETY: required by this function's contract.
    let bytes = unsafe { CStr::from_ptr(archive_json) }.to_bytes();
    if bytes.len() > MAX_ARCHIVE_BYTES {
        // SAFETY: `error` is checked before it is written.
        unsafe { set_error(error, 2, "archive exceeds 67108864 bytes") };
        return 2;
    }
    let json = match std::str::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => {
            // SAFETY: `error` is checked before it is written.
            unsafe { set_error(error, 3, "archive is not UTF-8") };
            return 3;
        }
    };
    match replay_contract(json)
        .and_then(|value| CString::new(value).map_err(|_| "replay output contains NUL".to_owned()))
    {
        Ok(value) => {
            // SAFETY: `output_json` is writable by contract.
            unsafe { ptr::write(output_json, value.into_raw()) };
            0
        }
        Err(message) => {
            // SAFETY: `error` is checked before it is written.
            unsafe { set_error(error, 4, &message) };
            4
        }
    }
}

/// # Safety
///
/// `value` must be null or a pointer returned by this library that has not
/// already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bunting_string_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: required by this function's contract.
        drop(unsafe { CString::from_raw(value) });
    }
}

unsafe fn set_error(error: *mut BuntingFfiError, code: i32, message: &str) {
    if error.is_null() {
        return;
    }
    let Ok(message) = CString::new(message).or_else(|_| CString::new("binding error")) else {
        return;
    };
    // SAFETY: caller supplied a writable error pointer by contract.
    unsafe {
        ptr::write(
            error,
            BuntingFfiError {
                code,
                message: message.into_raw(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_handle_has_explicit_lifetime() {
        let handle = bunting_handle_new();
        assert!(!handle.is_null());
        // SAFETY: handle was allocated above and is freed once.
        unsafe { bunting_handle_free(handle) };
    }
}
