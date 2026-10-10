// SPDX-License-Identifier: GPL-3.0-or-later
//! Which frame size turns a run of audio bytes into a stream a decoder will take.
//!
//! The bytes a client sent are known, and so is what a decoder wants: frames, and a
//! configuration that says what they are — raw AAC carries no sample rate, so a decoder
//! given a bare run of it refuses with "no valid frames found" and nothing is heard. What
//! is not known from a dump that recorded no lengths is where the frames begin and end.
//!
//! So this tries every frame size in the range a stream's frames can have, writes the run
//! cut at that size with the AudioSpecificConfig in front of it, and leaves the candidate
//! files behind for a decoder to be pointed at. One of them decodes; the sizes that do not
//! are silent, which is the evidence this port has been missing.

use std::io::Write;

/// The configuration of an AAC-ELD stream at 44.1 kHz in stereo, as the bits
/// `AudioSpecificConfig` is made of: five bits of object type (39, ELD), four bits of
/// sampling frequency (4, 44100), four bits of channel configuration (2, stereo), and the
/// three bits that follow them. Written out here because it is what a decoder needs and
/// what a raw frame does not carry.
const AUDIO_SPECIFIC_CONFIG: [u8; 3] = [0xf8, 0x2c, 0x00];

/// The frame sizes to try. A 480-sample frame at any bitrate an audio stream uses lands in
/// this range, and one of these is the size the client is sending.
const CANDIDATES: std::ops::RangeInclusive<usize> = 8..=600;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/client-audio.bin".to_owned());
    let bytes = std::fs::read(&path).expect("the dump is readable");
    println!("{path}: {} bytes", bytes.len());

    // The start-up markers are four bytes each and carry no audio, so they are skipped:
    // whatever follows them is a whole number of frames, which is what makes a size
    // findable by division as well as by trying.
    let mut at = 0usize;
    while at + 4 <= bytes.len()
        && bytes[at] == 0x00
        && bytes[at + 1] == 0x68
        && bytes[at + 2] == 0x34
        && bytes[at + 3] == 0x00
    {
        at += 4;
    }
    println!(
        "{at} bytes of start-up markers, then {} bytes of audio",
        bytes.len() - at
    );
    let audio = &bytes[at..];

    let out = std::path::Path::new("target/audio-candidates");
    std::fs::create_dir_all(out).expect("a directory for the candidates");

    let mut written = 0;
    for size in CANDIDATES {
        if audio.len() < size * 4 {
            break;
        }
        // A frame size that divides the run is far more likely than one that leaves a
        // remainder: a stream is whole frames, and the markers were taken off the front.
        // The remainder is counted rather than required to be zero, because the last frame
        // may be cut short by where the recording stopped.
        let whole = audio.len() / size;
        if whole < 4 {
            break;
        }
        let remainder = audio.len() % size;
        // Only sizes that come close to dividing the run are worth a file: the rest cannot
        // be the frame size of a stream that was recorded whole.
        if remainder > size / 2 {
            continue;
        }
        let file = out.join(format!("frame-{size}.aac"));
        let mut handle = std::fs::File::create(&file).expect("a candidate file");
        handle
            .write_all(&AUDIO_SPECIFIC_CONFIG)
            .expect("the configuration");
        handle
            .write_all(&audio[..whole * size])
            .expect("the frames");
        written += 1;
        if written <= 20 {
            println!(
                "  wrote {} for a frame size of {size} ({whole} frames, {remainder} left over)",
                file.display()
            );
        }
    }
    println!("\n{written} candidates in {}", out.display());
    println!("point a decoder at one and see which decodes");
}
