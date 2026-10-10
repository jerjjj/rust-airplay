// SPDX-License-Identifier: GPL-3.0-or-later
//! Ours beside upstream's, byte by byte, where they part.
//!
//! The rule this port reproduces is a mapping from a packet's byte index to a position in
//! the stream, and a mapping can be read off a table. This prints that table around the
//! first place the two disagree: the byte values from both sides, and the stream position
//! this port reads each byte from.
//!
//! Upstream's side is the oracle's own answer for the packet, which is a fact rather than
//! a rule. Where its bytes sit in the stream is deliberately *not* searched for here: a
//! search reports where a run of bytes can be found, which for bytes that are not in the
//! stream at all — a carry, which is the rest of a block already read past — finds the
//! wrong thing or nothing, and either way says the wrong thing about the rule.

use airplay_core::media::mirror::{MirrorDecryptor, stream_keys};

/// The pairs the oracle walks, `(first, second)`, in its own order.
const PAIRS: [(usize, usize); 10] = [
    (62415, 1265),
    (1265, 9710),
    (9710, 1237),
    (1356, 1355),
    (1265, 1265),
    (1265, 4096),
    (1265, 8000),
    (33, 8000),
    (1265, 4097),
    (4096, 8000),
];

/// How many bytes either side of the first difference to print.
const AROUND: usize = 6;

fn main() {
    let fields = oracle_fields();
    let connection_id: u64 = field(&fields, "mirror.connection_id")
        .parse()
        .expect("the oracle prints the connection identifier");
    let keys = stream_keys(connection_id, &[0x5a; 16]);

    for (pair, (first, second)) in PAIRS.iter().enumerate() {
        let upstream = hex(&field(&fields, &format!("mirror.pair_read{pair}")));
        let mut decryptor = MirrorDecryptor::new(&keys).expect("keys");
        let mut sink = vec![0u8; *first];
        decryptor
            .decrypt(&vec![0u8; *first], &mut sink)
            .expect("first");
        let mut ours = vec![0u8; *second];
        decryptor
            .decrypt(&vec![0u8; *second], &mut ours)
            .expect("second");

        let first_difference = ours.iter().zip(&upstream).position(|(a, b)| a != b);
        println!(
            "\npair {pair}: first {first} (block boundary {}), second {second}, \
             first difference at {first_difference:?}",
            first.div_ceil(16) * 16
        );
        let Some(at) = first_difference else {
            println!("  identical for all {second} bytes");
            continue;
        };
        // What this port read the diverging byte from, in blocks of the stream: the fresh
        // part starts on a block, so the block is the byte's own offset past it. This is
        // the number the question is about — a rule is a block number, and an unexplained
        // difference is a difference in one.
        let probe = {
            let mut probe = MirrorDecryptor::new(&keys).expect("keys");
            let mut sink = vec![0u8; *first];
            probe.decrypt(&vec![0u8; *first], &mut sink).expect("first");
            (probe.carried(), probe.read_block())
        };
        println!(
            "  carried {} bytes, fresh blocks from {}; byte {at} is fresh byte {} of block {}",
            probe.0,
            probe.1,
            at - probe.0,
            probe.1 + (at - probe.0) as u64 / 16
        );
        let from = at.saturating_sub(AROUND);
        let to = (at + AROUND).min(*second);
        for index in from..to {
            let marker = if ours[index] == upstream[index] {
                " "
            } else {
                "*"
            };
            println!(
                "{marker} byte {index:>6}: ours {:02x}, upstream {:02x}",
                ours[index], upstream[index]
            );
        }
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
