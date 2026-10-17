# Working on this repository

Notes for an automated contributor.

## Layout

```
crates/rust-airplay/            the library
crates/rust-airplay-cli/        the command-line receiver
crates/rust-airplay-gstreamer/  the GStreamer renderer, excluded from the workspace
tools/oracle/                   a C oracle, for the differential tests
docs/                           API, build guide, architecture
upstream/UxPlay/                the reference checkout the oracle builds from; not vendored
```

## Gates

Every change passes these in the workspace:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The renderer is a crate of its own, so it has its own copy:

```sh
cargo fmt --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --check
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml
cargo clippy --manifest-path crates/rust-airplay-gstreamer/Cargo.toml --all-targets -- -D warnings
```

Two suites need more than the workspace, and are run when a change touches them:

```sh
pwsh -File tools/oracle/run.ps1
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

The first compares this implementation with UxPlay's own C code. Run it for changes to the
mirror keystream, the counter, audio decryption, the pairing vectors or the FairPlay
transforms.

## Rules

- No `unsafe`. The workspace forbids it.
- No C dependency in the default workspace. `cargo tree --workspace` must show no `-sys`
  crate.
- The library reads no environment variable, chooses no path and prints nothing. Values go
  in through `Config`; progress comes out through `log`.
- No blocking call in a receive loop. Rendering runs on threads fed by bounded queues.
- Every public item is documented. `missing_docs` is a warning and should be treated as an
  error.
- A comment explains why. It earns its place by recording a measurement, a rule taken from
  upstream, or a mistake that would otherwise be repeated.

## Documentation

- Every document except this one exists as `<name>.md` and `<name>.zh-CN.md`.
- No emoji.

## Upstream

`upstream/UxPlay` is a checkout at commit `57ea83411d5f7e0b38c5841987439340543f025c`. It is
not vendored and must not be edited.

When working from it:

1. Read the source, not the header defaults. A constant in a header is what a build starts
   from; the program may change it before use.
2. Cite the file and line where the behaviour is not obvious.
3. If a rule can be measured, add the measurement as a test.

## Tools

- `tools/oracle/` builds a C program from UxPlay and prints its intermediate values. Prefer
   it to reasoning when a rule is in doubt.
- `tools/eld_probe.c` checks that raw AAC-ELD frames decode, without a device.

## Commits

One commit per coherent change. The message says what was wrong and why the change fixes
it; if the change was measured, put the number in the message.