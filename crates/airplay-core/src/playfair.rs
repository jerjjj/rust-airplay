//! FairPlay PlayFair helper primitives.
//!
//! The implementation follows UxPlay's table and byte-order conventions while
//! keeping all memory access bounded and explicit in Rust.

#![allow(dead_code)]

const SHIFT: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const K: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// Computes UxPlay's modified MD5-like compression over one 64-byte block.
///
/// The block words are read as big-endian message words, while the 16-byte
/// chaining key and output use native little-endian words as in the original
/// implementation on supported AirPlay hosts.
pub(crate) fn modified_md5(block: &[u8; 64], key: &[u8; 16]) -> [u8; 16] {
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_le_bytes(chunk.try_into().unwrap());
    }

    let key_words = std::array::from_fn::<_, 4, _>(|index| {
        u32::from_le_bytes(key[index * 4..index * 4 + 4].try_into().unwrap())
    });
    let mut a = key_words[0];
    let mut b = key_words[1];
    let mut c = key_words[2];
    let mut d = key_words[3];

    for i in 0..64 {
        let j = match i {
            0..=15 => i,
            16..=31 => (5 * i + 1) % 16,
            32..=47 => (3 * i + 5) % 16,
            _ => (7 * i) % 16,
        };
        let input = u32::from_be_bytes(words[j].to_le_bytes());
        let function = match i {
            0..=15 => (b & c) | (!b & d),
            16..=31 => (b & d) | (c & !d),
            32..=47 => b ^ c ^ d,
            _ => c ^ (b | !d),
        };
        let z = a
            .wrapping_add(input)
            .wrapping_add(K[i])
            .wrapping_add(function)
            .rotate_left(SHIFT[i]);
        let z = z.wrapping_add(b);
        (a, b, c, d) = (d, z, b, c);

        if i == 31 {
            for (left, right) in [(a & 15, b & 15), (c & 15, d & 15)] {
                words.swap(left as usize, right as usize);
            }
            for shift in [4, 8, 12] {
                let left = ((a >> shift) & 15) as usize;
                let right = ((b >> shift) & 15) as usize;
                words.swap(left, right);
            }
        }
    }

    let mut output = [0u8; 16];
    for (index, value) in [
        key_words[0].wrapping_add(a),
        key_words[1].wrapping_add(b),
        key_words[2].wrapping_add(c),
        key_words[3].wrapping_add(d),
    ]
    .into_iter()
    .enumerate()
    {
        output[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    output
}

fn sap_scramble(input: &[u8; 64]) -> [u8; 210] {
    let mut buffer = [0u8; 210];
    for index in 0..210 {
        let word_index = (index % 64) / 4;
        let word = u32::from_le_bytes(
            input[word_index * 4..word_index * 4 + 4]
                .try_into()
                .unwrap(),
        );
        let shift = (3 - (index % 4)) * 8;
        buffer[index] = ((word >> shift) & 0xff) as u8;
    }
    for index in 0..840 {
        let x = buffer[(index as u32).wrapping_sub(155) as usize % 210];
        let y = buffer[(index as u32).wrapping_sub(57) as usize % 210];
        let z = buffer[(index as u32).wrapping_sub(13) as usize % 210];
        let w = buffer[index % 210];
        buffer[index % 210] = y
            .rotate_left(5)
            .wrapping_add(z.rotate_left(3) ^ w)
            .wrapping_sub(x.rotate_left(7));
    }
    buffer
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sap_scramble_matches_upstream_vector() {
        let input = std::array::from_fn::<_, 64, _>(|index| {
            (index as u8).wrapping_mul(37).wrapping_add(11)
        });
        assert_eq!(
            sap_scramble(&input),
            [
                0x92, 0x08, 0x0a, 0x6f, 0x21, 0x72, 0x21, 0x98, 0x96, 0xf1, 0x13, 0x5c, 0x87, 0xf4,
                0x67, 0xda, 0x08, 0xcd, 0x95, 0xfb, 0x04, 0x61, 0x1a, 0xbe, 0x16, 0x23, 0xfa, 0x5f,
                0xc3, 0xb4, 0x65, 0x12, 0x49, 0xc4, 0x94, 0xca, 0x63, 0xfb, 0x5b, 0xe3, 0x94, 0x45,
                0x24, 0x15, 0x6b, 0xe1, 0xdb, 0xec, 0x79, 0x79, 0xdd, 0xb3, 0x8e, 0xe2, 0x77, 0xa4,
                0x5f, 0x7e, 0x0a, 0xf0, 0xe0, 0xd7, 0x96, 0x39, 0x5a, 0xf2, 0x28, 0xb7, 0x05, 0x72,
                0x0c, 0x2a, 0xb3, 0xa8, 0x8f, 0x5b, 0x15, 0x2e, 0x84, 0x08, 0xf3, 0x69, 0xc9, 0xa6,
                0xff, 0xcd, 0x76, 0xb5, 0xa6, 0x65, 0xc5, 0x4c, 0x48, 0x8f, 0x16, 0x46, 0xec, 0x11,
                0x44, 0xce, 0x90, 0x46, 0xd6, 0x76, 0x43, 0x08, 0x36, 0x17, 0x8c, 0xcf, 0x00, 0x52,
                0xf7, 0x59, 0x00, 0x64, 0x99, 0x4f, 0x98, 0x98, 0x04, 0x3f, 0x54, 0xe0, 0x53, 0x28,
                0x6c, 0xfb, 0xd9, 0x83, 0x04, 0x3c, 0x53, 0xc2, 0x24, 0xea, 0x0c, 0x7e, 0x68, 0xee,
                0x0b, 0xfb, 0x70, 0xc5, 0x7f, 0xe1, 0x13, 0x09, 0x4b, 0xbe, 0x14, 0x93, 0x27, 0xe2,
                0x27, 0x7b, 0x97, 0x37, 0x3d, 0xbc, 0x50, 0x53, 0x8d, 0x28, 0xf4, 0x89, 0x07, 0x0c,
                0x6e, 0x2a, 0x99, 0x8d, 0xeb, 0x43, 0xce, 0xef, 0x35, 0xea, 0x6f, 0x33, 0x93, 0xeb,
                0x5a, 0x32, 0x11, 0x72, 0xbe, 0x8c, 0x7a, 0xd9, 0x86, 0xdc, 0x27, 0x3b, 0x8e, 0xcd,
                0x97, 0x33, 0x0e, 0x9c, 0x6b, 0xaa, 0x99, 0x00, 0xab, 0x0a, 0x93, 0x12, 0xa8, 0x7a,
            ]
        );
    }

    #[test]
    fn modified_md5_matches_upstream_vector() {
        let block = std::array::from_fn::<_, 64, _>(|index| {
            (index as u8).wrapping_mul(37).wrapping_add(11)
        });
        let key = std::array::from_fn::<_, 16, _>(|index| {
            0xf0u8.wrapping_sub((index as u8).wrapping_mul(13))
        });
        assert_eq!(
            modified_md5(&block, &key),
            [
                0x71, 0xd2, 0x2f, 0xbd, 0x5c, 0x67, 0x86, 0xbe, 0x4e, 0x38, 0xcd, 0x0e, 0x05, 0xed,
                0x08, 0xe9,
            ]
        );
    }
}
