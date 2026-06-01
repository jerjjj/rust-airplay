use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <key_hex> <iv_hex>", args[0]);
        return;
    }
    let key = hex::decode(&args[1]).expect("invalid key hex");
    let iv = hex::decode(&args[2]).expect("invalid iv hex");
    
    let enc = fs::read("encrypted_frame.bin").expect("encrypted_frame.bin not found");
    
    // Decrypt with AES-CTR BE
    use aes::Aes128;
    use ctr::cipher::{KeyIvInit, StreamCipher};
    
    let key_arr: [u8; 16] = key.as_slice().try_into().unwrap();
    let iv_arr: [u8; 16] = iv.as_slice().try_into().unwrap();
    
    let mut dec = enc.clone();
    ctr::Ctr128BE::<Aes128>::new(&key_arr.into(), &iv_arr.into())
        .apply_keystream(&mut dec);
    
    let nalu_size = u32::from_be_bytes([dec[0], dec[1], dec[2], dec[3]]);
    println!("NAL unit size: {}", nalu_size);
    println!("First 32 bytes decrypted: {:02x?}", &dec[..32.min(dec.len())]);
    
    fs::write("decrypted_frame.bin", &dec).unwrap();
    println!("Saved decrypted_frame.bin ({} bytes)", dec.len());
    
    // Also try LE mode
    let mut dec_le = enc.clone();
    ctr::Ctr128LE::<Aes128>::new(&key_arr.into(), &iv_arr.into())
        .apply_keystream(&mut dec_le);
    let nalu_le = u32::from_be_bytes([dec_le[0], dec_le[1], dec_le[2], dec_le[3]]);
    println!("LE: NAL unit size: {}", nalu_le);
    println!("LE: First 32 bytes: {:02x?}", &dec_le[..32.min(dec_le.len())]);
    fs::write("decrypted_frame_le.bin", &dec_le).unwrap();
}
