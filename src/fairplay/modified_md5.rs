//! Modified MD5 hash — translation from ModifiedMD5.java.
//!
//! Modified version of MD5 used by FairPlay's OmgHax key derivation.
//! Differences from standard MD5:
//!   - Swap operations at round 31 (byte-level permutation of the input block)
//!   - Output is accumulated (added to input key words) rather than replacing them

/// Standard MD5 T-constants: floor(2^32 * abs(sin(i+1)))
const T: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
    0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
    0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
    0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// MD5 shift amounts
const SHIFT: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Modified MD5: 64 rounds of digest computation on a 64-byte block.
///
/// `block_in`: 64-byte input block (modified in-place at round 31!)
/// `key_in`:   16-byte key (4 little-endian u32 words used as initial state)
/// `key_out`:  16-byte output (accumulated result)
pub fn modified_md5(block_in: &[u8; 64], key_in: &[u8; 16], key_out: &mut [u8; 16]) {
    let mut block = *block_in;

    let mut a = u32::from_le_bytes([key_in[0], key_in[1], key_in[2], key_in[3]]);
    let mut b = u32::from_le_bytes([key_in[4], key_in[5], key_in[6], key_in[7]]);
    let mut c = u32::from_le_bytes([key_in[8], key_in[9], key_in[10], key_in[11]]);
    let mut d = u32::from_le_bytes([key_in[12], key_in[13], key_in[14], key_in[15]]);

    let orig_a = a;
    let orig_b = b;
    let orig_c = c;
    let orig_d = d;

    for i in 0..64usize {
        let j = if i < 16 {
            i
        } else if i < 32 {
            (5 * i + 1) % 16
        } else if i < 48 {
            (3 * i + 5) % 16
        } else {
            (7 * i) % 16
        };

        let input = u32::from_be_bytes([
            block[4 * j],
            block[4 * j + 1],
            block[4 * j + 2],
            block[4 * j + 3],
        ]);

        let mut z = a.wrapping_add(input).wrapping_add(T[i]);

        if i < 16 {
            z = rol(z.wrapping_add(f(b, c, d)), SHIFT[i]);
        } else if i < 32 {
            z = rol(z.wrapping_add(g(b, c, d)), SHIFT[i]);
        } else if i < 48 {
            z = rol(z.wrapping_add(h(b, c, d)), SHIFT[i]);
        } else {
            z = rol(z.wrapping_add(i_round(b, c, d)), SHIFT[i]);
        }

        z = z.wrapping_add(b);

        let tmp = d;
        d = c;
        c = b;
        b = z;
        a = tmp;

        // Swap operations at round 31
        if i == 31 {
            swap_words(&mut block, (a & 15) as usize, (b & 15) as usize);
            swap_words(&mut block, (c & 15) as usize, (d & 15) as usize);
            swap_words(&mut block, ((a >> 4) & 15) as usize, ((b >> 4) & 15) as usize);
            swap_words(&mut block, ((a >> 8) & 15) as usize, ((b >> 8) & 15) as usize);
            swap_words(&mut block, ((a >> 12) & 15) as usize, ((b >> 12) & 15) as usize);
        }
    }

    let out_a = orig_a.wrapping_add(a);
    let out_b = orig_b.wrapping_add(b);
    let out_c = orig_c.wrapping_add(c);
    let out_d = orig_d.wrapping_add(d);

    key_out[0..4].copy_from_slice(&out_a.to_le_bytes());
    key_out[4..8].copy_from_slice(&out_b.to_le_bytes());
    key_out[8..12].copy_from_slice(&out_c.to_le_bytes());
    key_out[12..16].copy_from_slice(&out_d.to_le_bytes());
}

fn f(b: u32, c: u32, d: u32) -> u32 { (b & c) | ((!b) & d) }
fn g(b: u32, c: u32, d: u32) -> u32 { (b & d) | (c & (!d)) }
fn h(b: u32, c: u32, d: u32) -> u32 { b ^ c ^ d }
fn i_round(b: u32, c: u32, d: u32) -> u32 { c ^ (b | (!d)) }

fn rol(input: u32, count: u32) -> u32 {
    input.rotate_left(count)
}

/// Swap 4-byte words at indices idx_a and idx_b (indices are word indices, 0..15).
fn swap_words(arr: &mut [u8; 64], idx_a: usize, idx_b: usize) {
    let a_off = 4 * idx_a;
    let b_off = 4 * idx_b;
    if a_off + 4 > 64 || b_off + 4 > 64 {
        return;
    }
    let a_val = u32::from_le_bytes([arr[a_off], arr[a_off+1], arr[a_off+2], arr[a_off+3]]);
    let b_val = u32::from_le_bytes([arr[b_off], arr[b_off+1], arr[b_off+2], arr[b_off+3]]);
    // Java: wrap.putInt(idxB, a); wrap.putInt(idxA, b);
    arr[b_off..b_off+4].copy_from_slice(&a_val.to_le_bytes());
    arr[a_off..a_off+4].copy_from_slice(&b_val.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modified_md5_basic() {
        let block = [0u8; 64];
        let key_in = [0u8; 16];
        let mut key_out = [0u8; 16];
        modified_md5(&block, &key_in, &mut key_out);
        // Output should be non-zero
        assert!(key_out != [0u8; 16]);
    }
}
