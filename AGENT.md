# Working on this repository

This file describes how to build, test and change the code. It is written for an automated
contributor, so it states the gates and the rules plainly.

## Layout

```
crates/rust-airplay/            the library
crates/rust-airplay-cli/        the command-line receiver
crates/rust-airplay-gstreamer/  the GStreamer renderer, excluded from the workspace
tools/oracle/                   a C oracle built from UxPlay, for the differential tests
tools/eld_probe.c               a standalone check that raw AAC-ELD frames decode
docs/                           API, build guide, architecture; English and Chinese
upstream/UxPlay/                the checkout the oracle builds from; not vendored
```

## Gates

A change is finished when these pass in the workspace:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The renderer is excluded from the workspace because it links GStreamer, so it has its own
copy of the same gates:

```sh
cargo fmt --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --check
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml
cargo clippy --manifest-path crates/rust-airplay-gstreamer/Cargo.toml --all-targets -- -D warnings
```

Two suites need more than a Rust toolchain and are not part of the gates above. Run them
when a change touches what they cover.

```sh
pwsh -File tools/oracle/run.ps1
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

The first compares this implementation with UxPlay's own C code and needs a checkout at
`upstream/UxPlay`, MSYS2, and the commit named below. Run it for changes to the mirror
keystream, the counter, audio decryption, the pairing vectors or the FairPlay transforms: it
is the only test that can tell a plausible implementation from a correct one.

The second drives a real GStreamer runtime, including decoding frames captured from a
session.

## Rules

No `unsafe`. The workspace forbids it, and a backend that needed FFI would be a separate
crate with its own review rather than an exception here.

No C dependency in the default workspace. `cargo tree --workspace` must show no `-sys`
crate. Anything that links a C library belongs in a crate like `rust-airplay-gstreamer`.

The library reads no environment variable, chooses no path, and prints nothing. Values arrive
through `Config`; progress leaves through `log`. A change that wants the environment belongs
in the program.

No blocking call in a receive loop. Rendering happens on threads fed by bounded queues,
because a loop that blocks stops reading its socket, and the client stalls with it.

Every public item is documented. `missing_docs` is a warning today; in review, treat it as
an error.

A comment explains why. The code says what it does, so a comment earns its place by
recording a measurement, a rule taken from upstream, or a mistake that would otherwise be
repeated.

## Documentation

Every document except this one exists twice, as `<name>.md` and `<name>.zh-CN.md`. The
English version is the primary one.

No emoji, in any document.

Say what was run against a device and what was only tested. A claim that a platform works
should say which device it was verified on.

## Working from upstream

`upstream/UxPlay` is a checkout at commit `57ea83411d5f7e0b38c5841987439340543f025c`. It is
not vendored and must not be edited.

When porting behaviour from it, read the source rather than the header defaults: a constant
in a header is what a build starts from, and the program may change it before use. That
distinction has cost more time in this project than any other, and the mDNS feature bitmap is
the example to remember. Cite the file and line where behaviour is not obvious, and when a
rule can be measured, commit the measurement as a test.

## Tools

`tools/oracle/` builds a C program from the upstream sources and prints their intermediate
values. It is faster to ask it than to reason about the protocol when a rule is in doubt.

`tools/eld_probe.c` does the same for audio decoding: it feeds payloads to GStreamer the way
a live session does and reports how many decoded, without needing a device.

## Commits

One commit per coherent change. The message says what was wrong and why the change fixes it,
in that order. If the change was measured, the message carries the number. A list of files
touched is not useful to a reader later.