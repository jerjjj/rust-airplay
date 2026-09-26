# Upstream provenance and third-party notices

This project targets behavior of [FDH2/UxPlay](https://github.com/FDH2/UxPlay) at commit `57ea83411d5f7e0b38c5841987439340543f025c` (2026-09-17). The local research checkout is under `upstream/UxPlay`, is ignored by the root repository, and is not linked into the Rust product.

The Rust migration is licensed under GPL-3.0-or-later. UxPlay incorporates sources with GPL, LGPL and MIT notices. This document does not relicense those components. Retain original copyright and license notices whenever translating or incorporating their implementation/data. Before each such migration, record the exact source files and notices here.

The FairPlay framing constants are translated from `upstream/UxPlay/lib/fairplay_playfair.c`; PlayFair table assets and the FairPlay transforms are translated from `upstream/UxPlay/lib/playfair/` (`omg_hax.{c,h}`, `modified_md5.c`, `playfair.c`, `sap_hash.c`, `hand_garble.c`). `fairplay_playfair.c` carries the LGPL-2.1-or-later notice of Juho Vähä-Herttua; the `playfair/` files carry no per-file header, so their Rust translations (`playfair.rs`, `playfair_tables.rs`, `playfair/hand_garble_generated.rs`) use the same LGPL-2.1-or-later notice. These files are retained locally for provenance and each Rust translation keeps the applicable upstream SPDX notice. Test vectors constructed in this project are synthetic and are not device captures.

The analysis in `research/` includes third-party README snapshots for evidence, not runtime code. Their upstream URLs and retrieval times are in `research/evidence/documents.json`; original links and attribution remain in those snapshots. `crates.json` records the queried package metadata. They do not establish a claim of tested integration or change any upstream license.

To obtain the reference checkout on a new machine:

```powershell
git clone https://github.com/FDH2/UxPlay.git upstream/UxPlay
git -C upstream/UxPlay checkout --detach 57ea83411d5f7e0b38c5841987439340543f025c
git -C upstream/UxPlay rev-parse HEAD
```

Original lineage includes UxPlay, RPiPlay, AirplayServer, shairplay and PlayFair. See the pinned upstream README author list and per-file notices for detailed attribution. The GPL license text in this repository is copied verbatim from the reference checkout's `LICENSE`.

