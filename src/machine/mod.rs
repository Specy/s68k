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
//!   and prints a run of bytes;
//! * [`cache`] keeps the instructions the Interpreter runs, decoded once, by
//!   the address they start at;
//! * [`exception`] says what an exception pushes and where it goes: the
//!   vectors, the frames and the program counter the 68000 stacks;
//! * [`memory_map`] says what the Program's Memory map does with an access:
//!   ignore it, refuse it with a bus error, or let it be;
//! * [`interrupts`] holds the IRQ latch, the stopped processor and what the
//!   program set up for its interrupt requests, and which request is taken
//!   when;
//! * [`timing`] says how many cycles an instruction, an exception and an IRQ
//!   take.
//!
//! Both are written for a [`Processor`], or a [`Dialect`], a processor with
//! the 68020's bit fields or without. All three are decoded in full, the
//! 68020 but for its module call; the later two are dialects of the 68000,
//! and each row of the decoder's table names the first processor that has
//! it, so that a word one of them adds is an illegal instruction on the
//! processors before it. The frames and the timing are each processor's.
//!
//! [`Instruction`]: crate::instructions::Instruction

pub mod cache;
pub mod decode;
pub mod disassemble;
pub mod encode;
pub mod exception;
pub mod interrupts;
pub mod memory_map;
pub mod timing;

use serde::Serialize;

/// The processor a Program is written for, and that words are decoded for: a
/// **Processor dialect** (CONTEXT.md).
///
/// The three are one family, each a superset of the one before for a program
/// in user mode, so they are ordered: `processor >= Processor::M68010` asks
/// whether a processor has what the 68010 added. The 68000 is the default,
/// as it is EASy68K's only processor; a Program chooses another with
/// `opt p=68010` or `opt p=68020` (ADR 0011).
///
/// It serialises as its [`name`](Processor::name), `"68000"`, which is how the
/// TypeScript side spells a processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
pub enum Processor {
    /// The 68000, which EASy68K simulates.
    #[default]
    #[serde(rename = "68000")]
    M68000,
    /// The 68010: the 68000 with `move ccr,<ea>`, `rtd`, `movec`, `moves`,
    /// `bkpt`, a vector base register and loop mode.
    #[serde(rename = "68010")]
    M68010,
    /// The 68020, as a 68EC020 with a 24-bit bus: the 68010 with new
    /// addressing modes, 32-bit branches and multiplies, bit fields and more.
    #[serde(rename = "68020")]
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

    /// Whether the cycle counter counts this processor's exact time, as it
    /// does the 68000's and the 68010's; the 68020's is an estimate, the
    /// cache case of its manual's tables, since its real time depends on its
    /// instruction cache and on how neighbouring instructions overlap
    /// ([`timing`]). A Program's own answer is
    /// [`Program::cycles_exact`](crate::assembler::program::Program::cycles_exact),
    /// which is an estimate too for the bit fields `opt bit` gives a 68000 or a
    /// 68010.
    pub fn cycles_exact(self) -> bool {
        self != Processor::M68020
    }
}

/// What words are decoded for: a [`Processor`], and whether the 68020's
/// bit-field instructions are in, which EASy68K's `opt bit` gives a 68000
/// Program too, with the 68000's addressing modes (ADR 0011). A processor
/// alone is its own dialect: the 68020 with its bit fields, the others
/// without.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dialect {
    /// The processor.
    pub processor: Processor,
    /// Whether it has the bit-field instructions.
    pub bit_fields: bool,
}

impl From<Processor> for Dialect {
    fn from(processor: Processor) -> Self {
        Dialect {
            processor,
            bit_fields: processor.has(Processor::M68020),
        }
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
        assert!(Processor::M68000.cycles_exact() && Processor::M68010.cycles_exact());
        assert!(
            !Processor::M68020.cycles_exact(),
            "the 68020's are an estimate"
        );
        let names: Vec<_> = Processor::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["68000", "68010", "68020"]);
        for processor in Processor::ALL {
            assert_eq!(
                serde_json::to_value(processor).unwrap(),
                processor.name(),
                "a processor serialises as its name"
            );
        }
    }
}
