// SPDX-License-Identifier: GPL-3.0-or-later
//! Differential checks against the upstream C implementation.
//!
//! The oracle under `tools/oracle/` compiles UxPlay's own `crypto.c`,
//! `pairing.c` and `playfair/*` into a small program that prints fixed vectors.
//! These tests run it and compare its answers with this port, so a protocol claim
//! is checked against the C code instead of against a second reading of it.
//!
//! Ignored by default because it needs a C toolchain; see
//! `tools/oracle/README.md`.
//!
//! ```text
//! pwsh tools/oracle/build.ps1
//! $env:AIRPLAY_UPSTREAM_ORACLE = "$PWD/tools/oracle/oracle.exe"
//! cargo test -p airplay-core --test upstream_oracle -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::process::Command;

use airplay_core::crypto::{Aes128CtrState, aes128_gcm16_decrypt, aes128_gcm16_encrypt};
use airplay_core::fairplay::FairPlaySession;
use airplay_core::media::mirror::{MirrorDecryptor, stream_keys};
use airplay_core::pairing::{
    derive_pair_setup_keys, derive_pair_verify_keys, pair_setup_gcm_nonces,
};

/// Runs the oracle in `mode` and returns its `key = value` lines.
fn oracle(mode: &str) -> HashMap<String, String> {
    let path = std::env::var("AIRPLAY_UPSTREAM_ORACLE")
        .expect("set AIRPLAY_UPSTREAM_ORACLE to the oracle built by tools/oracle/build.ps1");
    let output = Command::new(&path)
        .arg(mode)
        .output()
        .unwrap_or_else(|error| panic!("running {path} {mode}: {error}"));
    assert!(
        output.status.success(),
        "{path} {mode} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("the oracle prints ascii");
    let mut fields = HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(" = ") {
            fields.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    assert!(!fields.is_empty(), "no vectors in:\n{text}");
    fields
}

fn field<'a>(fields: &'a HashMap<String, String>, key: &str) -> &'a str {
    fields
        .get(key)
        .unwrap_or_else(|| panic!("the oracle did not print {key}"))
}

fn bytes<const N: usize>(fields: &HashMap<String, String>, key: &str) -> [u8; N] {
    let text = field(fields, key);
    let mut out = [0u8; N];
    assert_eq!(text.len(), N * 2, "{key} has an unexpected length: {text}");
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex");
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Fixed inputs the oracle uses for the pair-verify and pair-setup vectors.
fn sequence<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = index as u8;
    }
    out
}

#[test]
#[ignore = "needs the upstream C oracle; see tools/oracle/README.md"]
fn pair_verify_keystream_matches_upstream() {
    let fields = oracle("crypto");
    let shared: [u8; 32] = bytes(&fields, "verify.shared");
    assert_eq!(shared, sequence::<32>());

    let keys = derive_pair_verify_keys(&shared);
    assert_eq!(keys.key, bytes::<16>(&fields, "verify.key"));
    assert_eq!(keys.iv, bytes::<16>(&fields, "verify.iv"));

    // Upstream's AES-CTR context produces one 128-byte keystream for both
    // signatures; our port has to expose exactly the same two halves.
    let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv).expect("key and iv");
    let mut keystream = [0u8; 128];
    cipher.apply(&mut keystream);
    assert_eq!(
        &keystream[..64],
        &bytes::<64>(&fields, "verify.keystream0")[..]
    );
    assert_eq!(
        &keystream[64..],
        &bytes::<64>(&fields, "verify.keystream1")[..]
    );
}

#[test]
#[ignore = "needs the upstream C oracle; see tools/oracle/README.md"]
fn pair_setup_nonces_and_gcm_match_upstream() {
    let fields = oracle("crypto");
    let session_key: [u8; 40] = bytes(&fields, "setup.session_key");
    assert_eq!(session_key, sequence::<40>());

    let keys = derive_pair_setup_keys(&session_key);
    assert_eq!(keys.key, bytes::<16>(&fields, "setup.key"));

    let (decrypt_iv, encrypt_iv) = pair_setup_gcm_nonces(&session_key);
    assert_eq!(decrypt_iv, bytes::<16>(&fields, "setup.decrypt_iv"));
    assert_eq!(encrypt_iv, bytes::<16>(&fields, "setup.encrypt_iv"));

    let client_public = sequence::<32>();
    let mut server_public = [0u8; 32];
    for (index, byte) in server_public.iter_mut().enumerate() {
        *byte = index as u8 + 0x20;
    }

    let (epk, tag) = aes128_gcm16_encrypt(&keys.key, &decrypt_iv, &client_public).expect("seal");
    assert_eq!(epk, hex_of(&fields, "setup.request_epk"));
    assert_eq!(hex(&tag), field(&fields, "setup.request_tag"));

    let (epk, tag) = aes128_gcm16_encrypt(&keys.key, &encrypt_iv, &server_public).expect("seal");
    assert_eq!(epk, hex_of(&fields, "setup.reply_epk"));
    assert_eq!(hex(&tag), field(&fields, "setup.reply_tag"));
    assert_eq!(
        aes128_gcm16_decrypt(&keys.key, &encrypt_iv, &epk, &tag).expect("open"),
        server_public
    );
}

/// The oracle prints hex; a few fields are compared as text.
fn hex_of(fields: &HashMap<String, String>, key: &str) -> Vec<u8> {
    let text = field(fields, key);
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex"))
        .collect()
}

#[test]
#[ignore = "needs the upstream C oracle; see tools/oracle/README.md"]
fn fairplay_matches_upstream() {
    let fields = oracle("fairplay");
    let mut session = FairPlaySession::new();

    let mut request = [0u8; 16];
    request[4] = 0x03;
    let response = session.setup(&request).expect("setup");
    assert_eq!(hex(&response), field(&fields, "fairplay.setup_response"));

    let mut handshake = [0u8; 164];
    handshake[4] = 0x03;
    let response = session.handshake(&handshake).expect("handshake");
    assert_eq!(
        hex(&response),
        field(&fields, "fairplay.handshake_response")
    );

    let encrypted = sequence::<72>();
    let key = session.decrypt(&encrypted).expect("decrypt");
    assert_eq!(hex(&key), field(&fields, "fairplay.decrypted_key"));
}

/// The mirror stream: the keys its two hashes derive, and the CTR rule its packets
/// follow.
///
/// The mirror stream: the keys its two hashes derive, and the CTR rule its packets
/// follow.
///
/// `mirror_buffer_decrypt` XORs a packet with keystream bytes, so the oracle decrypts
/// all-zero packets of the lengths the port walks and prints exactly the bytes
/// upstream read — where in the stream they are. It also checks that upstream's own
/// decrypt gives back a plaintext encrypted with those bytes, so the vector says
/// what upstream does rather than what this file believes it does. On this side the
/// port has to read the same keystream bytes for the same packets, recover the
/// ciphertext, and carry the right number of bytes between packets.
///
/// The first two lengths are the fixed vector the slice was judged on: 40 bytes
/// followed by 24, which is where a receiver that loses the tail block's unused
/// keystream goes wrong.
#[test]
#[ignore = "needs the upstream C oracle; see tools/oracle/README.md"]
fn mirror_stream_matches_upstream() {
    let fields = oracle("mirror");
    let connection_id: u64 = field(&fields, "mirror.connection_id")
        .parse()
        .expect("the oracle prints the connection identifier");
    assert_eq!(connection_id, 2_765_102_747_429_192_913);
    let audio_key = [0x5a; 16];

    let keys = stream_keys(connection_id, &audio_key);
    assert_eq!(keys.key, bytes::<16>(&fields, "mirror.key"));
    assert_eq!(keys.iv, bytes::<16>(&fields, "mirror.iv"));

    let decryptor = MirrorDecryptor::new(&keys).expect("decryptor");
    // The keystream from the IV, block for block, as upstream's own CTR context
    // generates it.
    let upstream = hex_of(&fields, "mirror.keystream");
    assert_eq!(
        decryptor.keystream(0, upstream.len() / 16),
        upstream,
        "the keystream from the IV"
    );

    // The lengths the oracle walks: the fixed vector, a length that is never block
    // aligned, a whole number of blocks, and small packets.
    let lengths = [40usize, 24, 33, 47, 1, 64, 5, 128];
    for (step, length) in lengths.into_iter().enumerate() {
        assert_eq!(
            field(&fields, &format!("mirror.length{step}")),
            length.to_string(),
            "the oracle and this test walk the same lengths"
        );
        let model = hex_of(&fields, &format!("mirror.model{step}"));
        assert_eq!(
            model.len(),
            length,
            "the keystream a packet of {length} reads"
        );

        // Every packet is read from a stream that starts at the IV: they are not a
        // sequence, so how far the counter had moved is not part of the vector.
        let mut decryptor = MirrorDecryptor::new(&keys).expect("decryptor");
        let mut read = vec![0u8; length];
        decryptor
            .decrypt(&vec![0u8; length], &mut read)
            .unwrap_or_else(|error| panic!("packet of {length}: {error}"));
        assert_eq!(
            read, model,
            "a packet of {length} reads different keystream bytes than upstream"
        );
        assert_eq!(
            decryptor.carried(),
            (16 - length % 16) % 16,
            "the bytes a packet of {length} leaves for the next one"
        );

        // And the plaintext the oracle encrypted with those bytes comes back.
        let expected = hex_of(&fields, &format!("mirror.plaintext{step}"));
        let ciphertext = hex_of(&fields, &format!("mirror.ciphertext{step}"));
        assert_eq!(expected.len(), length);
        assert_eq!(ciphertext.len(), length);
        assert_eq!(
            ciphertext
                .iter()
                .zip(&expected)
                .zip(&model)
                .map(|((c, p), k)| c ^ p ^ k)
                .collect::<Vec<u8>>(),
            vec![0u8; length],
            "the oracle's ciphertext is its plaintext XOR its keystream"
        );
        let mut plaintext = vec![0u8; length];
        let mut decryptor = MirrorDecryptor::new(&keys).expect("decryptor");
        decryptor
            .decrypt(&ciphertext, &mut plaintext)
            .unwrap_or_else(|error| panic!("packet of {length}: {error}"));
        assert_eq!(
            plaintext, expected,
            "packet of {length} decrypted differently from upstream"
        );
    }
}
