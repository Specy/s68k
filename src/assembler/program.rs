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
//! * the **Memory map**, the ranges its `memory` lines declare;
//! * the **Processor dialect** its `opt` line chose, the 68000 when it chose
//!   none, and whether it has the 68020's bit-field instructions.
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
//! # The address map
//!
//! The instructions are sorted by address and no two share one — two lines
//! laid out over the same address is an error, so a Program cannot hold that —
//! so the list is itself the map from an address to the line laid out there,
//! read by a binary search ([`Program::instruction_at`]). It is what the source
//! lookups read: the current line, a step's line, the Breakpoints' addresses.
//! The Interpreter runs from its decode cache, filled from the same list, and
//! never from this map, so a line whose bytes the program changes keeps its
//! Location: the bytes moved on, the source did not.
//!
//! The Program also keeps the ranges of the bytes it placed, code and data
//! ([`Program::places_any`]): `$FFFF $FFFF` inside them is a `simhalt`, and
//! outside them the end of the program (ADR 0009).

use std::collections::BTreeMap;

use serde::Serialize;

use super::ast::{self, SizeSuffix};
use super::instructions::encoded::{BranchSize, Instruction};
use super::source::Location;
use super::symbols::{SymbolKind, SymbolTable, SymbolValue};
use crate::machine::disassemble::{disassemble, Symbols};
use crate::machine::{Dialect, Processor};

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
/// What an access inside it does is the Interpreter's to enforce
/// ([`crate::machine::memory_map`]); the Program carries the ranges so that
/// they travel with the source and reach the S-record.
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
    /// The processor the source chose with `opt p=`, the 68000 by default.
    #[serde(skip)]
    processor: Processor,
    /// Whether the program has the 68020's bit-field instructions: `opt bit`,
    /// or `opt p=68020`, which implies it.
    #[serde(skip)]
    bit_fields: bool,
    /// The ranges of bytes the image places, start included and end not, in
    /// address order, none touching another.
    #[serde(skip)]
    placed: Vec<(usize, usize)>,
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
        let placed = placed_ranges(&instructions, &memory);
        Self {
            layout_items: Vec::new(),
            instructions,
            memory,
            symbols,
            entry,
            memory_map: Vec::new(),
            processor: Processor::M68000,
            bit_fields: false,
            placed,
        }
    }

    /// A Program that is a raw image and nothing else: `bytes` placed at
    /// `address`, run from `entry`, with no instruction, no Symbol and no
    /// source. It is the conformance harness's loader for Musashi's
    /// self-checking programs (ADR 0010), which are bytes linked at an
    /// address, and runs through [`Interpreter::from_image`]: an
    /// [`Interpreter::new`] would end it before it started, as it ends every
    /// Program with no instruction.
    ///
    /// The bytes are placed, as a `dc.b` of them would be, so `$FFFF $FFFF`
    /// inside the image pauses and past it ends the program (ADR 0009).
    ///
    /// [`Interpreter::from_image`]: crate::interpreter::Interpreter::from_image
    /// [`Interpreter::new`]: crate::interpreter::Interpreter::new
    #[cfg(any(test, feature = "conformance"))]
    pub fn from_image(address: usize, bytes: Vec<u8>, entry: usize) -> Program {
        let run = MemoryRun {
            address,
            content: MemoryContent::Bytes { bytes },
            location: Location::new("image", 0, 0, 0),
        };
        Program::new(Vec::new(), vec![run], &SymbolTable::new(), entry)
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

    /// The Program written for `processor`, with the 68020's bit fields when
    /// `bit_fields` is set or the processor is the 68020, which has them.
    pub fn with_dialect(mut self, processor: Processor, bit_fields: bool) -> Self {
        self.processor = processor;
        self.bit_fields = bit_fields || processor.has(Processor::M68020);
        self
    }

    /// The processor the Program is written for, and that its code is decoded
    /// for: the one `opt p=` chose, the 68000 when no line chose one.
    pub fn processor(&self) -> Processor {
        self.processor
    }

    /// Whether the Program has the 68020's bit-field instructions, as `opt
    /// bit` gives them to a 68000 Program in EASy68K and as the 68020 has them.
    pub fn bit_fields(&self) -> bool {
        self.bit_fields
    }

    /// What its code is decoded for: its processor, with the bit fields when
    /// it has them.
    pub fn dialect(&self) -> Dialect {
        Dialect {
            processor: self.processor,
            bit_fields: self.bit_fields,
        }
    }

    pub fn with_layout_items(mut self, items: Vec<u32>) -> Self {
        self.layout_items = items;
        self
    }
    /// Flat tuples: address, length, kind, section number, alignment.
    pub fn layout_items(&self) -> &[u32] {
        &self.layout_items
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

    /// The instruction laid out at `address`, if one is: the address map.
    ///
    /// An address that falls *between* the starts of two instructions has
    /// none. This is what the source lookups read, never what runs: the
    /// Interpreter runs what memory holds, so after the program changes a line's
    /// bytes this still answers the line as it was assembled.
    pub fn instruction_at(&self, address: usize) -> Option<&AssembledInstruction> {
        self.instruction_index_at(address)
            .map(|index| &self.instructions[index])
    }

    /// The index in [`instructions`](Program::instructions) of the instruction
    /// laid out at `address`, if one is.
    pub fn instruction_index_at(&self, address: usize) -> Option<usize> {
        self.instructions
            .binary_search_by_key(&address, |instruction| instruction.address)
            .ok()
    }

    /// Whether any of the `length` bytes from `address` is a byte the Program
    /// placed: an instruction's, or one of `dc`, `dcb` or `incbin`. The pad
    /// byte before an aligned item and the room `ds` reserves are not placed.
    pub fn places_any(&self, address: usize, length: usize) -> bool {
        let index = self.placed.partition_point(|&(_, end)| end <= address);
        self.placed
            .get(index)
            .is_some_and(|&(start, _)| start < address.saturating_add(length))
    }

    /// One past the last byte of the last instruction, and 0 for a Program
    /// with none.
    ///
    /// It is where the code ends and not where a run ends: the Interpreter runs
    /// on into what follows, and a program that does not end itself ends when
    /// it reaches `$FFFF $FFFF` the Program did not place, which is what memory
    /// past the last instruction holds when nothing else was placed there
    /// (ADR 0009).
    pub fn end_address(&self) -> usize {
        self.instructions
            .last()
            .map(|instruction| instruction.address + instruction.size)
            .unwrap_or(0)
    }

    /// The instruction whose last byte is just before `address`, if there is
    /// one: the instruction a return address on the stack comes back from, as
    /// the Program laid it out.
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

    /// What `instruction` assembled as, in EASy68K's syntax as the
    /// [disassembler](crate::machine::disassemble) prints it, when the
    /// Assembler chose something its line did not write; `None` when it did
    /// not.
    ///
    /// It is the "assembled as" of a hover, so it counts the choices of
    /// EASy68K's assembler (ADR 0006), and nothing the line said:
    ///
    /// * **another mnemonic**: `add.l #1,d0` assembles as `ADDQ.L #$1,D0`,
    ///   `move.l #5,d0` as `MOVEQ #5,D0`, `add #100,d0` as `ADDI.W`, `move.l
    ///   d0,a0` as `MOVEA.L D0,A0`. An alias is the same mnemonic: `dbra` is
    ///   `DBF`, `bhs` is `BCC` and `blo` is `BCS`, and none of them counts.
    /// * **a branch's size** the line did not write: `bra loop` assembles as
    ///   `BRA.S loop` or `BRA.W loop`. `.b` is `.s`, and `.l`, which the 68000
    ///   has not got, is a word.
    /// * **an absolute address in a word** the line did not write `.w` on:
    ///   `lea $1000,a0` assembles as `LEA $1000.W,A0`. A long one is the
    ///   default form, and is not a choice to show.
    ///
    /// A size the line left to its default is not a choice either: `move
    /// d0,d1` is `MOVE.W` by definition. `symbols` names the addresses the
    /// text holds, a Label in place of its number.
    pub fn assembled_as(
        &self,
        instruction: &AssembledInstruction,
        symbols: Option<&dyn Symbols>,
    ) -> Option<String> {
        let line = super::parse_line(&instruction.source);
        let operation = line.operation?;
        let text = disassemble(&instruction.instruction, symbols);
        let (head, operands) = text.split_once(' ').unwrap_or((&text, ""));
        let mnemonic = head.split('.').next().unwrap_or(head);
        if canonical_mnemonic(mnemonic) != canonical_mnemonic(&operation.name) {
            return Some(text);
        }
        let branch = match instruction.instruction {
            Instruction::Bcc(_, _, size) | Instruction::BRA(_, size) | Instruction::BSR(_, size) => {
                Some(size)
            }
            _ => None,
        };
        if let Some(size) = branch {
            let written = operation.size.map(|written| match written {
                SizeSuffix::Short | SizeSuffix::Byte => BranchSize::Short,
                SizeSuffix::Word => BranchSize::Word,
                SizeSuffix::Long => BranchSize::Long,
            });
            if written != Some(size) {
                return Some(text);
            }
        }
        let words = top_level_operands(operands)
            .filter(|operand| {
                operand.ends_with(".W") && !operand.contains('(') && !operand.starts_with('#')
            })
            .count();
        let written_words = operation
            .operands
            .iter()
            .filter(|operand| {
                matches!(
                    operand,
                    ast::Operand::Absolute {
                        size: Some(SizeSuffix::Word),
                        ..
                    }
                )
            })
            .count();
        (words > written_words).then_some(text)
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

/// The ranges of bytes `instructions` and the written runs of `memory` place,
/// merged where they touch, in address order.
fn placed_ranges(instructions: &[AssembledInstruction], memory: &[MemoryRun]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = instructions
        .iter()
        .map(|instruction| (instruction.address, instruction.address + instruction.size))
        .chain(
            memory
                .iter()
                .filter_map(|run| run.bytes().map(|bytes| (run.address, run.address + bytes.len()))),
        )
        .filter(|(start, end)| start < end)
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// A mnemonic as [`Program::assembled_as`] compares it: in lower case, with
/// `dbra` written `dbf` and the condition aliases `hs` and `lo` written `cc`
/// and `cs`, as the disassembler prints them.
fn canonical_mnemonic(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    if name == "dbra" {
        return "dbf".to_string();
    }
    for prefix in ["db", "b", "s"] {
        match name.strip_prefix(prefix) {
            Some("hs") => return format!("{prefix}cc"),
            Some("lo") => return format!("{prefix}cs"),
            _ => {}
        }
    }
    name
}

/// The Operands of a disassembled line, split at the commas that are not
/// inside parentheses: `4(A0,D1.W),D0` is two.
fn top_level_operands(field: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut operands = Vec::new();
    for (index, character) in field.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                operands.push(&field[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < field.len() {
        operands.push(&field[start..]);
    }
    operands.into_iter()
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
