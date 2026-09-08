//! The Program: what the Assembler produces and the Interpreter runs.
//!
//! A Program exists only when no Diagnostic of the assembly is an error
//! (CONTEXT.md, "Program"). It holds four things and nothing else:
//!
//! * the **instructions**, each with its address, the number of bytes it takes
//!   up, the [`Location`] it came from and the Source line as it was written;
//! * the initial **memory**, one run per Directive that puts bytes there or
//!   reserves room for them;
//! * the **Symbols**, by full name, with their kind and their definition;
//! * the **Entry point**, the address the program starts running at.
//!
//! It never holds source: an instruction reaches its line through its
//! [`Location`], which is what makes a per-File breakpoint and a per-File
//! current line possible (the design record, "Files, `include`, `incbin`").
//!
//! # Looking an instruction up
//!
//! [`Program::instruction_at`] is what the Interpreter steps with. The
//! instructions are sorted by address and no two share one — two lines laid out
//! over the same address is an error, so a Program cannot hold that — which
//! makes the look-up a binary search over the list itself, with no index beside
//! it.

use std::collections::BTreeMap;

use serde::Serialize;

use super::instructions::encoded::Instruction;
use super::source::Location;
use super::symbols::{SymbolKind, SymbolTable, SymbolValue};

/// One assembled instruction: what it is, where it goes and where it came from.
///
/// It is serialised camelCase, like every other shape of the Assembler (the
/// design record, "Public API"); only `includeChain` has two words in it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssembledInstruction {
    /// The address it is laid out at, always even.
    pub address: usize,
    /// How many bytes it takes up. Every instruction is 4 bytes in this
    /// version (`tests/corpus/README.md`); the field is here so that real
    /// sizes are a change to the Assembler and not to everything that walks a
    /// Program.
    pub size: usize,
    /// The instruction itself.
    pub instruction: Instruction,
    /// The Source line it was written on.
    pub location: Location,
    /// The `include` lines it was reached through, innermost first, empty for
    /// an instruction of the Entry file.
    ///
    /// It is what lets the editor answer "through which `include` line did this
    /// instruction get here" (the design record, "Files, `include`, `incbin`"),
    /// which a Location alone cannot say once a File may be included twice: the
    /// two copies share a Location and differ only in this.
    pub include_chain: Vec<Location>,
    /// That line, as it was written, for a debugger to show.
    pub source: String,
}

/// What one Directive puts at its address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryContent {
    /// `dc` and `dcb`: the bytes themselves, in the order they sit in memory.
    Bytes {
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// `ds`: room reserved and **not** written. 1.4.2 wrote zeros over an
    /// eighth of it; the Directive's own documentation is explicit that
    /// "unlike DC, no data is stored in the reserved memory"
    /// (`Directives/ds.htm`).
    Reserved {
        /// How many bytes are reserved.
        length: usize,
    },
}

/// One run of initial memory: where it goes, what is there, and which line put
/// it there.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryRun {
    /// The address of its first byte.
    pub address: usize,
    /// The bytes, or the room reserved for them.
    pub content: MemoryContent,
    /// The Source line the Directive is on.
    pub location: Location,
}

impl MemoryRun {
    /// How many bytes of memory the run covers, written or reserved.
    pub fn len(&self) -> usize {
        match &self.content {
            MemoryContent::Bytes { bytes } => bytes.len(),
            MemoryContent::Reserved { length } => *length,
        }
    }

    /// Whether the run covers no memory at all (`ds.w 0`, the alignment
    /// idiom).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes the run writes, or `None` when it only reserves room.
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.content {
            MemoryContent::Bytes { bytes } => Some(bytes),
            MemoryContent::Reserved { .. } => None,
        }
    }
}

/// One Symbol of a built Program: what it stands for and where it was defined.
///
/// It is the flattened [`Symbol`](super::symbols::Symbol) of the symbol table:
/// a Register list's `value` is its `movem` mask, and a Variable's is the value
/// of its last definition. The name is the full name, so a Local label appears
/// as `start:loop`.
#[derive(Debug, Clone, Serialize)]
pub struct ProgramSymbol {
    /// The full name.
    pub name: String,
    /// Which of the four kinds it is.
    pub kind: SymbolKind,
    /// The value: an address for a Label, the mask for a Register list.
    pub value: i64,
    /// Where it was defined.
    pub location: Location,
}

/// A program ready to run.
#[derive(Debug, Clone, Serialize)]
pub struct Program {
    instructions: Vec<AssembledInstruction>,
    memory: Vec<MemoryRun>,
    symbols: BTreeMap<String, ProgramSymbol>,
    entry: usize,
}

impl Program {
    /// A Program from what the Layout worked out.
    ///
    /// The instructions and the memory runs are sorted by address, stably, so
    /// that two lines laid out over the same address keep their source order —
    /// which only a Program with an error in it can have, and which the
    /// fixtures of `tests/corpus/README.md` specify anyway.
    pub fn new(
        mut instructions: Vec<AssembledInstruction>,
        mut memory: Vec<MemoryRun>,
        symbols: &SymbolTable,
        entry: usize,
    ) -> Self {
        instructions.sort_by_key(|instruction| instruction.address);
        memory.sort_by_key(|run| run.address);
        let symbols = symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.name.clone(),
                    ProgramSymbol {
                        name: symbol.name.clone(),
                        kind: symbol.kind,
                        value: match symbol.value {
                            SymbolValue::Number(value) => value,
                            SymbolValue::RegisterList(mask) => mask as i64,
                        },
                        location: symbol.location.clone(),
                    },
                )
            })
            .collect();
        Self {
            instructions,
            memory,
            symbols,
            entry,
        }
    }

    /// Every instruction, in address order.
    pub fn instructions(&self) -> &[AssembledInstruction] {
        &self.instructions
    }

    /// Every run of initial memory, in address order.
    pub fn memory(&self) -> &[MemoryRun] {
        &self.memory
    }

    /// Every Symbol, by full name, sorted in byte order.
    pub fn symbols(&self) -> &BTreeMap<String, ProgramSymbol> {
        &self.symbols
    }

    /// The address the program starts running at.
    pub fn entry(&self) -> usize {
        self.entry
    }

    /// The instruction laid out at `address`, if one is.
    ///
    /// This is the Interpreter's step: an address that falls *between* two
    /// instructions has none, which is what makes a jump into the middle of the
    /// program a runtime error rather than a wrong instruction.
    pub fn instruction_at(&self, address: usize) -> Option<&AssembledInstruction> {
        self.instructions
            .binary_search_by_key(&address, |instruction| instruction.address)
            .ok()
            .map(|index| &self.instructions[index])
    }

    /// One past the last byte of the last instruction: where the program ends.
    ///
    /// A run that reaches this address has walked off the bottom of the program
    /// and stops. It is built from the last instruction's own
    /// [`size`](AssembledInstruction::size) and not from a fixed instruction
    /// width, so a later version that encodes real sizes needs no change here.
    /// An empty Program ends at 0, which is where it also starts.
    pub fn end_address(&self) -> usize {
        self.instructions
            .last()
            .map(|instruction| instruction.address + instruction.size)
            .unwrap_or(0)
    }

    /// The instruction whose last byte is just before `address`, if there is
    /// one.
    ///
    /// This is how the Interpreter finds the instruction that *called* a
    /// subroutine: the return address on the stack is one past the calling
    /// instruction, and the size stored with each instruction is what turns it
    /// back into the address the call was written at.
    pub fn instruction_ending_at(&self, address: usize) -> Option<&AssembledInstruction> {
        let index = self
            .instructions
            .partition_point(|instruction| instruction.address < address);
        self.instructions[..index]
            .last()
            .filter(|instruction| instruction.address + instruction.size == address)
    }

    /// Whether the Program holds no instruction at all, in which case there is
    /// nothing to run.
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::instructions::encoded::Instruction;

    fn location(line: usize) -> Location {
        Location::new("main.m68k", line, 0, 3)
    }

    fn instruction(address: usize, line: usize) -> AssembledInstruction {
        AssembledInstruction {
            address,
            size: 4,
            instruction: Instruction::NOP,
            location: location(line),
            include_chain: Vec::new(),
            source: "    nop".to_string(),
        }
    }

    #[test]
    fn a_program_is_sorted_by_address_and_looked_up_by_it() {
        let program = Program::new(
            vec![instruction(0x2000, 9), instruction(0x1000, 3)],
            Vec::new(),
            &SymbolTable::new(),
            0x1000,
        );
        assert_eq!(
            program
                .instructions()
                .iter()
                .map(|i| i.address)
                .collect::<Vec<_>>(),
            vec![0x1000, 0x2000]
        );
        assert_eq!(
            program.instruction_at(0x1000).map(|i| i.location.line),
            Some(3)
        );
        assert_eq!(
            program.instruction_at(0x2000).map(|i| i.location.line),
            Some(9)
        );
        assert!(
            program.instruction_at(0x1002).is_none(),
            "an address between two instructions has none"
        );
        assert_eq!(
            program.end_address(),
            0x2004,
            "the program ends after the last instruction, not on it"
        );
        assert_eq!(program.entry(), 0x1000);
    }

    #[test]
    fn the_calling_instruction_is_found_by_the_address_it_returns_to() {
        let program = Program::new(
            vec![instruction(0x1000, 3), instruction(0x1004, 4)],
            Vec::new(),
            &SymbolTable::new(),
            0x1000,
        );
        assert_eq!(
            program
                .instruction_ending_at(0x1004)
                .map(|instruction| instruction.address),
            Some(0x1000),
            "a return to $1004 comes back from the instruction at $1000"
        );
        assert_eq!(
            program.instruction_ending_at(0x1008).map(|i| i.address),
            Some(0x1004)
        );
        assert!(
            program.instruction_ending_at(0x1002).is_none(),
            "no instruction ends in the middle of another"
        );
        assert!(program.instruction_ending_at(0x1000).is_none());
        assert!(program.instruction_ending_at(0).is_none());
    }

    #[test]
    fn a_memory_run_says_what_it_writes_and_what_it_only_reserves() {
        let written = MemoryRun {
            address: 0x2000,
            content: MemoryContent::Bytes {
                bytes: vec![1, 2, 3],
            },
            location: location(1),
        };
        let reserved = MemoryRun {
            address: 0x2004,
            content: MemoryContent::Reserved { length: 8 },
            location: location(2),
        };
        assert_eq!(written.len(), 3);
        assert_eq!(written.bytes(), Some(&[1, 2, 3][..]));
        assert_eq!(reserved.len(), 8);
        assert_eq!(reserved.bytes(), None, "`ds` writes nothing");
        assert!(!reserved.is_empty());
    }

    #[test]
    fn the_symbols_come_out_flattened_and_sorted() {
        let mut symbols = SymbolTable::new();
        symbols
            .define(
                "start",
                None,
                SymbolKind::Label,
                SymbolValue::Number(0x1000),
                location(0),
                0,
            )
            .expect("a name defined once");
        symbols
            .define(
                "AllRegs",
                None,
                SymbolKind::RegisterList,
                SymbolValue::RegisterList(0b11),
                location(1),
                1,
            )
            .expect("a name defined once");
        let program = Program::new(Vec::new(), Vec::new(), &symbols, 0x1000);
        assert_eq!(
            program.symbols().keys().collect::<Vec<_>>(),
            vec!["AllRegs", "start"]
        );
        assert_eq!(program.symbols()["AllRegs"].value, 3);
        assert_eq!(program.symbols()["start"].kind, SymbolKind::Label);
        assert!(program.is_empty());
    }
}
