//! Everything s68k knows about the 68000's instructions.
//!
//! Four modules, in the order the front end uses them:
//!
//! * [`table`] — one row a Mnemonic: the Operand rules per position, the sizes
//!   and the default, the range of the instruction's own count or vector, and
//!   the [`Family`] that encodes it. It is the single source of
//!   truth of [ADR
//!   0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md),
//!   asked one question by the parser ("is this word a Mnemonic?") and all the
//!   others by the analyzer.
//! * [`lowering`] — from a checked Operation to an
//!   [`Instruction`], choosing among the 68000's encodings as EASy68K does.
//! * [`choice`] — EASy68K's rules for those choices, which read whether an
//!   Operand is a back reference and its value, and nothing else.
//! * [`encoded`] — the Instruction itself and the types it is made of, which
//!   used to be `src/instructions.rs`'s and which that module now re-exports.

pub mod choice;
pub mod encoded;
pub mod lowering;
pub mod table;

pub use encoded::{
    AddressWidth, BranchSize, Condition, IndexRegister, Instruction, Operand, RegisterOperand,
    ShiftDirection, Sign, Size, TargetDirection,
};
pub use lowering::{lower, lower_operand, lower_size, Lowered, Site, Values};
pub use table::{
    is_mnemonic, lookup, mnemonics, Family, Form, Implementation, InstructionSpec, Modes, SizeRule,
    ValueRule, TABLE,
};
