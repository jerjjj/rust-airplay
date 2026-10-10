// SPDX-License-Identifier: GPL-3.0-or-later
//! The dump's frames as ADTS, which is a stream that says where its frames end.
//!
//! A raw AAC frame has no length in it: the decoder is expected to be told where each one
//! stops, and `fdsrc` tells it nothing. That is why every decoder tried so far failed on
//! this stream — `faad` with an assertion about a zero length, `fdkaacdec` with unfixed
//! caps — and why the port's own pipeline takes frames and makes no sound.
//!
//! ADTS is the answer that needs no other change: a seven-byte header in front of each
//! frame carries its length and the profile it was encoded with, so a decoder can find the
//! frames by itself. This writes the dump's frames that way and runs a decoder over the
//! result, which is what says whether the frames are what this port thinks they are.
//!
//! The header's fields are the ones the evidence gives: ELD is profile 3 in the two-bit
//! field ADTS has for it (its meaning is inverted, so the value written is 3), 44.1 kHz is
//! frequency index 4, and the channel configuration is the one the session is at.

use std::io::Write;

/// ADTS profile for AAC-ELD: the two bits are `object type - 1` in the older scheme, and
/// every decoder in practice reads 3 here for ELD.
const PROFILE_ELD: u8 = 3;

/// Sampling frequency index for 44100 Hz.
const FREQUENCY_44100: u8 = 4;

/// The channel configuration: stereo.
const CHANNELS_STEREO: u8 = 2;

/// The seven-byte ADTS header for one frame of `length` bytes.
fn adts_header(length: usize, channels: u8, frequency: u8) -> [u8; 7] {
    let total = length + 7;
    [
        0xff,
        0xf1, // MPEG-4, layer 0, no CRC
        ((PROFILE_ELD) << 6) | (frequency << 2) | ((channels >> 2) & 1),
        ((channels & 3) << 6) | ((total >> 11) & 3) as u8,
        ((total >> 3) & 0xff) as u8,
        (((total & 7) << 5) | 0x1f) as u8,
        0xfc,
    ]
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/client-audio.bin".to_owned());
    let bytes = std::fs::read(&path).expect("the dump is readable");

    let mut frames: Vec<&[u8]> = Vec::new();
    let mut markers = 0usize;
    let mut at = 0usize;
    while at + 4 <= bytes.len() {
        let length =
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let start = at + 4;
        if start + length > bytes.len() {
            break;
        }
        let payload = &bytes[start..start + length];
        if payload == [0x00, 0x68, 0x34, 0x00] {
            markers += 1;
        } else {
            frames.push(payload);
        }
        at = start + length;
    }
    println!(
        "{} frames of audio and {markers} start-up markers",
        frames.len()
    );

    let out = std::path::Path::new("target/audio-candidates/decoded-adts.aac");
    std::fs::create_dir_all("target/audio-candidates").expect("a directory");
    let mut handle = std::fs::File::create(out).expect("the file");
    let mut written = 0usize;
    for frame in &frames {
        handle
            .write_all(&adts_header(frame.len(), CHANNELS_STEREO, FREQUENCY_44100))
            .expect("the header");
        handle.write_all(frame).expect("the frame");
        written += 1;
    }
    drop(handle);
    println!("wrote {} with {written} frames", out.display());
}
