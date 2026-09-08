//! Everything s68k knows about the 68000's instructions.
//!
//! Three modules, in the order the front end uses them:
//!
//! * [`table`] — one row a Mnemonic: the Operand rules per position, the sizes
//!   and the default, the range of the instruction's own count or vector, and
//!   the [`Family`] that encodes it. It is the single source of
//!   truth of [ADR
//!   0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md),
//!   asked one question by the parser ("is this word a Mnemonic?") and all the
//!   others by the analyzer.
//! * [`lowering`] — from a checked Operation to an
//!   [`Instruction`], normalising as
//!   `tests/corpus/README.md` records.
//! * [`encoded`] — the Instruction itself and the types it is made of, which
//!   used to be `src/instructions.rs`'s and which that module now re-exports.

pub mod encoded;
pub mod lowering;
pub mod table;

pub use encoded::{
    Condition, IndexRegister, Instruction, Operand, RegisterOperand, ShiftDirection, Sign, Size,
    TargetDirection,
};
pub use lowering::{lower, lower_operand, lower_size, Values};
pub use table::{
    is_mnemonic, lookup, mnemonics, Family, Form, Implementation, InstructionSpec, Modes, SizeRule,
    ValueRule, TABLE,
};
