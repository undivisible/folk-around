use std::ffi::{CStr, c_char};

use folk_core::AccessMode;
use thiserror::Error;

unsafe extern "C" {
    fn folk_zig_legacy_init() -> i32;
    fn folk_zig_legacy_call(
        name_ptr: *const u8,
        name_len: usize,
        args_ptr: *const u8,
        args_len: usize,
        mode: u8,
    ) -> *mut c_char;
    fn folk_zig_legacy_free(ptr: *mut c_char);
}

#[derive(Debug, Error)]
pub enum LegacyError {
    #[error("legacy bridge initialization failed")]
    Init,
    #[error("legacy bridge call failed")]
    Call,
    #[error("legacy bridge returned invalid utf8")]
    Utf8(#[from] std::str::Utf8Error),
}

pub struct LegacyBridge;

impl LegacyBridge {
    pub fn init() -> Result<Self, LegacyError> {
        let code = unsafe { folk_zig_legacy_init() };
        if code == 0 {
            Ok(Self)
        } else {
            Err(LegacyError::Init)
        }
    }

    pub fn call(
        &self,
        name: &str,
        args_json: &str,
        mode: AccessMode,
    ) -> Result<String, LegacyError> {
        let mode = match mode {
            AccessMode::Full => 0,
            AccessMode::Limited => 1,
            AccessMode::Sandbox => 2,
        };
        let ptr = unsafe {
            folk_zig_legacy_call(
                name.as_ptr(),
                name.len(),
                args_json.as_ptr(),
                args_json.len(),
                mode,
            )
        };
        if ptr.is_null() {
            return Err(LegacyError::Call);
        }
        let text = unsafe { CStr::from_ptr(ptr) }.to_str()?.to_string();
        unsafe { folk_zig_legacy_free(ptr) };
        Ok(text)
    }
}
