//! The Program: what the Assembler produces and the Interpreter runs.
//!
//! A Program exists only when no Diagnostic of the assembly is an error
//! (CONTEXT.md, "Program"). It holds four things and nothing else:
//!
//! * the **instructions**, each with its address, its encoding and the number
//!   of bytes it takes up, the [`Location`] it came from and the Source line as
//!   it was written;
//! * the initial **memory**, one run per Directive that puts bytes there or
//!   reserves room for them;
//! * the **Symbols**, by full name, with their kind and their definition;
//! * the **Entry point**, the address the program starts running at;
//! * the **Memory map**, the ranges its `memory` lines declare.
//!
//! It never holds source: an instruction reaches its line through its
//! [`Location`], which is what makes a per-File breakpoint and a per-File
//! current line possible (the design record, "Files, `include`, `incbin`").
//!
//! # The image
//!
//! What the Program puts in memory is its **image**: the bytes of every
//! instruction, as the 68000 reads them, and the bytes of every `dc`, `dcb` and
//! `incbin`. An instruction keeps its bytes beside the [`Instruction`] they
//! encode, so the code stays distinguishable from the data although both are
//! in memory, and [`Program::image`] puts the two together in address order.
//! A pad byte before an aligned item and the room `ds` reserves are not part
//! of it: neither is written, as in EASy68K.
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
    /// How many bytes it takes up: the length of its encoding, 2 to 10 bytes
    /// on a 68000, which the Layout's first pass decided by EASy68K's rules
    /// before the line was encoded.
    pub size: usize,
    /// Its encoding, the words a 68000 reads, as the bytes they are in memory:
    /// the operation word's high byte first. `simhalt` is `$FFFF $FFFF`.
    pub bytes: Vec<u8>,
    /// The instruction itself, as it was lowered: what the encoding decodes
    /// to, and what the Interpreter runs.
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

/// The kind of a range of the [Memory map](MemoryRange): what EASy68K's
/// simulator does with an access inside it (`docs/research/easy68k-machine-behaviour.md`
/// §9 in the asm-editor).
///
/// The order is EASy68K's, the order its assembler writes the ranges out in
/// and its hardware window lists them in, and so the order a Program keeps
/// them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryRangeKind {
    /// `rom`: a write is ignored.
    Rom,
    /// `read`: read only, a write is a bus error.
    Read,
    /// `protected`: any access from user mode is a bus error.
    Protected,
    /// `invalid`: any access is a bus error.
    Invalid,
}

impl MemoryRangeKind {
    /// The four, in EASy68K's order.
    pub const ALL: [MemoryRangeKind; 4] = [
        MemoryRangeKind::Rom,
        MemoryRangeKind::Read,
        MemoryRangeKind::Protected,
        MemoryRangeKind::Invalid,
    ];

    /// The word a `memory` line names the kind with, in lower case.
    pub fn name(self) -> &'static str {
        match self {
            MemoryRangeKind::Rom => "rom",
            MemoryRangeKind::Read => "read",
            MemoryRangeKind::Protected => "protected",
            MemoryRangeKind::Invalid => "invalid",
        }
    }

    /// The kind a word names, in any case, as EASy68K compares it (`strcmpi`).
    pub fn from_name(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| word.eq_ignore_ascii_case(kind.name()))
    }
}

/// One range of the **Memory map**, as `memory kind start,end` declares it.
///
/// Both ends are in the 24 bits of the 68000's address bus, as EASy68K masks
/// them, and both are inside the range: EASy68K tests a write with
/// `loc + bytes - 1 >= start && loc <= end`. A start above the end covers no
/// address. A Program holds at most one range of each kind, which is all
/// EASy68K's memory map has room for.
///
/// What an access inside it does is the Interpreter's to enforce, which it does
/// not do yet; the Program carries the ranges so that they travel with the
/// source and reach the S-record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryRange {
    /// What kind of range it is.
    pub kind: MemoryRangeKind,
    /// Its first address.
    pub start: u32,
    /// Its last address, which is in it.
    pub end: u32,
    /// The `memory` line that declared it.
    pub location: Location,
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
    #[serde(skip)]
    layout_items: Vec<u32>,
    memory: Vec<MemoryRun>,
    symbols: BTreeMap<String, ProgramSymbol>,
    entry: usize,
    memory_map: Vec<MemoryRange>,
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
            layout_items: Vec::new(),
            instructions,
            memory,
            symbols,
            entry,
            memory_map: Vec::new(),
        }
    }

    /// The Program with the ranges its `memory` lines declared, at most one of
    /// each kind, put in EASy68K's order.
    pub fn with_memory_map(mut self, mut ranges: Vec<MemoryRange>) -> Self {
        ranges.sort_by_key(|range| range.kind);
        self.memory_map = ranges;
        self
    }

    /// The Memory map: the ranges the `memory` lines declared, in EASy68K's
    /// order, ROM, read only, protected and invalid, each kind at most once.
    pub fn memory_map(&self) -> &[MemoryRange] {
        &self.memory_map
    }

    pub fn with_layout_items(mut self, items: Vec<u32>) -> Self { self.layout_items = items; self }
    /// Flat tuples: address, length, kind, section number, alignment.
    pub fn layout_items(&self) -> &[u32] { &self.layout_items }

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

    /// The Program as the S-record file EASy68K's assembler writes, `.S68`:
    /// the header, the [image](Program::image), the Memory map and the Entry
    /// point ([`srecord`](super::srecord)). EASy68K's simulator loads it.
    pub fn to_srecord(&self) -> String {
        super::srecord::write(self)
    }

    /// Everything the Program writes into memory, code and data, in address
    /// order: one run for each stretch of consecutive bytes.
    ///
    /// The instructions' encodings and the bytes of `dc`, `dcb` and `incbin`
    /// are merged where one ends at the address the next starts at, so a
    /// program's code is one run however many instructions it has. A pad byte
    /// and the room `ds` reserves are not written, so they end a run.
    pub fn image(&self) -> Vec<ImageRun> {
        let mut pieces: Vec<(usize, &[u8])> = self
            .instructions
            .iter()
            .map(|instruction| (instruction.address, instruction.bytes.as_slice()))
            .chain(
                self.memory
                    .iter()
                    .filter_map(|run| run.bytes().map(|bytes| (run.address, bytes))),
            )
            .filter(|(_, bytes)| !bytes.is_empty())
            .collect();
        pieces.sort_by_key(|(address, _)| *address);
        let mut runs: Vec<ImageRun> = Vec::new();
        for (address, bytes) in pieces {
            match runs.last_mut() {
                Some(run) if run.address + run.bytes.len() == address => {
                    run.bytes.extend_from_slice(bytes)
                }
                _ => runs.push(ImageRun {
                    address,
                    bytes: bytes.to_vec(),
                }),
            }
        }
        runs
    }
}

/// One stretch of consecutive bytes of a Program's [image](Program::image).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageRun {
    /// The address of its first byte.
    pub address: usize,
    /// The bytes, in the order they sit in memory.
    pub bytes: Vec<u8>,
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
            size: 2,
            bytes: vec![0x4E, 0x71],
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
            0x2002,
            "the program ends after the last instruction, not on it"
        );
        assert_eq!(program.entry(), 0x1000);
    }

    #[test]
    fn the_calling_instruction_is_found_by_the_address_it_returns_to() {
        let program = Program::new(
            vec![instruction(0x1000, 3), instruction(0x1002, 4)],
            Vec::new(),
            &SymbolTable::new(),
            0x1000,
        );
        assert_eq!(
            program
                .instruction_ending_at(0x1002)
                .map(|instruction| instruction.address),
            Some(0x1000),
            "a return to $1002 comes back from the nop at $1000"
        );
        assert_eq!(
            program.instruction_ending_at(0x1004).map(|i| i.address),
            Some(0x1002)
        );
        assert!(
            program.instruction_ending_at(0x1001).is_none(),
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
    fn the_image_merges_code_and_data_that_touch_and_leaves_the_rest_unwritten() {
        let data = |address: usize, content: MemoryContent| MemoryRun {
            address,
            content,
            location: location(9),
        };
        let program = Program::new(
            vec![instruction(0x1002, 4), instruction(0x1000, 3)],
            vec![
                data(0x1004, MemoryContent::Bytes { bytes: vec![1, 2, 3] }),
                // The pad byte at $1007 is not written, so the word after it
                // starts a run of its own.
                data(0x1008, MemoryContent::Bytes { bytes: vec![4, 5] }),
                data(0x100A, MemoryContent::Reserved { length: 6 }),
                data(0x1010, MemoryContent::Bytes { bytes: vec![] }),
                data(0x2000, MemoryContent::Bytes { bytes: vec![6] }),
            ],
            &SymbolTable::new(),
            0x1000,
        );
        assert_eq!(
            program.image(),
            vec![
                ImageRun {
                    address: 0x1000,
                    bytes: vec![0x4E, 0x71, 0x4E, 0x71, 1, 2, 3],
                },
                ImageRun {
                    address: 0x1008,
                    bytes: vec![4, 5],
                },
                ImageRun {
                    address: 0x2000,
                    bytes: vec![6],
                },
            ]
        );
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
