//! What the references of the differential harness of the 68010 dialect,
//! Musashi and Moira, and the Interpreter are compared on.

/// The registers of a 68010 the harness sets and compares: A7 is the stack
/// pointer of the mode `sr` says, `usp` or `ssp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// D0 to D7.
    pub d: [u32; 8],
    /// A0 to A6.
    pub a: [u32; 7],
    /// The user stack pointer.
    pub usp: u32,
    /// The supervisor stack pointer.
    pub ssp: u32,
    /// The status register.
    pub sr: u16,
    /// The program counter.
    pub pc: u32,
    /// The vector base register.
    pub vbr: u32,
    /// The source function code.
    pub sfc: u8,
    /// The destination function code.
    pub dfc: u8,
}
