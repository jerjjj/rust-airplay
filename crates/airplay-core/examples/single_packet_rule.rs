// SPDX-License-Identifier: GPL-3.0-or-later
//! Where a single long packet parts from upstream's reader, in blocks.
//!
//! No packet before it, so no carry and no counter to restart: the reader is given one
//! length and reads it straight through. Anything that goes wrong in the middle of that is
//! a property of the reader and of nothing else — which is why this is the measurement to
//! make, after two packets left the question tangled with what the first one did.
//!
//! Each answer is compared with this port's own stream at the offset the byte belongs to,
//! and where it stops agreeing is reported as a block number.

use airplay_core::media::mirror::{MirrorDecryptor, stream_keys};

/// How many blocks of the stream to generate.
const BLOCKS: usize = 32768;

fn main() {
    let fields = oracle_fields();
    let connection_id: u64 = field(&fields, "mirror.connection_id")
        .parse()
        .expect("the oracle prints the connection identifier");
    let keys = stream_keys(connection_id, &[0x5a; 16]);
    let stream = MirrorDecryptor::new(&keys)
        .expect("keys")
        .keystream(0, BLOCKS);

    let mut step = 0;
    // Every step the oracle printed, in its own order: asking for a key that is not there
    // is how the walk ends, so a missing length ends it too.
    while fields
        .iter()
        .any(|(name, _)| name == &format!("mirror.single_length{step}"))
    {
        let length = field(&fields, &format!("mirror.single_length{step}"));
        let length: usize = length.parse().expect("a length");
        let upstream = hex(&field(&fields, &format!("mirror.single{step}")));

        // The length of the answer, which is the packet's length: the oracle prints the
        // bytes the reader read, and it read exactly as many as the packet has.
        let known = upstream.len().min(length).min(stream.len());

        // The first byte where upstream's answer is not the stream's own byte, which for a
        // packet with nothing before it is where the reader stops following the stream.
        let parts = (0..known).find(|at| upstream[*at] != stream[*at]);
        match parts {
            Some(at) => {
                // The bytes either side of the parting, from both sides, and where the
                // stream's own bytes for the parting are. The distance between the two is
                // the number this is looking for, and the bytes say whether the answer is
                // a shift at all — a run of eight bytes can be found somewhere it does not
                // belong.
                let from = at.saturating_sub(4);
                let to = (at + 12).min(known);
                println!(
                    "length {length:>7}: parts at byte {at} (block {} + {})",
                    at / 16,
                    at % 16
                );
                println!("  upstream {:02x?}", &upstream[from..to]);
                println!("  stream   {:02x?}", &stream[from..to]);
                // Where the stream's own bytes from the parting onwards turn up again,
                // which is what a shifted counter would do.
                let here = &stream[at..(at + 64).min(stream.len())];
                let found = stream
                    .windows(here.len())
                    .enumerate()
                    .filter(|(_, window)| *window == here)
                    .map(|(at, _)| at)
                    .collect::<Vec<_>>();
                println!("  the stream's own bytes from {at} occur at {found:?}");
            }
            None => println!("length {length:>7}: the stream itself, for all {known} bytes"),
        }
        step += 1;
    }
}

/// The oracle's output as key/value pairs, running it once.
fn oracle_fields() -> Vec<(String, String)> {
    let path = std::env::var("AIRPLAY_UPSTREAM_ORACLE")
        .unwrap_or_else(|_| "tools/oracle/oracle.exe".to_owned());
    let output = std::process::Command::new(path)
        .arg("mirror")
        .output()
        .expect("the oracle runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(" = "))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn field(fields: &[(String, String)], key: &str) -> String {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| panic!("the oracle printed no {key}"))
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex"))
        .collect()
}
