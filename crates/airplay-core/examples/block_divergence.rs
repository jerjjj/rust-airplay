// SPDX-License-Identifier: GPL-3.0-or-later
//! Where a long packet's bytes come from, as block numbers, on both sides.
//!
//! The divergence this looks at has no carry in it: the first packet is a whole number of
//! blocks, so the second starts on a block boundary and reads straight through. That
//! leaves one question — which block of the stream each of its bytes comes from — and it
//! is answerable, because upstream's bytes for the packet are its keystream: they can be
//! found in a stream generated here.
//!
//! The search is local, within a window around where this port reads the byte, and not a
//! search of the whole stream: a run of sixty-four bytes is long enough to be unique
//! nearby, and a whole-stream search can find a coincidence thousands of bytes away and
//! report a rule that is not there. That mistake has already been made once, against this
//! very pair.

use airplay_core::media::mirror::{MirrorDecryptor, stream_keys};

/// The pair to look at: a first packet of whole blocks and a long second one, which
/// diverges and has no carry to explain it.
const FIRST: usize = 4096;
const SECOND: usize = 8000;

/// The window a byte's sixty-four-byte run is looked for in, either side of where this
/// port reads it.
const NEARBY: usize = 8192;

fn main() {
    let fields = oracle_fields();
    let connection_id: u64 = field(&fields, "mirror.connection_id")
        .parse()
        .expect("the oracle prints the connection identifier");
    let keys = stream_keys(connection_id, &[0x5a; 16]);
    let stream = MirrorDecryptor::new(&keys)
        .expect("keys")
        .keystream(0, 16384);

    // Upstream's answer for this pair, from the oracle's own output.
    let upstream = hex(&field(&fields, "mirror.pair_read9"));

    let mut decryptor = MirrorDecryptor::new(&keys).expect("keys");
    let mut sink = vec![0u8; FIRST];
    decryptor
        .decrypt(&vec![0u8; FIRST], &mut sink)
        .expect("first");
    let mut ours = vec![0u8; SECOND];
    decryptor
        .decrypt(&vec![0u8; SECOND], &mut ours)
        .expect("second");

    println!(
        "first {FIRST} bytes ({FIRST} is {} blocks, carry {}), second {SECOND}",
        FIRST / 16,
        decryptor.carried()
    );
    println!(
        "the second packet's fresh blocks start at 0 of its own bytes, so byte b is stream byte {FIRST} + b"
    );

    // Every byte this port reads, by its own rule: a whole-block first packet means the
    // second reads straight from a block boundary with nothing carried.
    let mut reported = 0;
    let mut differing = 0;
    for at in 0..SECOND {
        if at + 64 > upstream.len() {
            break;
        }
        let ours_at = FIRST + at;
        // The longest run of upstream's bytes starting here that this port's stream has
        // somewhere near, and where. A fixed window is no use here: the first byte that
        // does not match makes every window that contains it unfindable, and the report
        // then says "nowhere" about bytes that are only one byte past the end of a run.
        let mut best: Option<(usize, usize)> = None;
        for width in (8..=64).rev() {
            if at + width > upstream.len() {
                continue;
            }
            let want = &upstream[at..at + width];
            let near = ours_at.saturating_sub(NEARBY);
            let far = (ours_at + NEARBY).min(stream.len().saturating_sub(width));
            if let Some(offset) = stream[near..far]
                .windows(width)
                .position(|window| window == want)
            {
                best = Some((near + offset, width));
                break;
            }
        }

        if best.map(|(found, _)| found) != Some(ours_at) {
            differing += 1;
            if reported < 24 {
                match best {
                    Some((found, width)) => println!(
                        "  byte {at:>5}: this reads stream byte {ours_at} (block {}), upstream's \
                         next {width} bytes sit at {found} (block {}, a shift of {} blocks)",
                        ours_at / 16,
                        found / 16,
                        found as i64 / 16 - ours_at as i64 / 16
                    ),
                    None => println!(
                        "  byte {at:>5}: this reads stream byte {ours_at} (block {}), and \
                         upstream's bytes are not in the stream within {NEARBY} bytes of it",
                        ours_at / 16
                    ),
                }
                reported += 1;
            }
        }
    }
    println!("{differing} of {SECOND} bytes do not sit where this port reads them");
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
