//! The 68000 as a machine: the words an instruction is made of, and the
//! processor that reads them.
//!
//! The rest of the crate was written for a Program that holds [`Instruction`]
//! values and never their encoding. This module is where the encoding lives:
//!
//! * [`decode`] reads the words of one instruction, wherever they come from,
//!   and answers the [`Instruction`] they encode and its size;
//! * [`encode`] writes the words of an [`Instruction`] at an address, and is
//!   the decoder's inverse;
//! * [`disassemble`] prints an [`Instruction`] in EASy68K's syntax, and reads
//!   and prints a run of bytes.
//!
//! Both are written for a [`Processor`]. Only the 68000 is decoded in full;
//! the 68010 and the 68020 are dialects of it, and each row of the decoder's
//! table names the first processor that has it, so that a word one of them
//! adds is an illegal instruction on the processors before it.
//!
//! [`Instruction`]: crate::instructions::Instruction

pub mod decode;
pub mod disassemble;
pub mod encode;

use serde::Serialize;

/// The processor a Program is written for, and that words are decoded for: a
/// **Processor dialect** (CONTEXT.md).
///
/// The three are one family, each a superset of the one before for a program
/// in user mode, so they are ordered: `processor >= Processor::M68010` asks
/// whether a processor has what the 68010 added. The 68000 is the default,
/// as it is EASy68K's only processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
pub enum Processor {
    /// The 68000, which EASy68K simulates.
    #[default]
    M68000,
    /// The 68010: the 68000 with `move ccr,<ea>`, `rtd`, `movec`, `moves`,
    /// `bkpt`, a vector base register and loop mode.
    M68010,
    /// The 68020, as a 68EC020 with a 24-bit bus: the 68010 with new
    /// addressing modes, 32-bit branches and multiplies, bit fields and more.
    M68020,
}

impl Processor {
    /// The three, oldest first.
    pub const ALL: [Processor; 3] = [Processor::M68000, Processor::M68010, Processor::M68020];

    /// The name a person writes, `68000`, `68010` or `68020`: the number
    /// `opt p=` takes.
    pub fn name(self) -> &'static str {
        match self {
            Processor::M68000 => "68000",
            Processor::M68010 => "68010",
            Processor::M68020 => "68020",
        }
    }

    /// Whether this processor has what `first` introduced: the gate every
    /// dialect-specific instruction and decoder row goes through.
    pub fn has(self, first: Processor) -> bool {
        self >= first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_processor_has_what_the_ones_before_it_introduced() {
        assert!(Processor::M68000.has(Processor::M68000));
        assert!(!Processor::M68000.has(Processor::M68010));
        assert!(Processor::M68010.has(Processor::M68000));
        assert!(!Processor::M68010.has(Processor::M68020));
        assert!(Processor::M68020.has(Processor::M68010));
        assert_eq!(Processor::default(), Processor::M68000);
        let names: Vec<_> = Processor::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["68000", "68010", "68020"]);
    }
}
