# Upstream differential oracle

`oracle.c` compiles the pinned upstream C sources of UxPlay into one small
program and prints fixed vectors. The Rust tests under
`crates/airplay-core/tests/upstream_oracle.rs` and the ignored test
`srp::tests::upstream_verifier_accepts_our_proof` run it and compare its answers
with this port, so protocol claims are checked against the C implementation
instead of against a second reading of it.

The oracle is a test tool. It is never linked into the product, and it is built
from the reference checkout in `upstream/UxPlay` (which is ignored by the root
repository) only.

## Build

Needs the MSYS2/UCRT64 toolchain (`gcc`, OpenSSL headers and `libcrypto`, all
present in `C:\msys64\ucrt64` on the development machine):

```powershell
pwsh tools/oracle/build.ps1
```

`-DWIN32` is deliberate: upstream's `srp.c` reads `/dev/urandom` in its POSIX
branch, which does not exist on Windows, and its Windows branch uses
`advapi32` instead.

## Run

```powershell
tools/oracle/oracle.exe crypto        # pair-verify keystream, pair-setup nonces
tools/oracle/oracle.exe fairplay      # PlayFair setup/handshake/decrypt
tools/oracle/oracle.exe srp-init      # salt, verifier, server public value
tools/oracle/oracle.exe srp-init short  # the same with a 255-byte public value
tools/oracle/oracle.exe srp-verify <salt> <verifier> <b> <A> <M>
```

Then:

```powershell
$env:AIRPLAY_UPSTREAM_ORACLE = "$PWD/tools/oracle/oracle.exe"
cargo test -p airplay-core --test upstream_oracle -- --ignored --nocapture
cargo test -p airplay-core --lib -- --ignored upstream_verifier --nocapture
```

## What this settles

| Vector | Claim it checks |
|---|---|
| `verify.keystream0/1` | The server signs with the first 64 bytes of the shared AES-CTR keystream and the client's signature arrives in the second 64 (`pairing.c:414-415`, `:440-443`) |
| `setup.decrypt_iv` / `setup.encrypt_iv` | Pair-Setup AES-GCM uses the base IV incremented once for the request and twice for the reply (`pairing.c:615`, `:636`) |
| `setup.request_*` / `setup.reply_*` | Both GCM directions produce the bytes this port produces, tag included |
| `fairplay.setup_response` | PlayFair's 142-byte setup reply is byte-identical |
| `fairplay.handshake_response` | PlayFair's 32-byte handshake reply is byte-identical |
| `fairplay.decrypted_key` | The FairPlay key unwrap of a 72-byte key is byte-identical |
| `srp.server_public` | The server public value is the minimal encoding, also when it is 255 bytes long |
| `srp-verify` accepting our proof | Upstream's own verifier authenticates the proof this port computes, and derives the same server proof and session key |

## Notes

- `srp_init` lets upstream generate the salt, so the salt differs between runs;
  the tests use the values of the run they started, which is why `srp-init` and
  `srp-verify` are two invocations of the same run.
- `srp_verifier_new` writes through `bytes_B`/`len_B` unconditionally, so those
  arguments cannot be `NULL` (the oracle passes real pointers).
- The oracle's `#` lines are diagnostics for a failing run; the tests ignore
  anything that is not a `key = value` line.
