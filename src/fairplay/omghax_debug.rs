//! Detailed debug test for OmgHax with known values.
//! Run with: cargo test omghax_debug -- --nocapture

#[cfg(test)]
mod tests {
    #[test]
    fn omghax_debug() {
        let key_msg: [u8; 164] = [
            0x46,0x50,0x4c,0x59,0x03,0x01,0x03,0x00,0x00,0x00,0x00,0x98,0x00,0x8f,0x1a,0x9c,
            0x0c,0xbc,0xf8,0x97,0x1b,0x17,0xd0,0x8a,0x02,0x60,0xc8,0x97,0x13,0xde,0x4c,0xd0,
            0xcc,0x3b,0xc9,0xd2,0xe7,0x5f,0xe7,0xcb,0x70,0xd4,0xf5,0x71,0x92,0x25,0x62,0x57,
            0xbe,0x8e,0xe2,0xd2,0xdd,0x0c,0xd1,0x38,0x0f,0xad,0x13,0xb2,0xb8,0xc1,0xfb,0x3a,
            0xf4,0xa4,0xc9,0x87,0x65,0xf2,0x82,0x3e,0xb0,0xcb,0x2a,0x26,0xbb,0xf7,0x27,0xdd,
            0x13,0x35,0x76,0x28,0xa7,0x93,0xd1,0xc8,0x77,0xb7,0x74,0x92,0xe6,0x20,0xd6,0x47,
            0x90,0x8e,0x0a,0xa9,0xa5,0x17,0xf8,0x59,0x6f,0xda,0x43,0x5f,0x1c,0xaf,0x82,0x27,
            0xe0,0xc8,0x3a,0x51,0x27,0x89,0xd9,0xe0,0x53,0x66,0xb0,0xb4,0x4c,0xeb,0x26,0x11,
            0xe9,0xcf,0x53,0xbf,0xfb,0xf8,0x4b,0xa1,0x8d,0x80,0x1e,0x1b,0x43,0x72,0x59,0x57,
            0xd6,0x3f,0xa2,0x02,0x10,0xaf,0xe2,0xce,0x7f,0xc0,0x57,0xcb,0x1e,0xe0,0xb6,0xb3,
            0x14,0xa4,0x6c,0x3a,
        ];

        let ekey: [u8; 72] = [
            0x46,0x50,0x4c,0x59,0x01,0x02,0x01,0x00,0x00,0x00,0x00,0x3c,0x00,0x00,0x00,0x00,
            0x36,0x46,0x75,0x4a,0xa9,0x5b,0xbc,0x4d,0x64,0x3a,0x65,0x68,0x3b,0x81,0x9d,0x7d,
            0x00,0x00,0x00,0x10,0x83,0x8a,0xdc,0x52,0xcf,0xa0,0x76,0x28,0xb0,0xfe,0xa3,0xed,
            0x72,0x6e,0xb2,0x68,0x5e,0xf5,0xcb,0xf3,0x48,0xf1,0x6f,0x41,0x43,0xcc,0xfa,0x34,
            0xd5,0x03,0x11,0xe9,0x26,0x79,0xa3,0xe3,
        ];

        let expected: [u8; 16] = [
            0x65,0x5a,0x5f,0xfc,0x27,0x20,0xea,0x6d,
            0x97,0x30,0xf1,0x6e,0xb1,0x7f,0x58,0xe2,
        ];

        // Test: compare Rust and C generate_key_schedule
        let sap_key: [u8; 16] = [0xa8, 0x6f, 0xdb, 0x2b, 0x79, 0x82, 0x6d, 0x3d, 0x46, 0xa3, 0x13, 0x62, 0x4a, 0xfe, 0x80, 0xf1];
        
        let mut rust_ks = [[0xdeadbeefu32; 4]; 11];
        crate::fairplay::omghax::generate_key_schedule(&sap_key, &mut rust_ks);
        println!("Rust key_schedule[0]: {:08x?}", rust_ks[0]);
        println!("Rust key_schedule[10]: {:08x?}", rust_ks[10]);

        // C generate_key_schedule
        extern "C" {
            fn generate_key_schedule(key_material: *const u8, key_schedule: *mut u32);
        }
        let mut c_ks = [[0xdeadbeefu32; 4]; 11];
        unsafe {
            generate_key_schedule(sap_key.as_ptr(), c_ks.as_mut_ptr() as *mut u32);
        }
        println!("C   key_schedule[0]: {:08x?}", c_ks[0]);
        println!("C   key_schedule[10]: {:08x?}", c_ks[10]);

        assert_eq!(rust_ks[0], c_ks[0], "key_schedule[0] mismatch!");
        assert_eq!(rust_ks[10], c_ks[10], "key_schedule[10] mismatch!");

        // Test: compare Rust and C modified_md5
        let block = [0u8; 64];
        let key = [0u8; 16];
        let mut rust_md5 = [0u8; 16];
        crate::fairplay::modified_md5::modified_md5(&block, &key, &mut rust_md5);
        
        extern "C" {
            fn modified_md5(block: *const u8, key: *const u8, out: *mut u8);
        }
        let mut c_md5 = [0u8; 16];
        unsafe {
            modified_md5(block.as_ptr(), key.as_ptr(), c_md5.as_mut_ptr());
        }
        println!("Rust modified_md5: {:02x?}", rust_md5);
        println!("C   modified_md5: {:02x?}", c_md5);
        assert_eq!(rust_md5, c_md5, "modified_md5 mismatch!");

        // Test: compare Rust and C sap_hash
        let mut rust_sap = [0u8; 16];
        crate::fairplay::sap_hash::sap_hash_impl(&block, &mut rust_sap);
        
        let mut c_sap = [0u8; 16];
        unsafe {
            crate::fairplay::playfair_ffi::sap_hash(block.as_ptr(), c_sap.as_mut_ptr());
        }
        println!("Rust sap_hash: {:02x?}", rust_sap);
        println!("C   sap_hash: {:02x?}", c_sap);
        assert_eq!(rust_sap, c_sap, "sap_hash mismatch!");

        // Test: compare Rust and C generate_session_key
        let default_sap = crate::fairplay::omghax_const::DEFAULT_SAP;
        let mut rust_sk = [0u8; 16];
        crate::fairplay::omghax::generate_session_key(&default_sap, &key_msg, &mut rust_sk);
        
        let mut c_sk = [0u8; 16];
        unsafe {
            extern "C" {
                fn generate_session_key(oldSap: *const u8, messageIn: *const u8, sessionKey: *mut u8);
            }
            generate_session_key(default_sap.as_ptr(), key_msg.as_ptr(), c_sk.as_mut_ptr());
        }
        println!("Rust sapKey: {:02x?}", rust_sk);
        println!("C   sapKey: {:02x?}", c_sk);
        assert_eq!(rust_sk, c_sk, "sapKey mismatch!");

        // Test: compare Rust and C cycle
        let chunk2: [u8; 16] = ekey[56..72].try_into().unwrap();
        let mut rust_block = [0u8; 16];
        for i in 0..16 {
            rust_block[i] = chunk2[i] ^ crate::fairplay::omghax_const::Z_KEY[i];
        }
        crate::fairplay::omghax::cycle(&mut rust_block, &rust_ks);
        
        let mut c_block = [0u8; 16];
        for i in 0..16 {
            c_block[i] = chunk2[i] ^ crate::fairplay::omghax_const::Z_KEY[i];
        }
        unsafe {
            extern "C" {
                fn cycle(block: *mut u8, key_schedule: *mut u32);
            }
            cycle(c_block.as_mut_ptr(), c_ks.as_mut_ptr() as *mut u32);
        }
        println!("Rust cycle: {:02x?}", rust_block);
        println!("C   cycle: {:02x?}", c_block);
        assert_eq!(rust_block, c_block, "cycle mismatch!");

        // Test: compare full decrypt_aes_key
        let chunk1_rust = &ekey[16..];
        let chunk2_rust = &ekey[56..];
        println!("Rust chunk1[0..16]: {:02x?}", &chunk1_rust[..16]);
        println!("Rust chunk2[0..16]: {:02x?}", &chunk2_rust[..16]);

        let rust_key = crate::fairplay::omghax::decrypt_aes_key(&key_msg, &ekey);
        let mut c_key = [0u8; 16];
        unsafe {
            extern "C" {
                fn playfair_decrypt(message3: *const u8, cipher_text: *const u8, key_out: *mut u8);
            }
            playfair_decrypt(key_msg.as_ptr(), ekey.as_ptr(), c_key.as_mut_ptr());
        }
        println!("Rust key: {:02x?}", rust_key);
        println!("C   key: {:02x?}", c_key);
        assert_eq!(rust_key, c_key, "Final key mismatch!");

        // Both implementations should agree
        println!("All intermediate values match between Rust and C!");
    }
}
