# The vendored subset of SingleStepTests/m68000

The conformance oracle of the Interpreter (ADR 0010): tests generated from
MAME's microcoded 68000, each the state before one instruction, the state
after it and its bus cycles.

- **Source:** https://github.com/SingleStepTests/m68000, at commit
  `64b253116a3de04aaac4346c43680960dc9b67e5` (2024-07-31), the folder `v1`.
  MIT, "Copyright (c) 2024 SingleStepTests": the `LICENSE` beside this file
  is the suite's own.
- **The cut:** the first 50 tests of each of the 127 files, in the file's
  order, that raise no address error (no `re` or `we` cycle), 6,350 tests in
  all. Each test's bytes are copied unchanged under a new count, so the files
  are in the suite's own binary format and the reader that runs the whole
  suite reads these. They are 2.6 MB and not gzipped: git stores them
  compressed already, to about 1 MB, and reading gzip would need a crate.
- **Cut again** with `S68K_M68000_WRITE_CUT=1 cargo test --release
  the_vendored_subset_is_the_cut -- --ignored`, from the whole suite that
  `scripts/fetch-m68000-tests.sh` puts in `fetched/`, which git ignores.
  Without the variable, the same test checks that these files are that cut.

Every `cargo test` runs this subset (`src/test/conformance.rs`). The whole
suite runs from the fetched copy with

    scripts/fetch-m68000-tests.sh
    cargo test --release --features conformance conformance:: -- --ignored

and `S68K_M68000_ONLY=ASR.b,ASR.w` runs only the files it names. What is
skipped and compared less, and why, is `src/conformance/skips.rs`.

On 2026-10-28 all 5,733 tests of this subset that are not skipped pass, and
617 are skipped: 102 privilege violations, 100 `move usp`, 100 line A and F,
50 each of `reset`, `rte` and `stop`, 47 `trap #0` to `#14`, 46 `chk` out of
bounds, 40 mode changes, 29 `trapv` with V set and 3 `trap #15`. The whole
suite's counts are in the crate's README, "Conformance".
