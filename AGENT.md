# Working on this repository

Instructions for an automated contributor. Humans are welcome to read it too; it says how
the project is built and what it will and will not accept.

## Layout

```
crates/rust-airplay/            the library: protocol, media session, renderer boundary
crates/rust-airplay-cli/        the program: arguments, files, printing
crates/rust-airplay-gstreamer/  a renderer; excluded from the workspace
tools/oracle/                   a C oracle built from UxPlay, for differential tests
tools/eld_probe.c               a standalone check that raw AAC-ELD frames decode
docs/                           architecture, API, build guide, each in English and Chinese
upstream/UxPlay/                the reference checkout; ignored by git, not vendored
```

## The gates

Every change must pass all four, in the workspace and, separately, in the GStreamer crate:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

```sh
cargo fmt --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --check
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml
cargo clippy --manifest-path crates/rust-airplay-gstreamer/Cargo.toml --all-targets -- -D warnings
```

The differential suite is separate because it needs a C toolchain and an upstream checkout:

```sh
pwsh -File tools/oracle/run.ps1
```

Run it whenever a change touches the mirror keystream, the counter, the audio decryption,
pairing vectors or the FairPlay transforms. It is the only thing that can tell you a
plausible implementation is wrong.

## Rules the code follows

- **No `unsafe`.** The workspace forbids it. A backend that needed FFI would be a separate
  crate with its own review, not an exception here.
- **No C dependency in the default workspace.** `cargo tree --workspace` must show no `-sys`
  crate. Anything that links a C library belongs in `rust-airplay-gstreamer` or a crate like
  it.
- **The library does not read the environment, choose a path, or print.** Values go in
  through `Config`; progress comes out through `log`. If a change needs the environment, it
  belongs in the CLI.
- **No blocking call in a receive loop.** Rendering happens on threads fed by bounded
  queues. A receive loop that blocks stops answering the client that is sending to it.
- **Every public item is documented.** `missing_docs` is a warning today and should be
  treated as an error.
- **Comments explain why, not what.** The code says what it does. A comment earns its place
  by recording a measurement, a rule taken from upstream, or a mistake that would otherwise
  be repeated.

## Documentation rules

- Every document except this one exists in English and Chinese, as `<name>.md` and
  `<name>.zh-CN.md`.
- No emoji, in any document.
- Claims about what has been verified say which platform and which device. macOS and Linux
  have never been run against a real device; do not imply otherwise.
- Measurements are given with the thing measured, not as adjectives.

## Working with upstream

`upstream/UxPlay` is a checkout of the reference implementation, at the commit recorded in
the pinned commit 57ea83411d5f7e0b38c5841987439340543f025c. It is not vendored and must not be edited.

When porting behaviour from it:

1. Read the upstream source, not the header defaults. A constant in a header is what a
   build starts from; the running program may change it. This has cost more time than any
   other mistake in this project.
2. Cite the file and line in a comment where the behaviour is non-obvious.
3. If a rule can be measured, measure it and commit the measurement as a test. The mirror
   counter rule and the audio delivery shape both became tests for this reason.

## What is not verified

State these plainly rather than working around them:

- Screen mirroring and audio playback: Windows, real iPad, MSYS2 UCRT64 GStreamer 1.28.
- Everything else: test suite only.
- A/V synchronisation is not implemented. Both paths play at arrival rate.
- `set_volume` reaches the GStreamer backend's memory but not a live pipeline.

## Commit style

One commit per coherent change. The message explains what was wrong and why the change
 fixes it, in that order; a summary of the files touched is not useful. If a change was
measured, put the number in the message.

## Tools available

`tools/oracle` builds a C binary from the upstream sources and prints upstream's own
intermediate values. Prefer it to reasoning when a rule is in doubt. `tools/eld_probe.c`
does the same for audio decoding, without needing a device.
