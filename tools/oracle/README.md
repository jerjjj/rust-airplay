# The differential oracle

A small C program that links upstream sources and prints their intermediate values, so the
Rust implementation can be compared with the code it follows instead of with a description
of it.

## Build

It needs a UxPlay checkout at commit `57ea83411d5f7e0b38c5841987439340543f025c` in
`upstream/UxPlay`, and MSYS2 UCRT64:

```powershell
pwsh -File tools/oracle/build.ps1
```

## Run

```powershell
pwsh -File tools/oracle/run.ps1            # build if missing, then diff
pwsh -File tools/oracle/run.ps1 -Rebuild   # rebuild first
```

The exit code is the test result, so a pipeline can gate on it. It is the same as running:

```powershell
$env:AIRPLAY_UPSTREAM_ORACLE = "$PWD/tools/oracle/oracle.exe"
cargo test -p rust-airplay --test upstream_oracle -- --ignored
```

## What it checks

| Mode | What it prints |
|---|---|
| `srp` | SRP-6a intermediate values |
| `pairing` | the `/pair-verify` and `/pair-setup` vectors |
| `fairplay` | the FairPlay transforms |
| `mirror` | where the mirror decryptor takes its keystream from, packet by packet |

The `mirror` mode is the one to reach for first when a picture is wrong: it prints the
counter and the keystream position, so a disagreement says which of the two rules is wrong
rather than only that the output differs.