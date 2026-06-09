//! SapHash — calls verified C implementation for the garble-based hash.
//! This is the ONLY C FFI dependency in the codebase.
//! All other FairPlay crypto (OmgHax, cycle, key schedule, modified_md5,
//! decrypt_message, decrypt_aes_key, video/audio decryptors) is pure Rust.

extern "C" {
    fn sap_hash(block_in: *const u8, key_out: *mut u8);
}

/// Call the C sap_hash implementation (byte-verified against java-airplay).
pub fn sap_hash_impl(block_in: &[u8; 64], key_out: &mut [u8; 16]) {
    unsafe {
        sap_hash(block_in.as_ptr(), key_out.as_mut_ptr());
    }
}
