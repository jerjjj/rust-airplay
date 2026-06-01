//! Pure Rust implementation of HandGarble (garble function).
//! Translated from UxPlay's hand_garble.c.

fn weird_ror8(input: u8, count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    ((input >> count) as u32 & 0xff) | ((input as u32) << (8 - count))
}

fn weird_rol8(input: u8, count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    ((input as u32) << count) | ((input as u32) >> (8 - count))
}

fn weird_rol32(input: u8, count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    ((input as u32) << count) ^ ((input as u32) >> (8 - count))
}

pub fn garble(
    buffer0: &mut [u8; 20],
    buffer1: &mut [u8; 210],
    buffer2: &mut [u8; 35],
    buffer3: &mut [u8; 132],
    buffer4: &mut [u8; 21],
) {
    let mut tmp: u32;
    let mut tmp2: u32;
    let tmp3: u32;
    let mut a: u32;
    let mut b: u32;
    let mut c: u32;
    let mut d: u32;
    let mut e: u32;
    let mut f: u32;
    let mut g: u32;
    let mut h: u32;
    let mut j: u32;
    let mut k: u32;
    let mut m: u32;
    let mut r: u32;
    let mut s: u32;
    let mut t: u32;
    let mut u: u32;
    let mut v: u32;
    let mut w: u32;
    let mut x: u32;
    let mut y: u32;
    let mut z: u32;

    // buffer2[12] = 0x14 + (((buffer1[64] & 92) | ((buffer1[99] / 3) & 35)) & buffer4[rol8x(buffer4[(buffer1[206] % 21)],4) % 21]);
    let rol8x_val = weird_rol8(buffer4[(buffer1[206] % 21) as usize], 4);
    buffer2[12] = 0x14 + (((buffer1[64] & 92) | ((buffer1[99] / 3) & 35)) & buffer4[(rol8x_val % 21) as usize]);

    // buffer1[4] = (buffer1[99] / 5) * (buffer1[99] / 5) * 2;
    buffer1[4] = (buffer1[99] / 5).wrapping_mul(buffer1[99] / 5).wrapping_mul(2);

    // buffer2[34] = 0xb8;
    buffer2[34] = 0xb8;

    // buffer1[153] ^= (buffer2[buffer1[203] % 35] * buffer2[buffer1[203] % 35] * buffer1[190]);
    buffer1[153] ^= buffer2[(buffer1[203] % 35) as usize].wrapping_mul(buffer2[(buffer1[203] % 35) as usize]).wrapping_mul(buffer1[190]);

    // buffer0[3] -= (((buffer4[buffer1[205] % 21]>>1) & 80) | 0xe6440);
    buffer0[3] = buffer0[3].wrapping_sub((((buffer4[(buffer1[205] % 21) as usize] >> 1) & 80) | 0x40) as u8);

    // buffer0[16] = 0x93;
    buffer0[16] = 0x93;

    // buffer0[13] = 0x62;
    buffer0[13] = 0x62;

    // buffer1[33] -= (buffer4[buffer1[36] % 21] & 0xf6);
    buffer1[33] = buffer1[33].wrapping_sub(buffer4[(buffer1[36] % 21) as usize] & 0xf6);

    // tmp2 = buffer2[buffer1[67] % 35];
    tmp2 = buffer2[(buffer1[67] % 35) as usize] as u32;
    // buffer2[12] = 0x07;
    buffer2[12] = 0x07;

    // tmp = buffer0[buffer1[181] % 20];
    tmp = buffer0[(buffer1[181] % 20) as usize] as u32;
    // buffer1[2] -= 3136;
    buffer1[2] = buffer1[2].wrapping_sub((3136 % 256) as u8);

    // buffer0[19] = buffer4[buffer1[58] % 21];
    buffer0[19] = buffer4[(buffer1[58] % 21) as usize];

    // buffer3[0] = 92 - buffer2[buffer1[32] % 35];
    buffer3[0] = 92u8.wrapping_sub(buffer2[(buffer1[32] % 35) as usize]);

    // buffer3[4] = buffer2[buffer1[15] % 35] + 0x9e;
    buffer3[4] = buffer2[(buffer1[15] % 35) as usize].wrapping_add(0x9e);

    // buffer1[34] += (buffer4[((buffer2[buffer1[15] % 35] + 0x9e) & 0xff) % 21] / 5);
    buffer1[34] = buffer1[34].wrapping_add(buffer4[((buffer3[4] as u32) % 21) as usize] / 5);

    // buffer0[19] += 0xfffffee6 - ((buffer0[buffer3[4] % 20]>>1) & 102);
    buffer0[19] = buffer0[19].wrapping_add((0xfffffee6u32.wrapping_sub(((buffer0[(buffer3[4] % 20) as usize] as u32) >> 1) & 102)) as u8);

    // buffer1[15] = (3*(((buffer1[72] >> (buffer4[buffer1[190] % 21] & 7)) ^ (buffer1[72] << ((7 - (buffer4[buffer1[190] % 21]-1)&7)))) - (3*buffer4[buffer1[126] % 21]))) ^ buffer1[15];
    let shift1 = buffer4[(buffer1[190] % 21) as usize] & 7;
    let shift2 = (7 - ((buffer4[(buffer1[190] % 21) as usize].wrapping_sub(1)) & 7)) & 7;
    let val1 = (buffer1[72] as u32) >> shift1;
    let val2 = (buffer1[72] as u32) << shift2;
    buffer1[15] ^= (3u32.wrapping_mul(val1 ^ val2).wrapping_sub(3u32.wrapping_mul(buffer4[(buffer1[126] % 21) as usize] as u32))) as u8;

    // buffer0[15] ^= buffer2[buffer1[181] % 35] * buffer2[buffer1[181] % 35] * buffer2[buffer1[181] % 35];
    let idx = (buffer1[181] % 35) as usize;
    buffer0[15] ^= buffer2[idx].wrapping_mul(buffer2[idx]).wrapping_mul(buffer2[idx]);

    // buffer2[4] ^= buffer1[202]/3;
    buffer2[4] ^= buffer1[202] / 3;

    // A = 92 - buffer0[buffer3[0] % 20];
    a = 92u32.wrapping_sub(buffer0[(buffer3[0] % 20) as usize] as u32);
    // E = (A & 0xc6) | (~buffer1[105] & 0xc6) | (A & (~buffer1[105]));
    e = (a & 0xc6) | ((!buffer1[105] as u32) & 0xc6) | (a & (!buffer1[105] as u32));
    // buffer2[1] += (E*E*E);
    buffer2[1] = buffer2[1].wrapping_add((e.wrapping_mul(e).wrapping_mul(e)) as u8);

    // buffer0[19] ^= ((224 | (buffer4[buffer1[92] % 21] & 27)) * buffer2[buffer1[41] % 35]) / 3;
    buffer0[19] ^= (((224 | (buffer4[(buffer1[92] % 21) as usize] & 27)) as u32).wrapping_mul(buffer2[(buffer1[41] % 35) as usize] as u32) / 3) as u8;

    // buffer1[140] += weird_ror8(92, buffer1[5] & 7);
    buffer1[140] = buffer1[140].wrapping_add(weird_ror8(92, (buffer1[5] & 7) as u32) as u8);

    // buffer2[12] += ((((~buffer1[4]) ^ buffer2[buffer1[12] % 35]) | buffer1[182]) & 192) | (((~buffer1[4]) ^ buffer2[buffer1[12] % 35]) & buffer1[182]);
    let expr = (!buffer1[4] as u32) ^ (buffer2[(buffer1[12] % 35) as usize] as u32);
    buffer2[12] = buffer2[12].wrapping_add((((expr | buffer1[182] as u32) & 192) | (expr & buffer1[182] as u32)) as u8);

    // buffer1[36] += 125;
    buffer1[36] = buffer1[36].wrapping_add(125);

    // buffer1[124] = rol8x((((74 & buffer1[138]) | ((74 | buffer1[138]) & buffer0[15])) & buffer0[buffer1[43] % 20]) | (((74 & buffer1[138]) | ((74 | buffer1[138]) & buffer0[15]) | buffer0[buffer1[43] % 20]) & 95), 4);
    let inner = (74 & buffer1[138]) | ((74 | buffer1[138]) & buffer0[15]);
    let outer = (inner & buffer0[(buffer1[43] % 20) as usize]) | ((inner | buffer0[(buffer1[43] % 20) as usize]) & 95);
    buffer1[124] = weird_rol8(outer, 4) as u8;

    // buffer3[8] = ((((buffer0[buffer3[4] % 20] & 95)) & ((buffer4[buffer1[68] % 21] & 46) << 1)) | 16) ^ 92;
    buffer3[8] = (((buffer0[(buffer3[4] % 20) as usize] & 95) & ((buffer4[(buffer1[68] % 21) as usize] & 46) << 1)) | 16) ^ 92;

    // A = buffer1[177] + buffer4[buffer1[79] % 21];
    a = buffer1[177] as u32 + buffer4[(buffer1[79] % 21) as usize] as u32;
    // D = (((A >> 1) | ((3 * buffer1[148]) / 5)) & buffer2[1]) | ((A >> 1) & ((3 * buffer1[148])/5));
    d = (((a >> 1) | ((3u32.wrapping_mul(buffer1[148] as u32)) / 5)) & buffer2[1] as u32) | ((a >> 1) & ((3u32.wrapping_mul(buffer1[148] as u32)) / 5));
    // buffer3[12] = ((-34 - D));
    buffer3[12] = (34u32.wrapping_add(d).wrapping_neg()) as u8;

    // A = 8 - ((buffer2[22] & 7));
    a = 8 - ((buffer2[22] & 7) as u32);
    // B = (buffer1[33] >> (A & 7));
    b = (buffer1[33] as u32) >> (a & 7);
    // C = buffer1[33] << (buffer2[22] & 7);
    c = (buffer1[33] as u32) << ((buffer2[22] & 7) as u32);
    // buffer2[16] += ((buffer2[buffer3[0] % 35] & 159) | buffer0[buffer3[4] % 20] | 8) - ((B^C) | 128);
    buffer2[16] = buffer2[16].wrapping_add(
        (((buffer2[(buffer3[0] % 35) as usize] & 159) | buffer0[(buffer3[4] % 20) as usize] | 8) as u32).wrapping_sub((b ^ c) | 128) as u8
    );

    // buffer0[14] ^= buffer2[buffer3[12] % 35];
    buffer0[14] ^= buffer2[(buffer3[12] % 35) as usize];

    // Monster goes here
    a = weird_rol8(buffer4[(buffer0[(buffer1[201] % 20) as usize] % 21) as usize], ((buffer2[(buffer1[112] % 35) as usize] << 1) & 7) as u32);
    d = (buffer0[(buffer1[208] % 20) as usize] as u32 & 131) | (buffer0[(buffer1[164] % 20) as usize] as u32 & 124);
    buffer1[19] = buffer1[19].wrapping_add(((a & (d / 5)) | ((a | (d / 5)) & 37)) as u8);

    // buffer2[8] = weird_ror8(140, ((buffer4[buffer1[45] % 21] + 92) * (buffer4[buffer1[45] % 21] + 92)) & 7);
    let val = buffer4[(buffer1[45] % 21) as usize] as u32 + 92;
    buffer2[8] = weird_ror8(140, val.wrapping_mul(val) & 7) as u8;

    // buffer1[190] = 56;
    buffer1[190] = 56;

    // buffer2[8] ^= buffer3[0];
    buffer2[8] ^= buffer3[0];

    // buffer1[53] = ~((buffer0[buffer1[83] % 20] | 204)/5);
    buffer1[53] = !((buffer0[(buffer1[83] % 20) as usize] | 204) / 5);

    // buffer0[13] += buffer0[buffer1[41] % 20];
    buffer0[13] = buffer0[13].wrapping_add(buffer0[(buffer1[41] % 20) as usize]);

    // buffer0[10] = ((buffer2[buffer3[0] % 35] & buffer1[2]) | ((buffer2[buffer3[0] % 35] | buffer1[2]) & buffer3[12])) / 15;
    buffer0[10] = ((buffer2[(buffer3[0] % 35) as usize] & buffer1[2]) | ((buffer2[(buffer3[0] % 35) as usize] | buffer1[2]) & buffer3[12])) / 15;

    // A = (((56 | (buffer4[buffer1[2] % 21] & 68)) | buffer2[buffer3[8] % 35]) & 42) | (((buffer4[buffer1[2] % 21] & 68) | 56) & buffer2[buffer3[8] % 35]);
    a = (((56 | (buffer4[(buffer1[2] % 21) as usize] & 68)) | buffer2[(buffer3[8] % 35) as usize]) & 42) as u32 | 
        ((((buffer4[(buffer1[2] % 21) as usize] & 68) | 56) & buffer2[(buffer3[8] % 35) as usize]) as u32);
    // buffer3[16] = (A*A) + 110;
    buffer3[16] = (a.wrapping_mul(a) + 110) as u8;

    // buffer3[20] = 202 - buffer3[16];
    buffer3[20] = 202u8.wrapping_sub(buffer3[16]);

    // buffer3[24] = buffer1[151];
    buffer3[24] = buffer1[151];

    // buffer2[13] ^= buffer4[buffer3[0] % 21];
    buffer2[13] ^= buffer4[(buffer3[0] % 21) as usize];

    // B = ((buffer2[buffer1[179] % 35] - 38) & 177) | (buffer3[12] & 177);
    b = ((buffer2[(buffer1[179] % 35) as usize] as u32).wrapping_sub(38) & 177) | (buffer3[12] as u32 & 177);
    // C = ((buffer2[buffer1[179] % 35] - 38)) & buffer3[12];
    c = ((buffer2[(buffer1[179] % 35) as usize] as u32).wrapping_sub(38)) & (buffer3[12] as u32);
    // buffer3[28] = 30 + ((B | C) * (B | C));
    buffer3[28] = (30 + (b | c).wrapping_mul(b | c)) as u8;

    // buffer3[32] = buffer3[28] + 62;
    buffer3[32] = buffer3[28].wrapping_add(62);

    // eek
    a = ((buffer3[20] as u32 + (buffer3[0] as u32 & 74)) | !(buffer4[(buffer3[0] % 21) as usize] as u32)) & 121;
    b = (buffer3[20] as u32 + (buffer3[0] as u32 & 74)) & !(buffer4[(buffer3[0] % 21) as usize] as u32);
    tmp3 = a | b;
    c = (((tmp3 ^ 0xffffffa6) | buffer3[0] as u32) & 4) | ((tmp3 ^ 0xffffffa6) & buffer3[0] as u32);
    buffer1[47] ^= (buffer2[(buffer1[89] % 35) as usize] as u32 + c) as u8;

    // buffer3[36] = ((rol8((tmp & 179) + 68, 2) & buffer0[3]) | (tmp2 & ~buffer0[3])) - 15;
    let rol8_val = weird_rol8(((tmp & 179) + 68) as u8, 2);
    buffer3[36] = (((rol8_val & buffer0[3] as u32) | (tmp2 & !buffer0[3] as u32)).wrapping_sub(15)) as u8;

    // buffer1[123] ^= 221;
    buffer1[123] ^= 221;

    // A = ((buffer4[buffer3[0] % 21]) / 3) - buffer2[buffer3[4] % 35];
    a = (buffer4[(buffer3[0] % 21) as usize] as u32 / 3).wrapping_sub(buffer2[(buffer3[4] % 35) as usize] as u32);
    // C = (((buffer3[0] & 163) + 92) & 246) | (buffer3[0] & 92);
    c = (((buffer3[0] as u32 & 163) + 92) & 246) | (buffer3[0] as u32 & 92);
    // E = ((C | buffer3[24]) & 54) | (C & buffer3[24]);
    e = ((c | buffer3[24] as u32) & 54) | (c & buffer3[24] as u32);
    // buffer3[40] = A - E;
    buffer3[40] = a.wrapping_sub(e) as u8;

    // buffer3[44] = tmp3 ^ 81 ^ (((buffer3[0] >> 1) & 101) + 26);
    buffer3[44] = (tmp3 ^ 81 ^ (((buffer3[0] as u32 >> 1) & 101) + 26)) as u8;

    // buffer3[48] = buffer2[buffer3[4] % 35] & 27;
    buffer3[48] = buffer2[(buffer3[4] % 35) as usize] & 27;
    // buffer3[52] = 27;
    buffer3[52] = 27;
    // buffer3[56] = 199;
    buffer3[56] = 199;

    // caffeine
    a = ((buffer3[40] as u32 | buffer3[24] as u32) & 177) | (buffer3[40] as u32 & buffer3[24] as u32);
    let inner1 = ((buffer4[(buffer3[0] % 21) as usize] as u32 & 177) | 176) | (buffer4[(buffer3[0] % 21) as usize] as u32 & !3);
    let inner2 = ((buffer3[40] as u32 & buffer3[24] as u32) | a) & 199;
    let inner3 = (((buffer4[(buffer3[0] % 21) as usize] as u32 & 1) + 176) | (buffer4[(buffer3[0] % 21) as usize] as u32 & !3));
    let combined = (a & inner1) | (inner2 & inner3);
    buffer3[64] = buffer3[4].wrapping_add((combined & !buffer3[52] as u32) as u8).wrapping_add(buffer3[48]);

    // buffer2[33] ^= buffer1[26];
    buffer2[33] ^= buffer1[26];

    // buffer1[106] ^= buffer3[20] ^ 133;
    buffer1[106] ^= buffer3[20] ^ 133;

    // buffer2[30] = ((buffer3[64] / 3) - (275 | (buffer3[0] & 247))) ^ buffer0[buffer1[122] % 20];
    buffer2[30] = ((buffer3[64] / 3).wrapping_sub(((19u8) | (buffer3[0] & 247)))) ^ buffer0[(buffer1[122] % 20) as usize];

    // buffer1[22] = (buffer2[buffer1[90] % 35] & 95) | 68;
    buffer1[22] = (buffer2[(buffer1[90] % 35) as usize] & 95) | 68;

    // A = (buffer4[buffer3[36] % 21] & 184) | (buffer2[buffer3[44] % 35] & ~184);
    a = (buffer4[(buffer3[36] % 21) as usize] as u32 & 184) | (buffer2[(buffer3[44] % 35) as usize] as u32 & !184);
    // buffer2[18] += ((A*A*A) >> 1);
    buffer2[18] = buffer2[18].wrapping_add((a.wrapping_mul(a).wrapping_mul(a) >> 1) as u8);

    // buffer2[5] -= buffer4[buffer1[92] % 21];
    buffer2[5] = buffer2[5].wrapping_sub(buffer4[(buffer1[92] % 21) as usize]);

    // A = (((buffer1[41] & ~24)|(buffer2[buffer1[183] % 35] & 24)) & (buffer3[16] + 53)) | (buffer3[20] & buffer2[buffer3[20] % 35]);
    a = (((buffer1[41] as u32 & !24) | (buffer2[(buffer1[183] % 35) as usize] as u32 & 24)) & (buffer3[16] as u32 + 53)) | 
        (buffer3[20] as u32 & buffer2[(buffer3[20] % 35) as usize] as u32);
    // B = (buffer1[17] & (~buffer3[44])) | (buffer0[buffer1[59] % 20] & buffer3[44]);
    b = (buffer1[17] as u32 & !buffer3[44] as u32) | (buffer0[(buffer1[59] % 20) as usize] as u32 & buffer3[44] as u32);
    // buffer2[18] ^= (A*B);
    buffer2[18] ^= (a.wrapping_mul(b)) as u8;

    // A = weird_ror8(buffer1[11], buffer2[buffer1[28] % 35] & 7) & 7;
    a = weird_ror8(buffer1[11], (buffer2[(buffer1[28] % 35) as usize] & 7) as u32) & 7;
    // B = (((buffer0[buffer1[93] % 20] & ~buffer0[14]) | (buffer0[14] & 150)) & ~28) | (buffer1[7] & 28);
    b = (((buffer0[(buffer1[93] % 20) as usize] as u32 & !buffer0[14] as u32) | (buffer0[14] as u32 & 150)) & !28) | (buffer1[7] as u32 & 28);
    // buffer2[22] = (((((B | weird_rol8(buffer2[buffer3[0] % 35], A)) & buffer2[33]) | (B & weird_rol8(buffer2[buffer3[0] % 35], A))) + 74) & 0xff);
    let rol8_val2 = weird_rol8(buffer2[(buffer3[0] % 35) as usize], a);
    buffer2[22] = ((((b | rol8_val2) & buffer2[33] as u32) | (b & rol8_val2) + 74) & 0xff) as u8;

    // A = buffer4[(buffer0[buffer1[39] % 20] ^ 217) % 21];
    a = buffer4[((buffer0[(buffer1[39] % 20) as usize] as u32 ^ 217) % 21) as usize] as u32;
    // buffer0[15] -= ((((buffer3[20] | buffer3[0]) & 214) | (buffer3[20] & buffer3[0])) & A) | ((((buffer3[20] | buffer3[0]) & 214) | (buffer3[20] & buffer3[0]) | A) & buffer3[32]);
    let combined2 = ((buffer3[20] as u32 | buffer3[0] as u32) & 214) | (buffer3[20] as u32 & buffer3[0] as u32);
    buffer0[15] = buffer0[15].wrapping_sub(((combined2 & a) | ((combined2 | a) & buffer3[32] as u32)) as u8);

    // We need to save T here, and boy is it complicated to calculate!
    // B = (((buffer2[buffer1[57] % 35] & buffer0[buffer3[64] % 20]) | ((buffer0[buffer3[64] % 20] | buffer2[buffer1[57] % 35]) & 95) | (buffer3[64] & 45) | 82) & 32);
    b = (((buffer2[(buffer1[57] % 35) as usize] as u32 & buffer0[(buffer3[64] % 20) as usize] as u32) | 
          ((buffer0[(buffer3[64] % 20) as usize] as u32 | buffer2[(buffer1[57] % 35) as usize] as u32) & 95) | 
          (buffer3[64] as u32 & 45) | 82) & 32);
    // C = ((buffer2[buffer1[57] % 35] & buffer0[buffer3[64] % 20]) | ((buffer2[buffer1[57] % 35] | buffer0[buffer3[64] % 20]) & 95)) & ((buffer3[64] & 45) | 82);
    c = ((buffer2[(buffer1[57] % 35) as usize] as u32 & buffer0[(buffer3[64] % 20) as usize] as u32) | 
         ((buffer2[(buffer1[57] % 35) as usize] as u32 | buffer0[(buffer3[64] % 20) as usize] as u32) & 95)) & 
        ((buffer3[64] as u32 & 45) | 82);
    // D = ((((buffer3[0]/3) - (buffer3[64]|buffer1[22]))) ^ (buffer3[28] + 62) ^ ((B|C)));
    d = ((buffer3[0] as u32 / 3).wrapping_sub((buffer3[64] | buffer1[22]) as u32)) ^ (buffer3[28] as u32 + 62) ^ (b | c);
    // T = buffer0[(D & 0xff) % 20];
    t = buffer0[((d & 0xff) % 20) as usize] as u32;

    // buffer3[68] = (buffer0[buffer1[99] % 20] * buffer0[buffer1[99] % 20] * buffer0[buffer1[99] % 20] * buffer0[buffer1[99] % 20]) | buffer2[buffer3[64] % 35];
    let idx2 = (buffer1[99] % 20) as usize;
    buffer3[68] = (buffer0[idx2].wrapping_mul(buffer0[idx2]).wrapping_mul(buffer0[idx2]).wrapping_mul(buffer0[idx2])) | buffer2[(buffer3[64] % 35) as usize];

    // U = buffer0[buffer1[50] % 20];
    u = buffer0[(buffer1[50] % 20) as usize] as u32;
    // W = buffer2[buffer1[138] % 35];
    w = buffer2[(buffer1[138] % 35) as usize] as u32;
    // X = buffer4[buffer1[39] % 21];
    x = buffer4[(buffer1[39] % 21) as usize] as u32;
    // Y = buffer0[buffer1[4] % 20];
    y = buffer0[(buffer1[4] % 20) as usize] as u32;
    // Z = buffer4[buffer1[202] % 21];
    z = buffer4[(buffer1[202] % 21) as usize] as u32;
    // V = buffer0[buffer1[151] % 20];
    v = buffer0[(buffer1[151] % 20) as usize] as u32;
    // S = buffer2[buffer1[14] % 35];
    s = buffer2[(buffer1[14] % 35) as usize] as u32;
    // R = buffer0[buffer1[145] % 20];
    r = buffer0[(buffer1[145] % 20) as usize] as u32;

    // A = (buffer2[buffer3[68] % 35] & buffer0[buffer1[209] % 20]) | ((buffer2[buffer3[68] % 35] | buffer0[buffer1[209] % 20]) & 24);
    a = (buffer2[(buffer3[68] % 35) as usize] as u32 & buffer0[(buffer1[209] % 20) as usize] as u32) | 
        ((buffer2[(buffer3[68] % 35) as usize] as u32 | buffer0[(buffer1[209] % 20) as usize] as u32) & 24);
    // B = weird_rol8(buffer4[buffer1[127] % 21], buffer2[buffer3[68] % 35] & 7);
    b = weird_rol8(buffer4[(buffer1[127] % 21) as usize], (buffer2[(buffer3[68] % 35) as usize] & 7) as u32);
    // C = (A & buffer0[10]) | (B & ~buffer0[10]);
    c = (a & buffer0[10] as u32) | (b & !buffer0[10] as u32);
    // D = 7 ^ (buffer4[buffer2[buffer3[36] % 35] % 21] << 1);
    d = 7 ^ ((buffer4[(buffer2[(buffer3[36] % 35) as usize] % 21) as usize] as u32) << 1);
    // buffer3[72] = (C & 71) | (D & ~71);
    buffer3[72] = ((c & 71) | (d & !71)) as u8;

    // buffer2[2] += (((buffer0[buffer3[20] % 20] << 1) & 159) | (buffer4[buffer1[190] % 21] & ~159)) & ((((buffer4[buffer3[64] % 21] & 110) | (buffer0[buffer1[25] % 20] & ~110)) & ~150) | (buffer1[25] & 150));
    let left = ((buffer0[(buffer3[20] % 20) as usize] as u32) << 1 & 159) | (buffer4[(buffer1[190] % 21) as usize] as u32 & !159);
    let right = (((buffer4[(buffer3[64] % 21) as usize] as u32 & 110) | (buffer0[(buffer1[25] % 20) as usize] as u32 & !110)) & !150) | (buffer1[25] as u32 & 150);
    buffer2[2] = buffer2[2].wrapping_add((left & right) as u8);

    // buffer2[14] -= ((buffer2[buffer3[20] % 35] & (buffer3[72] ^ buffer2[buffer1[100] % 35])) & ~34) | (buffer1[97] & 34);
    buffer2[14] = buffer2[14].wrapping_sub(
        (((buffer2[(buffer3[20] % 35) as usize] as u32 & (buffer3[72] as u32 ^ buffer2[(buffer1[100] % 35) as usize] as u32)) & !34) | (buffer1[97] as u32 & 34)) as u8
    );

    // buffer0[17] = 115;
    buffer0[17] = 115;

    // buffer1[23] ^= ((((((buffer4[buffer1[17] % 21] | buffer0[buffer3[20] % 20]) & buffer3[72]) | (buffer4[buffer1[17] % 21] & buffer0[buffer3[20] % 20])) & (buffer1[50]/3)) |
    //                 ((((buffer4[buffer1[17] % 21] | buffer0[buffer3[20] % 20]) & buffer3[72]) | (buffer4[buffer1[17] % 21] & buffer0[buffer3[20] % 20]) | (buffer1[50] / 3)) & 246)) << 1);
    let left2 = ((buffer4[(buffer1[17] % 21) as usize] as u32 | buffer0[(buffer3[20] % 20) as usize] as u32) & buffer3[72] as u32) | 
                (buffer4[(buffer1[17] % 21) as usize] as u32 & buffer0[(buffer3[20] % 20) as usize] as u32);
    buffer1[23] ^= ((left2 & (buffer1[50] / 3) as u32) | ((left2 | (buffer1[50] / 3) as u32) & 246)) as u8;

    // buffer0[13] = ((((((buffer0[buffer3[40] % 20] | buffer1[10]) & 82) | (buffer0[buffer3[40] % 20] & buffer1[10])) & 209) |
    //                ((buffer0[buffer1[39] % 20] << 1) & 46)) >> 1);
    let inner3 = ((buffer0[(buffer3[40] % 20) as usize] as u32 | buffer1[10] as u32) & 82) | (buffer0[(buffer3[40] % 20) as usize] as u32 & buffer1[10] as u32);
    buffer0[13] = (((inner3 & 209) | ((buffer0[(buffer1[39] % 20) as usize] as u32) << 1 & 46)) >> 1) as u8;

    // buffer2[33] -= buffer1[113] & 9;
    buffer2[33] = buffer2[33].wrapping_sub(buffer1[113] & 9);

    // buffer2[28] -= ((((2 | (buffer1[110] & 222)) >> 1) & ~223) | (buffer3[20] & 223));
    buffer2[28] = buffer2[28].wrapping_sub(((((2 | (buffer1[110] & 222)) >> 1) & !223) | (buffer3[20] & 223)) as u8);

    // J = weird_rol8((V | Z), (U & 7));
    j = weird_rol8((v | z) as u8, u & 7);
    // A = (buffer2[16] & T) | (W & (~buffer2[16]));
    a = (buffer2[16] as u32 & t) | (w & !buffer2[16] as u32);
    // B = (buffer1[33] & 17) | (X & ~17);
    b = (buffer1[33] as u32 & 17) | (x & !17);
    // E = ((Y | ((A+B) / 5)) & 147) | (Y & ((A+B) / 5));
    e = ((y | ((a.wrapping_add(b)) / 5)) & 147) | (y & ((a.wrapping_add(b)) / 5));
    // M = (buffer3[40] & buffer4[((buffer3[8] + J + E) & 0xff) % 21]) | ((buffer3[40] | buffer4[((buffer3[8] + J + E) & 0xff) % 21]) & buffer2[23]);
    let idx3 = (((buffer3[8] as u32).wrapping_add(j).wrapping_add(e) & 0xff) % 21) as usize;
    m = (buffer3[40] as u32 & buffer4[idx3] as u32) | ((buffer3[40] as u32 | buffer4[idx3] as u32) & buffer2[23] as u32);

    // buffer0[15] = (((buffer4[buffer3[20] % 21] - 48) & (~buffer1[184])) | ((buffer4[buffer3[20] % 21] - 48) & 189) | (189 & ~buffer1[184])) & (M*M*M);
    let left3 = ((buffer4[(buffer3[20] % 21) as usize] as u32).wrapping_sub(48) & !buffer1[184] as u32) | 
                ((buffer4[(buffer3[20] % 21) as usize] as u32).wrapping_sub(48) & 189) | (189 & !buffer1[184] as u32);
    buffer0[15] = (left3 & m.wrapping_mul(m).wrapping_mul(m)) as u8;

    // buffer2[22] += buffer1[183];
    buffer2[22] = buffer2[22].wrapping_add(buffer1[183]);

    // buffer3[76] = (3 * buffer4[buffer1[1] % 21]) ^ buffer3[0];
    buffer3[76] = (3u8.wrapping_mul(buffer4[(buffer1[1] % 21) as usize])) ^ buffer3[0];

    // A = buffer2[((buffer3[8] + (J + E)) & 0xff) % 35];
    a = buffer2[(((buffer3[8] as u32).wrapping_add(j.wrapping_add(e)) & 0xff) % 35) as usize] as u32;
    // F = (((buffer4[buffer1[178] % 21] & A) | ((buffer4[buffer1[178] % 21] | A) & 209)) * buffer0[buffer1[13] % 20]) * (buffer4[buffer1[26] % 21] >> 1);
    f = (((buffer4[(buffer1[178] % 21) as usize] as u32 & a) | ((buffer4[(buffer1[178] % 21) as usize] as u32 | a) & 209)).wrapping_mul(buffer0[(buffer1[13] % 20) as usize] as u32)).wrapping_mul( 
        buffer4[(buffer1[26] % 21) as usize] as u32 >> 1);
    // G = (F + 0x733ffff9) * 198 - (((F + 0x733ffff9) * 396 + 212) & 212) + 85;
    g = f.wrapping_add(0x733ffff9).wrapping_mul(198).wrapping_sub((f.wrapping_add(0x733ffff9).wrapping_mul(396).wrapping_add(212) & 212)).wrapping_add(85);
    // buffer3[80] = buffer3[36] + (G ^ 148) + ((G ^ 107) << 1) - 127;
    buffer3[80] = (buffer3[36] as u32).wrapping_add(g ^ 148).wrapping_add((g ^ 107).wrapping_shl(1)).wrapping_sub(127) as u8;

    // buffer3[84] = ((buffer2[buffer3[64] % 35]) & 245) | (buffer2[buffer3[20] % 35] & 10);
    buffer3[84] = (buffer2[(buffer3[64] % 35) as usize] & 245) | (buffer2[(buffer3[20] % 35) as usize] & 10);

    // A = buffer0[buffer3[68] % 20] | 81;
    a = buffer0[(buffer3[68] % 20) as usize] as u32 | 81;
    // buffer2[18] -= ((A*A*A) & ~buffer0[15]) | ((buffer3[80] / 15) & buffer0[15]);
    buffer2[18] = buffer2[18].wrapping_sub(
        ((a.wrapping_mul(a).wrapping_mul(a) & !buffer0[15] as u32) | ((buffer3[80] as u32 / 15) & buffer0[15] as u32)) as u8
    );

    // buffer3[88] = buffer3[8] + J + E - buffer0[buffer1[160] % 20] + (buffer4[buffer0[((buffer3[8] + J + E) & 255) % 20] % 21] / 3);
    let idx4 = (((buffer3[8] as u32).wrapping_add(j).wrapping_add(e) & 255) % 20) as usize;
    buffer3[88] = (buffer3[8] as u32).wrapping_add(j).wrapping_add(e)
        .wrapping_sub(buffer0[(buffer1[160] % 20) as usize] as u32)
        .wrapping_add(buffer4[(buffer0[idx4] % 21) as usize] as u32 / 3) as u8;

    // B = ((R ^ buffer3[72]) & ~198) | ((S * S) & 198);
    b = ((r ^ buffer3[72] as u32) & !198) | ((s.wrapping_mul(s)) & 198);
    // F = (buffer4[buffer1[69] % 21] & buffer1[172]) | ((buffer4[buffer1[69] % 21] | buffer1[172]) & ((buffer3[12] - B) + 77));
    f = (buffer4[(buffer1[69] % 21) as usize] as u32 & buffer1[172] as u32) | 
        ((buffer4[(buffer1[69] % 21) as usize] as u32 | buffer1[172] as u32) & ((buffer3[12] as u32).wrapping_sub(b).wrapping_add(77)));
    // buffer0[16] = 147 - ((buffer3[72] & ((F & 251) | 1)) | (((F & 250) | buffer3[72]) & 198));
    buffer0[16] = 147u8.wrapping_sub(
        ((buffer3[72] as u32 & ((f & 251) | 1)) | (((f & 250) | buffer3[72] as u32) & 198)) as u8
    );

    // C = (buffer4[buffer1[168] % 21] & buffer0[buffer1[29] % 20] & 7) | ((buffer4[buffer1[168] % 21] | buffer0[buffer1[29] % 20]) & 6);
    c = (buffer4[(buffer1[168] % 21) as usize] as u32 & buffer0[(buffer1[29] % 20) as usize] as u32 & 7) | 
        ((buffer4[(buffer1[168] % 21) as usize] as u32 | buffer0[(buffer1[29] % 20) as usize] as u32) & 6);
    // F = (buffer4[buffer1[155] % 21] & buffer1[105]) | ((buffer4[buffer1[155] % 21] | buffer1[105]) & 141);
    f = (buffer4[(buffer1[155] % 21) as usize] as u32 & buffer1[105] as u32) | 
        ((buffer4[(buffer1[155] % 21) as usize] as u32 | buffer1[105] as u32) & 141);
    // buffer0[3] -= buffer4[weird_rol32(F, C) % 21];
    buffer0[3] = buffer0[3].wrapping_sub(buffer4[(weird_rol32(f as u8, c) % 21) as usize]);

    // buffer1[5] = weird_ror8(buffer0[12], ((buffer0[buffer1[61] % 20] / 5) & 7)) ^ (((~buffer2[buffer3[84] % 35]) & 0xffffffff) / 5);
    buffer1[5] = (weird_ror8(buffer0[12], ((buffer0[(buffer1[61] % 20) as usize] / 5) & 7) as u32) ^ ((!buffer2[(buffer3[84] % 35) as usize] as u32) / 5)) as u8;

    // buffer1[198] += buffer1[3];
    buffer1[198] = buffer1[198].wrapping_add(buffer1[3]);

    // A = (162 | buffer2[buffer3[64] % 35]);
    a = 162 | buffer2[(buffer3[64] % 35) as usize] as u32;
    // buffer1[164] += ((A*A)/5);
    buffer1[164] = buffer1[164].wrapping_add((a.wrapping_mul(a) / 5) as u8);

    // G = weird_ror8(139, (buffer3[80] & 7));
    g = weird_ror8(139, (buffer3[80] & 7) as u32);
    // C = ((buffer4[buffer3[64] % 21] * buffer4[buffer3[64] % 21] * buffer4[buffer3[64] % 21]) & 95) | (buffer0[buffer3[40] % 20] & ~95);
    let idx5 = (buffer3[64] % 21) as usize;
    c = ((buffer4[idx5].wrapping_mul(buffer4[idx5]).wrapping_mul(buffer4[idx5]) as u32) & 95) | (buffer0[(buffer3[40] % 20) as usize] as u32 & !95);
    // buffer3[92] = (G & 12) | (buffer0[buffer3[20] % 20] & 12) | (G & buffer0[buffer3[20] % 20]) | C;
    buffer3[92] = ((g & 12) | (buffer0[(buffer3[20] % 20) as usize] as u32 & 12) | (g & buffer0[(buffer3[20] % 20) as usize] as u32) | c) as u8;

    // buffer2[12] += ((buffer1[103] & 32) | (buffer3[92] & ((buffer1[103] | 60))) | 16)/3;
    buffer2[12] = buffer2[12].wrapping_add((((buffer1[103] & 32) | (buffer3[92] & (buffer1[103] | 60)) | 16) / 3) as u8);

    // buffer3[96] = buffer1[143];
    buffer3[96] = buffer1[143];

    // buffer3[100] = 27;
    buffer3[100] = 27;

    // buffer3[104] = (((buffer3[40] & ~buffer2[8]) | (buffer1[35] & buffer2[8])) & buffer3[64]) ^ 119;
    buffer3[104] = (((buffer3[40] & !buffer2[8]) | (buffer1[35] & buffer2[8])) & buffer3[64]) ^ 119;

    // buffer3[108] = 238 & ((((buffer3[40] & ~buffer2[8]) | (buffer1[35] & buffer2[8])) & buffer3[64]) << 1);
    buffer3[108] = 238 & ((((buffer3[40] & !buffer2[8]) | (buffer1[35] & buffer2[8])) & buffer3[64]) << 1);

    // buffer3[112] = (~buffer3[64] & (buffer3[84] / 3)) ^ 49;
    buffer3[112] = (!buffer3[64] & (buffer3[84] / 3)) ^ 49;

    // buffer3[116] = 98 & ((~buffer3[64] & (buffer3[84] / 3)) << 1);
    buffer3[116] = 98 & ((!buffer3[64] & (buffer3[84] / 3)) << 1);

    // finale
    // A = (buffer1[35] & buffer2[8]) | (buffer3[40] & ~buffer2[8]);
    a = (buffer1[35] as u32 & buffer2[8] as u32) | (buffer3[40] as u32 & !buffer2[8] as u32);
    // B = (A & buffer3[64]) | (((buffer3[84] / 3) & ~buffer3[64]));
    b = (a & buffer3[64] as u32) | ((buffer3[84] / 3) as u32 & !buffer3[64] as u32);
    // buffer1[143] = buffer3[96] - ((B & (86 + ((buffer1[172] & 64) >> 1))) | (((((buffer1[172] & 65) >> 1) ^ 86) | ((~buffer3[64] & (buffer3[84] / 3)) | (((buffer3[40] & ~buffer2[8]) | (buffer1[35] & buffer2[8])) & buffer3[64]))) & buffer3[100]));
    let right2 = (((buffer1[172] & 65) >> 1) ^ 86) | ((!buffer3[64] & (buffer3[84] / 3)) | (((buffer3[40] & !buffer2[8]) | (buffer1[35] & buffer2[8])) & buffer3[64]));
    buffer1[143] = buffer3[96].wrapping_sub(
        ((b & (86 + ((buffer1[172] & 64) >> 1) as u32)) | (right2 as u32 & buffer3[100] as u32)) as u8
    );

    // buffer2[29] = 162;
    buffer2[29] = 162;

    // A = ((((buffer4[buffer3[88] % 21]) & 160) | (buffer0[buffer1[125] % 20] & 95)) >> 1);
    a = (((buffer4[(buffer3[88] % 21) as usize] as u32 & 160) | (buffer0[(buffer1[125] % 20) as usize] as u32 & 95)) >> 1);
    // B = buffer2[buffer1[149] % 35] ^ (buffer1[43] * buffer1[43]);
    b = buffer2[(buffer1[149] % 35) as usize] as u32 ^ (buffer1[43] as u32).wrapping_mul(buffer1[43] as u32);
    // buffer0[15] += (B&A) | ((A|B) & 115);
    buffer0[15] = buffer0[15].wrapping_add(((b & a) | ((a | b) & 115)) as u8);

    // buffer3[120] = buffer3[64] - buffer0[buffer3[40] % 20];
    buffer3[120] = buffer3[64].wrapping_sub(buffer0[(buffer3[40] % 20) as usize]);

    // buffer1[95] = buffer4[buffer3[20] % 21];
    buffer1[95] = buffer4[(buffer3[20] % 21) as usize];

    // A = weird_ror8(buffer2[buffer3[80] % 35], (buffer2[buffer1[17] % 35] * buffer2[buffer1[17] % 35] * buffer2[buffer1[17] % 35]) & 7);
    let idx6 = (buffer1[17] % 35) as usize;
    a = weird_ror8(buffer2[(buffer3[80] % 35) as usize], (buffer2[idx6].wrapping_mul(buffer2[idx6]).wrapping_mul(buffer2[idx6]) & 7) as u32);
    // buffer0[7] -= (A*A);
    buffer0[7] = buffer0[7].wrapping_sub((a.wrapping_mul(a)) as u8);

    // buffer2[8] = buffer2[8] - buffer1[184] + (buffer4[buffer1[202] % 21] * buffer4[buffer1[202] % 21] * buffer4[buffer1[202] % 21]);
    let idx7 = (buffer1[202] % 21) as usize;
    buffer2[8] = buffer2[8].wrapping_sub(buffer1[184]).wrapping_add(buffer4[idx7].wrapping_mul(buffer4[idx7]).wrapping_mul(buffer4[idx7]));

    // buffer0[16] = (buffer2[buffer1[102] % 35] << 1) & 132;
    buffer0[16] = (buffer2[(buffer1[102] % 35) as usize] << 1) & 132;

    // buffer3[124] = (buffer4[buffer3[40] % 21] >> 1) ^ buffer3[68];
    buffer3[124] = (buffer4[(buffer3[40] % 21) as usize] >> 1) ^ buffer3[68];

    // buffer0[7] -= (buffer0[buffer1[191] % 20] - (((buffer4[buffer1[80] % 21] << 1) & ~177) | (buffer4[buffer4[buffer3[88] % 21] % 21] & 177)));
    let inner4 = ((buffer4[(buffer1[80] % 21) as usize] << 1) & !177) | (buffer4[(buffer4[(buffer3[88] % 21) as usize] % 21) as usize] & 177);
    buffer0[7] = buffer0[7].wrapping_sub(buffer0[(buffer1[191] % 20) as usize].wrapping_sub(inner4));

    // buffer0[6] = buffer0[buffer1[119] % 20];
    buffer0[6] = buffer0[(buffer1[119] % 20) as usize];

    // A = (buffer4[buffer1[190] % 21] & ~209) | (buffer1[118] & 209);
    a = (buffer4[(buffer1[190] % 21) as usize] as u32 & !209) | (buffer1[118] as u32 & 209);
    // B = buffer0[buffer3[120] % 20] * buffer0[buffer3[120] % 20];
    b = buffer0[(buffer3[120] % 20) as usize] as u32 * buffer0[(buffer3[120] % 20) as usize] as u32;
    // buffer0[12] = (buffer0[buffer3[84] % 20] ^ (buffer2[buffer1[71] % 35] + buffer2[buffer1[15] % 35])) & ((A & B) | ((A | B) & 27));
    buffer0[12] = (buffer0[(buffer3[84] % 20) as usize] ^ (buffer2[(buffer1[71] % 35) as usize].wrapping_add(buffer2[(buffer1[15] % 35) as usize]))) & 
                  (((a & b) | ((a | b) & 27)) as u8);

    // B = (buffer1[32] & buffer2[buffer3[88] % 35]) | ((buffer1[32] | buffer2[buffer3[88] % 35]) & 23);
    b = (buffer1[32] as u32 & buffer2[(buffer3[88] % 35) as usize] as u32) | ((buffer1[32] as u32 | buffer2[(buffer3[88] % 35) as usize] as u32) & 23);
    // D = (((buffer4[buffer1[57] % 21] * 231) & 169) | (B & 86));
    d = ((buffer4[(buffer1[57] % 21) as usize] as u32).wrapping_mul(231) & 169) | (b & 86);
    // F = (((buffer0[buffer1[82] % 20] & ~29) | (buffer4[buffer3[124] % 21] & 29)) & 190) | (buffer4[(D/5) % 21] & ~190);
    f = ((buffer0[(buffer1[82] % 20) as usize] as u32 & !29) | (buffer4[(buffer3[124] % 21) as usize] as u32 & 29) & 190) | 
        (buffer4[((d / 5) % 21) as usize] as u32 & !190);
    // H = buffer0[buffer3[40] % 20] * buffer0[buffer3[40] % 20] * buffer0[buffer3[40] % 20];
    h = buffer0[(buffer3[40] % 20) as usize] as u32 * buffer0[(buffer3[40] % 20) as usize] as u32 * buffer0[(buffer3[40] % 20) as usize] as u32;
    // K = (H & buffer1[82]) | (H & 92) | (buffer1[82] & 92);
    k = (h & buffer1[82] as u32) | (h & 92) | (buffer1[82] as u32 & 92);
    // buffer3[128] = ((F & K) | ((F | K) & 192)) ^ (D/5);
    buffer3[128] = (((f & k) | ((f | k) & 192)) ^ (d / 5)) as u8;

    // buffer2[25] ^= ((buffer0[buffer3[120] % 20] << 1) * buffer1[5]) - (weird_rol8(buffer3[76], (buffer4[buffer3[124] % 21] & 7)) & (buffer3[20] + 110));
    let left4 = ((buffer0[(buffer3[120] % 20) as usize] as u32) << 1).wrapping_mul(buffer1[5] as u32);
    let right3 = weird_rol8(buffer3[76], (buffer4[(buffer3[124] % 21) as usize] & 7) as u32) & (buffer3[20] as u32 + 110);
    buffer2[25] ^= (left4.wrapping_sub(right3)) as u8;
}
