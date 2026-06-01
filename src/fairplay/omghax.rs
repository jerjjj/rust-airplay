//! OmgHax decryption algorithm — faithful translation from OmgHax.java.
//!
//! Reverse-engineered custom block cipher used by FairPlay to protect
//! the AES stream key. Based on:
//!   https://github.com/serezhka/java-airplay/blob/main/lib/src/main/java/com/github/serezhka/airplay/lib/internal/OmgHax.java
//!
//! NOTE: The core algorithm depends on binary lookup tables (table_s1..table_s10)
//! that are loaded from JAR resources in the Java implementation. Until those
//! tables are extracted, the algorithm structure is correct but output WILL NOT
//! match the real implementation.

use super::omghax_const;

/// Decrypt the FairPlay-encrypted AES stream key.
///
/// `key_msg`: 164-byte buffer from fp-setup round 2.
/// `ekey`: ekey data from RTSP SETUP (at least 72 bytes).
///
/// Returns the 16-byte AES key.
pub fn decrypt_aes_key(key_msg: &[u8], ekey: &[u8]) -> [u8; 16] {
    // Java: chunk1 = Arrays.copyOfRange(cipherText, 16, cipherText.length)
    // Java: chunk2 = Arrays.copyOfRange(cipherText, 56, cipherText.length)
    let chunk1 = &ekey[16..];
    let chunk2 = &ekey[56..];

    let mut block_in = [0u8; 16];
    let mut sap_key = [0u8; 16];
    let mut key_schedule = [[0u32; 4]; 11];

    // Use C generate_session_key
    unsafe {
        extern "C" {
            fn generate_session_key(oldSap: *const u8, messageIn: *const u8, sessionKey: *mut u8);
        }
        generate_session_key(omghax_const::DEFAULT_SAP.as_ptr(), key_msg.as_ptr(), sap_key.as_mut_ptr());
    }

    // Use C generate_key_schedule
    unsafe {
        extern "C" {
            fn generate_key_schedule(key_material: *const u8, key_schedule: *mut u32);
        }
        generate_key_schedule(sap_key.as_ptr(), key_schedule.as_mut_ptr() as *mut u32);
    }

    // Use C z_xor, cycle, x_xor, z_xor
    unsafe {
        extern "C" {
            fn z_xor(in_ptr: *const u8, out_ptr: *mut u8, blocks: i32);
            fn cycle(block: *mut u8, key_schedule: *mut u32);
            fn x_xor(in_ptr: *const u8, out_ptr: *mut u8, blocks: i32);
        }
        
        z_xor(chunk2.as_ptr(), block_in.as_mut_ptr(), 1);
        println!("after z_xor: {:02x?}", block_in);
        
        cycle(block_in.as_mut_ptr(), key_schedule.as_mut_ptr() as *mut u32);
        println!("after cycle: {:02x?}", block_in);
        
        let mut key_out = [0u8; 16];
        for i in 0..16 {
            key_out[i] = block_in[i] ^ chunk1[i];
        }
        println!("after xor chunk1: {:02x?}", key_out);
        println!("x_key: {:02x?}", omghax_const::X_KEY);
        
        x_xor(key_out.as_ptr(), key_out.as_mut_ptr(), 1);
        println!("after x_xor: {:02x?}", key_out);
        println!("z_key: {:02x?}", omghax_const::Z_KEY);
        
        z_xor(key_out.as_ptr(), key_out.as_mut_ptr(), 1);
        println!("after z_xor: {:02x?}", key_out);
        
        key_out
    }
}

fn get_chunk(data: &[u8], offset: usize) -> [u8; 16] {
    let mut chunk = [0u8; 16];
    let end = (offset + 16).min(data.len());
    let len = end.saturating_sub(offset);
    chunk[..len].copy_from_slice(&data[offset..end]);
    chunk
}

// ── Core cipher: cycle() ─────────────────────────────────────

/// 10-round custom block cipher operating on 16 bytes in-place.
pub fn cycle(block: &mut [u8; 16], key_schedule: &[[u32; 4]; 11]) {
    // Initial: XOR with key_schedule[10]
    for i in 0..4 {
        let val = u32::from_le_bytes([block[i*4], block[i*4+1], block[i*4+2], block[i*4+3]]);
        let xored = val ^ key_schedule[10][i];
        let bytes = xored.to_le_bytes();
        block[i*4..i*4+4].copy_from_slice(&bytes);
    }

    // First permutation
    permute_block_1(block);

    for round in 0..9 {
        // S-Box round using key_schedule[9-round]
        let key_idx = 9 - round;
        let key_bytes = key_schedule_to_bytes(&key_schedule[key_idx]);

        // Four 32-bit words via T-tables
        let w0 = omghax_const::TABLE_S5[(block[3] as usize) ^ (key_bytes[3] as usize)]
            ^ omghax_const::TABLE_S6[(block[2] as usize) ^ (key_bytes[2] as usize)]
            ^ omghax_const::TABLE_S8[(block[0] as usize) ^ (key_bytes[0] as usize)]
            ^ omghax_const::TABLE_S7[(block[1] as usize) ^ (key_bytes[1] as usize)];

        let w1 = omghax_const::TABLE_S5[(block[7] as usize) ^ (key_bytes[7] as usize)]
            ^ omghax_const::TABLE_S6[(block[6] as usize) ^ (key_bytes[6] as usize)]
            ^ omghax_const::TABLE_S7[(block[5] as usize) ^ (key_bytes[5] as usize)]
            ^ omghax_const::TABLE_S8[(block[4] as usize) ^ (key_bytes[4] as usize)];

        let w2 = omghax_const::TABLE_S5[(block[11] as usize) ^ (key_bytes[11] as usize)]
            ^ omghax_const::TABLE_S6[(block[10] as usize) ^ (key_bytes[10] as usize)]
            ^ omghax_const::TABLE_S7[(block[9] as usize) ^ (key_bytes[9] as usize)]
            ^ omghax_const::TABLE_S8[(block[8] as usize) ^ (key_bytes[8] as usize)];

        let w3 = omghax_const::TABLE_S5[(block[15] as usize) ^ (key_bytes[15] as usize)]
            ^ omghax_const::TABLE_S6[(block[14] as usize) ^ (key_bytes[14] as usize)]
            ^ omghax_const::TABLE_S7[(block[13] as usize) ^ (key_bytes[13] as usize)]
            ^ omghax_const::TABLE_S8[(block[12] as usize) ^ (key_bytes[12] as usize)];

        block[0..4].copy_from_slice(&w0.to_le_bytes());
        block[4..8].copy_from_slice(&w1.to_le_bytes());
        block[8..12].copy_from_slice(&w2.to_le_bytes());
        block[12..16].copy_from_slice(&w3.to_le_bytes());

        // Permutation (except on last round — but Java always does it for rounds 0..8)
        permute_block_2(block, 8 - round);
    }

    // Final: XOR with key_schedule[0]
    for i in 0..4 {
        let val = u32::from_le_bytes([block[i*4], block[i*4+1], block[i*4+2], block[i*4+3]]);
        let xored = val ^ key_schedule[0][i];
        let bytes = xored.to_le_bytes();
        block[i*4..i*4+4].copy_from_slice(&bytes);
    }
}

fn key_schedule_to_bytes(ks: &[u32; 4]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for i in 0..4 {
        bytes[i*4..i*4+4].copy_from_slice(&ks[i].to_le_bytes());
    }
    bytes
}

// ── Permutations ─────────────────────────────────────────────

/// Permute block using table_s3 (4096-entry S-Box).
/// Direct translation of permute_block_1 from OmgHax.java.
fn permute_block_1(block: &mut [u8; 16]) {
    let s3 = &omghax_const::TABLE_S3;

    block[0]  = s3[block[0] as usize];
    block[4]  = s3[0x400 + block[4] as usize];
    block[8]  = s3[0x800 + block[8] as usize];
    block[12] = s3[0xC00 + block[12] as usize];

    let tmp = block[13];
    block[13] = s3[0x100 + block[9] as usize];
    block[9]  = s3[0xD00 + block[5] as usize];
    block[5]  = s3[0x900 + block[1] as usize];
    block[1]  = s3[0x500 + tmp as usize];

    let tmp = block[2];
    block[2]  = s3[0xA00 + block[10] as usize];
    block[10] = s3[0x200 + tmp as usize];
    let tmp = block[6];
    block[6]  = s3[0xE00 + block[14] as usize];
    block[14] = s3[0x600 + tmp as usize];

    let tmp = block[3];
    block[3]  = s3[0xF00 + block[7] as usize];
    block[7]  = s3[0x300 + block[11] as usize];
    block[11] = s3[0x700 + block[15] as usize];
    block[15] = s3[0xB00 + tmp as usize];
}

/// Permute block using a slice of table_s4.
/// The slice index is: (71 * (round*16 + col)) % 144, with 256 bytes each.
fn permute_block_2(block: &mut [u8; 16], round: usize) {
    let base = round * 16;

    block[0]  = permute_table_2_lookup(base + 0,  block[0]);
    block[4]  = permute_table_2_lookup(base + 4,  block[4]);
    block[8]  = permute_table_2_lookup(base + 8,  block[8]);
    block[12] = permute_table_2_lookup(base + 12, block[12]);

    let tmp = block[13];
    block[13] = permute_table_2_lookup(base + 13, block[9]);
    block[9]  = permute_table_2_lookup(base + 9,  block[5]);
    block[5]  = permute_table_2_lookup(base + 5,  block[1]);
    block[1]  = permute_table_2_lookup(base + 1,  tmp);

    let tmp = block[2];
    block[2]  = permute_table_2_lookup(base + 2,  block[10]);
    block[10] = permute_table_2_lookup(base + 10, tmp);
    let tmp = block[6];
    block[6]  = permute_table_2_lookup(base + 6,  block[14]);
    block[14] = permute_table_2_lookup(base + 14, tmp);

    let tmp = block[3];
    block[3]  = permute_table_2_lookup(base + 3,  block[7]);
    block[7]  = permute_table_2_lookup(base + 7,  block[11]);
    block[11] = permute_table_2_lookup(base + 11, block[15]);
    block[15] = permute_table_2_lookup(base + 15, tmp);
}

fn permute_table_2_lookup(idx: usize, byte_val: u8) -> u8 {
    let table_idx = (71 * idx) % 144;
    let offset = (table_idx << 8) + byte_val as usize;
    if offset < omghax_const::TABLE_S4.len() {
        omghax_const::TABLE_S4[offset]
    } else {
        byte_val
    }
}

// ── XOR helpers ──────────────────────────────────────────────

fn z_xor(input: &[u8; 16], output: &mut [u8; 16]) {
    for i in 0..16 {
        output[i] = input[i] ^ omghax_const::Z_KEY[i];
    }
}

fn x_xor_inplace(block: &mut [u8; 16]) {
    for i in 0..16 {
        block[i] ^= omghax_const::X_KEY[i];
    }
}

#[allow(dead_code)]
fn t_xor(input: &[u8; 16], output: &mut [u8; 16]) {
    for i in 0..16 {
        output[i] = input[i] ^ omghax_const::T_KEY[i];
    }
}

// ── Key schedule generation ──────────────────────────────────

/// Generate the 11-round key schedule from session key material.
/// Translation of generate_key_schedule from OmgHax.java.
pub fn generate_key_schedule(key_material: &[u8; 16], key_schedule: &mut [[u32; 4]; 11]) {
    // Initialize with 0xDEADBEEF
    for round in 0..11 {
        key_schedule[round] = [0xDEAD_BEEFu32; 4];
    }

    let mut buffer = [0u8; 16];
    t_xor(key_material, &mut buffer);

    let mut key_data = [0u32; 4];
    for i in 0..4 {
        key_data[i] = u32::from_le_bytes([
            buffer[i*4], buffer[i*4+1], buffer[i*4+2], buffer[i*4+3],
        ]);
    }

    let mut ti: usize = 0;
    for round in 0..11 {
        key_schedule[round][0] = key_data[0];

        let t1 = table_index(ti);
        let t2 = table_index(ti + 1);
        let t3 = table_index(ti + 2);
        let t4 = table_index(ti + 3);
        ti += 4;

        buffer[0] ^= lookup(t1, buffer[0x0D]) ^ omghax_const::INDEX_MANGLE[round];
        buffer[1] ^= lookup(t2, buffer[0x0E]);
        buffer[2] ^= lookup(t3, buffer[0x0F]);
        buffer[3] ^= lookup(t4, buffer[0x0C]);

        key_data[0] = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);

        key_schedule[round][1] = key_data[1];
        key_data[1] ^= key_data[0];
        buffer[4..8].copy_from_slice(&key_data[1].to_le_bytes());

        key_schedule[round][2] = key_data[2];
        key_data[2] ^= key_data[1];
        buffer[8..12].copy_from_slice(&key_data[2].to_le_bytes());

        key_schedule[round][3] = key_data[3];
        key_data[3] ^= key_data[2];
        buffer[12..16].copy_from_slice(&key_data[3].to_le_bytes());
    }
}

fn table_index(i: usize) -> usize {
    ((31 * i) % 0x28) << 8
}

fn lookup(table_offset: usize, idx: u8) -> u8 {
    let offset = table_offset + idx as usize;
    if offset < omghax_const::TABLE_S1.len() {
        omghax_const::TABLE_S1[offset]
    } else {
        0
    }
}

// ── Session key generation ───────────────────────────────────

/// Generate session key from default SAP and the key message.
/// Full translation of generate_session_key from OmgHax.java.
pub fn generate_session_key(old_sap: &[u8; 320], message_in: &[u8], session_key: &mut [u8; 16]) {
    let mut decrypted_message = [0u8; 128];
    let mut new_sap = [0u8; 320];

    // Step 1: Decrypt the message
    decrypt_message(message_in, &mut decrypted_message);

    // Step 2: Build new SAP
    new_sap[0..0x11].copy_from_slice(&omghax_const::STATIC_SOURCE_1);
    new_sap[0x11..0x91].copy_from_slice(&decrypted_message[..0x80]);
    new_sap[0x91..0x111].copy_from_slice(&old_sap[0x80..0x100]);
    new_sap[0x111..0x140].copy_from_slice(&omghax_const::STATIC_SOURCE_2);

    // Step 3: Copy initial session key
    session_key.copy_from_slice(&omghax_const::INITIAL_SESSION_KEY);

    // Step 4: 5 rounds of ModifiedMD5 + SapHash
    for round in 0..5 {
        let base_start = round * 64;
        let base_end = base_start + 64;
        if base_end > new_sap.len() {
            break;
        }
        let base = &new_sap[base_start..base_end];

        let md5 = modified_md5_stub(base, session_key);
        sap_hash_stub(base, session_key);

        // sessionKey[i] += md5[i] (as little-endian u32)
        for i in 0..4 {
            let sk = u32::from_le_bytes([
                session_key[i*4], session_key[i*4+1], session_key[i*4+2], session_key[i*4+3],
            ]);
            let md = u32::from_le_bytes([
                md5[i*4], md5[i*4+1], md5[i*4+2], md5[i*4+3],
            ]);
            let sum = sk.wrapping_add(md);
            session_key[i*4..i*4+4].copy_from_slice(&sum.to_le_bytes());
        }
    }

    // Step 5: Byte-swap each 4-byte word (reverse endianness)
    for i in (0..16).step_by(4) {
        session_key.swap(i, i + 3);
        session_key.swap(i + 1, i + 2);
    }

    // Step 6: XOR with 121
    for i in 0..16 {
        session_key[i] ^= 121;
    }
}

/// Wrapper for ModifiedMD5.modified_md5(base, sessionKey, md5).
/// Takes a variable-length base slice, extracts first 64 bytes.
fn modified_md5_stub(base: &[u8], session_key: &[u8; 16]) -> [u8; 16] {
    let mut block = [0u8; 64];
    let len = base.len().min(64);
    block[..len].copy_from_slice(&base[..len]);
    let mut out = [0u8; 16];
    super::modified_md5::modified_md5(&block, session_key, &mut out);
    out
}

/// Wrapper for SapHash.sap_hash(blockIn, keyOut).
fn sap_hash_stub(block_in: &[u8], key_out: &mut [u8; 16]) {
    let mut block = [0u8; 64];
    let len = block_in.len().min(64);
    block[..len].copy_from_slice(&block_in[..len]);
    super::sap_hash::sap_hash_impl(&block, key_out);
}

// ── Message decryption ───────────────────────────────────────

/// Decrypt the 164-byte key message into 128 bytes of plaintext.
/// Translation of decryptMessage from OmgHax.java.
pub fn decrypt_message(message_in: &[u8], decrypted_message: &mut [u8; 128]) {
    let mode = if message_in.len() > 12 {
        message_in[12] as usize
    } else {
        0
    };

    let mut buffer = [0u8; 16];

    for i in 0..8 {
        // Copy the nth block into buffer (in reverse for mode 3)
        for j in 0..16 {
            if mode == 3 {
                buffer[j] = message_in[(0x80 - 0x10 * i) + j];
            } else {
                buffer[j] = message_in[(0x10 * (i + 1)) + j];
            }
        }

        // 9 rounds of S-Box permutation using message_table_index
        for j in 0..9 {
            let base = 0x80 - 0x10 * j;

            buffer[0x0] = message_table_lookup(base + 0x0, buffer[0x0], mode);
            buffer[0x4] = message_table_lookup(base + 0x4, buffer[0x4], mode);
            buffer[0x8] = message_table_lookup(base + 0x8, buffer[0x8], mode);
            buffer[0xC] = message_table_lookup(base + 0xC, buffer[0xC], mode);

            let tmp = buffer[0x0D];
            buffer[0xD] = message_table_lookup(base + 0xD, buffer[0x9], mode);
            buffer[0x9] = message_table_lookup(base + 0x9, buffer[0x5], mode);
            buffer[0x5] = message_table_lookup(base + 0x5, buffer[0x1], mode);
            buffer[0x1] = message_table_lookup(base + 0x1, tmp, mode);

            let tmp = buffer[0x02];
            buffer[0x2] = message_table_lookup(base + 0x2, buffer[0xA], mode);
            buffer[0xA] = message_table_lookup(base + 0xA, tmp, mode);
            let tmp = buffer[0x06];
            buffer[0x6] = message_table_lookup(base + 0x6, buffer[0xE], mode);
            buffer[0xE] = message_table_lookup(base + 0xE, tmp, mode);

            let tmp = buffer[0x3];
            buffer[0x3] = message_table_lookup(base + 0x3, buffer[0x7], mode);
            buffer[0x7] = message_table_lookup(base + 0x7, buffer[0xB], mode);
            buffer[0xB] = message_table_lookup(base + 0xB, buffer[0xF], mode);
            buffer[0xF] = message_table_lookup(base + 0xF, tmp, mode);

            // table_s9 T-table substitution (little-endian word XOR)
            let w0 = omghax_const::TABLE_S9[0x000 + buffer[0x0] as usize]
                ^ omghax_const::TABLE_S9[0x100 + buffer[0x1] as usize]
                ^ omghax_const::TABLE_S9[0x200 + buffer[0x2] as usize]
                ^ omghax_const::TABLE_S9[0x300 + buffer[0x3] as usize];
            let w1 = omghax_const::TABLE_S9[0x000 + buffer[0x4] as usize]
                ^ omghax_const::TABLE_S9[0x100 + buffer[0x5] as usize]
                ^ omghax_const::TABLE_S9[0x200 + buffer[0x6] as usize]
                ^ omghax_const::TABLE_S9[0x300 + buffer[0x7] as usize];
            let w2 = omghax_const::TABLE_S9[0x000 + buffer[0x8] as usize]
                ^ omghax_const::TABLE_S9[0x100 + buffer[0x9] as usize]
                ^ omghax_const::TABLE_S9[0x200 + buffer[0xA] as usize]
                ^ omghax_const::TABLE_S9[0x300 + buffer[0xB] as usize];
            let w3 = omghax_const::TABLE_S9[0x000 + buffer[0xC] as usize]
                ^ omghax_const::TABLE_S9[0x100 + buffer[0xD] as usize]
                ^ omghax_const::TABLE_S9[0x200 + buffer[0xE] as usize]
                ^ omghax_const::TABLE_S9[0x300 + buffer[0xF] as usize];

            buffer[0..4].copy_from_slice(&w0.to_le_bytes());
            buffer[4..8].copy_from_slice(&w1.to_le_bytes());
            buffer[8..12].copy_from_slice(&w2.to_le_bytes());
            buffer[12..16].copy_from_slice(&w3.to_le_bytes());
        }

        // table_s10 permutation
        let s10 = &omghax_const::TABLE_S10;
        buffer[0x0] = s10[(0x0 << 8) + buffer[0x0] as usize];
        buffer[0x4] = s10[(0x4 << 8) + buffer[0x4] as usize];
        buffer[0x8] = s10[(0x8 << 8) + buffer[0x8] as usize];
        buffer[0xC] = s10[(0xC << 8) + buffer[0xC] as usize];

        let tmp = buffer[0xD];
        buffer[0xD] = s10[(0xD << 8) + buffer[0x9] as usize];
        buffer[0x9] = s10[(0x9 << 8) + buffer[0x5] as usize];
        buffer[0x5] = s10[(0x5 << 8) + buffer[0x1] as usize];
        buffer[0x1] = s10[(0x1 << 8) + tmp as usize];

        let tmp = buffer[0x2];
        buffer[0x2] = s10[(0x2 << 8) + buffer[0xA] as usize];
        buffer[0xA] = s10[(0xA << 8) + tmp as usize];
        let tmp = buffer[0x6];
        buffer[0x6] = s10[(0x6 << 8) + buffer[0xE] as usize];
        buffer[0xE] = s10[(0xE << 8) + tmp as usize];

        let tmp = buffer[0x3];
        buffer[0x3] = s10[(0x3 << 8) + buffer[0x7] as usize];
        buffer[0x7] = s10[(0x7 << 8) + buffer[0xB] as usize];
        buffer[0xB] = s10[(0xB << 8) + buffer[0xF] as usize];
        buffer[0xF] = s10[(0xF << 8) + tmp as usize];

        // XOR with previous block or IV
        if mode == 2 || mode == 1 || mode == 0 {
            let xor_src: [u8; 16] = if i > 0 {
                let start = 0x10 * i;
                let mut src = [0u8; 16];
                if start + 16 <= message_in.len() {
                    src.copy_from_slice(&message_in[start..start + 16]);
                }
                src
            } else {
                omghax_const::MESSAGE_IV[mode]
            };
            let dest_start = 0x10 * i;
            for k in 0..16 {
                decrypted_message[dest_start + k] = buffer[k] ^ xor_src[k];
            }
        } else {
            // mode 3: reverse order
            let xor_src: [u8; 16] = if i < 7 {
                let start = 0x70 - 0x10 * i;
                let mut src = [0u8; 16];
                if start + 16 <= message_in.len() {
                    src.copy_from_slice(&message_in[start..start + 16]);
                }
                src
            } else {
                omghax_const::MESSAGE_IV[mode]
            };
            let dest_start = 0x70 - 0x10 * i;
            for k in 0..16 {
                decrypted_message[dest_start + k] = buffer[k] ^ xor_src[k];
            }
        }
    }
}

fn message_table_index(i: usize) -> usize {
    (97 * i % 144) << 8
}

fn message_table_lookup(base: usize, idx: u8, mode: usize) -> u8 {
    let table_offset = message_table_index(base);
    let s2 = &omghax_const::TABLE_S2;
    let mk = &omghax_const::MESSAGE_KEY[mode];

    let val = if table_offset + (idx as usize) < s2.len() {
        s2[table_offset + idx as usize]
    } else {
        idx
    };

    let key_byte = if base < mk.len() { mk[base] } else { 0 };
    val ^ key_byte
}
