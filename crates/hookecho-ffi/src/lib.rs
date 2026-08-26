//! C ABI bridge to the HookEcho decoders.
//!
//! The bridge hands back JSON rather than opaque handles. A decoded `Level3Product` is a tree of
//! `Vec`s, `String`s and nested `Vec`s; exposing it accessor-by-accessor would take dozens of
//! extern functions and a lifetime story to match, while one `serde_json` string crosses the
//! boundary with a single ownership rule: whatever this library returns, this library frees.
//!
//! Every entry point is total. Decode failures come back as `{"error": "…"}` rather than a null
//! pointer, so a caller only has to null-check for out-of-memory.
//!
//! See `include/hookecho.h` for the C declarations.

use std::ffi::{c_char, CString};

/// Decode a NEXRAD Level 3 product and return it as a JSON string.
///
/// `data` must point to `len` readable bytes, or be null when `len` is zero. The returned string
/// is NUL-terminated, owned by this library, and must be released with
/// [`hookecho_string_free`]. It is null only if the allocation itself failed.
///
/// # Safety
///
/// The caller guarantees `data` points to `len` initialised bytes that stay valid for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn hookecho_l3_decode_json(data: *const u8, len: usize) -> *mut c_char {
    let bytes: &[u8] = if data.is_null() {
        &[]
    } else {
        // SAFETY: the caller's contract, above.
        unsafe { std::slice::from_raw_parts(data, len) }
    };

    let json = match nexrad_level3::decode(bytes) {
        Ok(product) => serde_json::to_string(&product)
            .unwrap_or_else(|e| error_json(&format!("serialize: {e}"))),
        Err(e) => error_json(&e.to_string()),
    };

    // A decoded product can carry raw text lifted straight out of the message, which may hold an
    // interior NUL. Trading it for a space keeps the string printable rather than truncated.
    let json = json.replace('\0', " ");
    match CString::new(json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by this library. Passing null is a no-op; passing anything this library
/// did not return is undefined behaviour.
///
/// # Safety
///
/// `s` must be null or a pointer previously returned by [`hookecho_l3_decode_json`], and must not
/// be freed twice.
#[no_mangle]
pub unsafe extern "C" fn hookecho_string_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: the caller's contract, above — this reclaims the `CString::into_raw` allocation.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Build the `{"error": …}` payload, letting `serde_json` do the escaping.
fn error_json(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    /// Call the pair the way C does — through the extern function pointers — on a real product.
    #[test]
    fn decodes_a_storm_tracking_product_through_the_c_abi() {
        let decode: unsafe extern "C" fn(*const u8, usize) -> *mut c_char = hookecho_l3_decode_json;
        let free: unsafe extern "C" fn(*mut c_char) = hookecho_string_free;

        let bytes = include_bytes!("../../nexrad-level3/tests/data/nst_tlx.l3");
        let raw = unsafe { decode(bytes.as_ptr(), bytes.len()) };
        assert!(!raw.is_null(), "allocation failed");

        let json = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_owned();
        unsafe { free(raw) };

        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["code"], 58, "NST product code");
        assert!(
            (v["lat"].as_f64().unwrap() - 35.33).abs() < 0.1,
            "KTLX latitude, got {}",
            v["lat"]
        );
        assert!(
            !v["cells"].as_array().unwrap().is_empty(),
            "expected tracked storm cells"
        );
    }

    /// Garbage in, `{"error": …}` out — never a null pointer, never a panic across the boundary.
    #[test]
    fn reports_decode_failure_as_json() {
        for (data, len) in [
            (b"not a radar product".as_ptr(), 19usize),
            (std::ptr::null(), 0),
        ] {
            let raw = unsafe { hookecho_l3_decode_json(data, len) };
            assert!(!raw.is_null());
            let json = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_owned();
            unsafe { hookecho_string_free(raw) };

            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(
                v["error"].is_string(),
                "expected an error payload, got {json}"
            );
        }
    }

    /// Freeing null is the documented no-op.
    #[test]
    fn freeing_null_is_a_no_op() {
        unsafe { hookecho_string_free(std::ptr::null_mut()) };
    }
}
