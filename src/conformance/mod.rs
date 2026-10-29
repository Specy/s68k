//! The conformance harness: the Interpreter checked against the
//! SingleStepTests/m68000 suite, generated from MAME's microcoded 68000, and
//! against Musashi's self-checking programs (ADR 0010).
//!
//! * [`suite`] reads the suite's binary files and cuts the vendored subset;
//! * [`runner`] runs a test from its initial state and compares the final
//!   one, and runs one of Musashi's programs to its verdict;
//! * [`skips`] is the one list of what is left out or compared less, with a
//!   reason for each;
//! * `musashi`, behind the `musashi` feature, is Musashi as a 68010, the
//!   reference of the differential harness of the 68010 dialect, which has
//!   no suite of its own, and `moira`, behind the `moira` feature, is Moira
//!   as a 68010, its reference for cycles; `reference` is the state the two
//!   and the Interpreter are compared on.
//!
//! The crate's tests run the vendored subset, `tests/m68000/`, in every
//! `cargo test`, and the whole suite from a fetched copy with
//! `cargo test --release --features conformance conformance:: -- --ignored`
//! (`scripts/fetch-m68000-tests.sh`). It is compiled for the crate's tests
//! and behind the `conformance` feature, never into the WebAssembly build.

#[cfg(feature = "moira")]
pub mod moira;
#[cfg(feature = "musashi")]
pub mod musashi;
#[cfg(any(feature = "musashi", feature = "moira"))]
pub mod reference;
pub mod runner;
pub mod skips;
pub mod suite;
