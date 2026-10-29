//! What the references of the differential harness of the 68010 and 68020
//! dialects, Musashi and Moira, and the Interpreter are compared on.

/// The registers of a 68010 or a 68020 the harness sets and compares: A7 is
/// the stack pointer of the mode `sr` says, `usp`, `ssp` or on a 68020
/// `msp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// D0 to D7.
    pub d: [u32; 8],
    /// A0 to A6.
    pub a: [u32; 7],
    /// The user stack pointer.
    pub usp: u32,
    /// The supervisor stack pointer, the 68020's interrupt stack pointer.
    pub ssp: u32,
    /// The 68020's master stack pointer, 0 on a 68010.
    pub msp: u32,
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
    /// The 68020's cache control register, 0 on a 68010.
    pub cacr: u32,
    /// The 68020's cache address register, 0 on a 68010.
    pub caar: u32,
}
