//! SapHash algorithm — delegates to C implementation via FFI.

use super::playfair_ffi::sap_hash as sap_hash_c;

/// Call the C sap_hash implementation directly.
pub fn sap_hash_impl(block_in: &[u8; 64], key_out: &mut [u8; 16]) {
    unsafe {
        sap_hash_c(block_in.as_ptr(), key_out.as_mut_ptr());
    }
}
