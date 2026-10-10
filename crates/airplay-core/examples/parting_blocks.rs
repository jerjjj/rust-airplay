// SPDX-License-Identifier: GPL-3.0-or-later
//! Whether upstream's keystream after the parting is this stream at another place.
//!
//! A counter that moved wrongly still reads *this* stream, just from somewhere else, so
//! every one of its blocks is a block that is here — at a different number. A stream that
//! is not this one at all cannot be found here however far the search goes, and those are
//! different faults with different fixes.
//!
//! So each of upstream's blocks after the parting is looked for in the whole stream, one
//! at a time, and the answer for each is printed as a block number.

use airplay_core::media::mirror::{MirrorDecryptor, stream_keys};

/// How many blocks of the stream to generate, which is the whole of one for these lengths.
const BLOCKS: usize = 32768;

/// How many of upstream's blocks after the parting to look for.
const LOOKED_FOR: usize = 16;

fn main() {
    let fields = oracle_fields();
    let connection_id: u64 = field(&fields, "mirror.connection_id")
        .parse()
        .expect("the oracle prints the connection identifier");
    let keys = stream_keys(connection_id, &[0x5a; 16]);
    let stream = MirrorDecryptor::new(&keys)
        .expect("keys")
        .keystream(0, BLOCKS);

    // The length whose answer parts, and the byte it parts at: both measured, and the same
    // for every length past three thousand.
    let length = "8000";
    let upstream = hex(&field(
        &fields,
        &format!("mirror.single{}", index_of(length)),
    ));
    let parting = (0..upstream.len())
        .find(|at| upstream[*at] != stream[*at])
        .expect("the answer parts from the stream");

    println!(
        "the answer parts at byte {parting} (block {} + {}); \
         looking for each of its blocks after that in the stream",
        parting / 16,
        parting % 16
    );
    for block in 0..LOOKED_FOR {
        let at = parting + block * 16;
        if at + 16 > upstream.len() {
            break;
        }
        let want = &upstream[at..at + 16];
        let found: Vec<usize> = stream
            .windows(16)
            .enumerate()
            .filter(|(_, window)| *window == want)
            .map(|(at, _)| at / 16)
            .collect();
        println!(
            "  upstream's block {block:>2} after the parting (stream block {}): found in the \
             stream at blocks {found:?}",
            at / 16
        );
    }
}

fn index_of(length: &str) -> usize {
    let fields = oracle_fields();
    (0..)
        .find(|step| {
            field(&fields, &format!("mirror.single_length{step}"))
                .parse::<usize>()
                .map(|printed| printed.to_string() == length)
                .unwrap_or(false)
        })
        .expect("the oracle printed that length")
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
