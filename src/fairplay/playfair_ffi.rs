//! FFI to UxPlay's verified playfair library.

extern "C" {
    pub fn playfair_decrypt(
        message3: *const u8,
        cipher_text: *const u8,
        key_out: *mut u8,
    );
    pub fn sap_hash(
        block_in: *const u8,
        key_out: *mut u8,
    );
}
