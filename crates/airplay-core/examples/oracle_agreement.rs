// SPDX-License-Identifier: GPL-3.0-or-later
//! The oracle's own answer for one long packet, against this port's stream, byte by byte.
//!
//! Two of the oracle's outputs disagree with each other about the same stream: its
//! `mirror.block*` values, read through one cipher walked block by block, and its
//! `mirror.single3`, read through a packet. A port can only be compared with one of them,
//! and which one is right has to be settled before anything else is.
//!
//! `mirror.single3` is the one that matters: it is what a reader actually reads for a
//! packet, which is what a session does. So this compares it with the stream this port
//! generates, byte by byte, and prints where they first disagree and where the oracle's
//! bytes are instead.

use airplay_core::media::mirror::{MirrorDecryptor, stream_keys};

fn main() {
    let fields = oracle_fields();
    let connection_id: u64 = field(&fields, "mirror.connection_id")
        .parse()
        .expect("the oracle prints the connection identifier");
    let keys = stream_keys(connection_id, &[0x5a; 16]);
    let stream = MirrorDecryptor::new(&keys)
        .expect("keys")
        .keystream(0, 16384);

    let single = hex(&field(&fields, "mirror.single3"));
    let parts = (0..single.len().min(stream.len())).find(|at| single[*at] != stream[*at]);
    println!(
        "the oracle's packet of {} bytes parts from the stream at {parts:?}",
        single.len()
    );

    // Where each of the packet's blocks sits in the stream, one block at a time, over the
    // neighbourhood of the parting: the same list the earlier report gave, from the packet
    // rather than from a walked cipher, so the two can be compared with each other.
    if let Some(at) = parts {
        for block in (at / 16).saturating_sub(2)..(at / 16) + 4 {
            let want = &single[block * 16..block * 16 + 16];
            let found: Vec<u64> = (0..1024u64)
                .filter(|other| {
                    MirrorDecryptor::new(&keys)
                        .expect("keys")
                        .keystream(*other, 1)
                        == want
                })
                .collect();
            println!("  the packet's block {block} is this port's block {found:?}");
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
