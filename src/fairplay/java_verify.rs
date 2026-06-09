//! Verify Rust OmgHax implementation against Java test vectors from java-airplay-lib.
//! These tests use the PURE RUST implementation (no C FFI).

#[cfg(test)]
mod tests {
    #[test]
    fn java_generate_session_key() {
        // From OmgHaxTest.java: generateSessionKey()
        let message_in: [u8; 164] = [
            70, 80, 76, 89, 3, 1, 3, 0, 0, 0, 0, 152, 0, 143, 26, 156,
            216, 164, 246, 52, 109, 20, 120, 6, 194, 189, 138, 75, 209, 185, 147, 211,
            195, 106, 161, 1, 36, 152, 249, 78, 255, 243, 70, 123, 207, 27, 49, 152,
            98, 92, 162, 69, 142, 62, 208, 30, 221, 53, 231, 41, 53, 125, 249, 75,
            128, 205, 10, 206, 35, 84, 214, 140, 227, 127, 94, 24, 240, 207, 210, 109,
            65, 103, 21, 63, 192, 180, 54, 35, 22, 111, 8, 198, 111, 211, 1, 56,
            14, 176, 158, 159, 141, 232, 59, 210, 174, 199, 164, 1, 241, 251, 189, 243,
            46, 10, 213, 81, 232, 121, 63, 231, 193, 25, 35, 51, 153, 165, 53, 76,
            197, 67, 7, 30, 188, 206, 224, 172, 133, 34, 174, 27, 171, 51, 212, 65,
            196, 120, 245, 99, 206, 253, 66, 117, 251, 85, 90, 58, 227, 58, 216, 185,
            249, 148, 249, 181
        ];
        
        let mut session_key = [0u8; 16];
        let old_sap = crate::fairplay::omghax_const::DEFAULT_SAP;
        
        // Pure Rust version (no C FFI)
        crate::fairplay::omghax::generate_session_key(&old_sap, &message_in, &mut session_key);
        
        // Java expected: [39, 110, -67, 89, -58, 116, 70, 37, 101, -9, -9, -68, -58, 68, 4, 50]
        let expected: [u8; 16] = [39, 110, 189, 89, 198, 116, 70, 37, 101, 247, 247, 188, 198, 68, 4, 50];
        
        println!("Rust session_key: {:02x?}", session_key);
        println!("Java expected:    {:02x?}", expected);
        assert_eq!(session_key, expected, "generate_session_key mismatch!");
    }

    #[test]
    fn java_generate_key_schedule() {
        // From OmgHaxTest.java: generateKeySchedule1Test()
        let key_material: [u8; 16] = crate::fairplay::omghax_const::INITIAL_SESSION_KEY;
        
        let mut ks = [[0xdeadbeefu32; 4]; 11];
        
        // Pure Rust version
        crate::fairplay::omghax::generate_key_schedule(&key_material, &mut ks);
        
        // Java expected key_schedule[0]
        let expected0: [u32; 4] = [0xD85AD80C, 0x7CDCD060, 0x75A8F4EE, 0xF8494703];
        println!("Rust ks[0]: {:08x?}", ks[0]);
        println!("Java ks[0]: {:08x?}", expected0);
        assert_eq!(ks[0], expected0, "key_schedule[0] mismatch!");
        
        // Java expected key_schedule[10]
        let expected10: [u32; 4] = [0xE2DCE619, 0x651B2A22, 0x1B38C199, 0xFD72B9D5];
        println!("Rust ks[10]: {:08x?}", ks[10]);
        println!("Java ks[10]: {:08x?}", expected10);
        assert_eq!(ks[10], expected10, "key_schedule[10] mismatch!");
    }
}
