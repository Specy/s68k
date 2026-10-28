//! The encoder against the Assembler: every Form of the instruction table,
//! assembled, lowered, encoded and decoded back.
//!
//! The asm-editor's `docs/design/m68k-machine.md`, decision 35: for every
//! implemented row of the table, every Form, every size it takes and none,
//! and one or two representative Operands for every Addressing mode each
//! position takes, the line is written out, assembled with the others into one
//! Program, and its Instruction is encoded at its address and decoded back.
//! The two must be equal, and the encoder must measure what it writes.
//!
//! Some lines are refused, by rules no single position states, and the test
//! says which ones and why rather than skipping what fails: a byte with an
//! address register in it, two Operands in memory, an address register added
//! into memory, a static bit number on an immediate. Immediates are taken
//! inside the instruction's value rule, so a count of 3 and not 9.
//!
//! Where an instruction has several encodings the test also says which one
//! EASy68K's order of forms gives (`expected_variant`), what size a branch
//! is and how wide an address, so that the round trip cannot pass on an
//! encoding EASy68K would not have chosen.
//!
//! `the_encodings_are_gnu_as_encodings` is the opt-in half: each line's
//! Instruction, written out with an explicit mnemonic in GNU `as`'s Motorola
//! syntax, is assembled by `as -m68000` and the bytes compared.

use std::collections::{BTreeMap, BTreeSet};

use crate::assembler::ast::SizeSuffix;
use crate::assembler::diagnostics::Diagnostic;
use crate::assembler::instructions::table::{
    Combination, Family, Form, InstructionSpec, Modes, SizeRule, TABLE,
};
use crate::assembler::program::Program;
use crate::assembler::source::{Files, DEFAULT_ENTRY_PATH};
use crate::assembler::{include, layout};
use crate::instructions::{
    AddressWidth, BranchSize, IndexRegister, Instruction, Operand, RegisterOperand, ShiftDirection,
    Sign, Size, TargetDirection,
};
use crate::machine::decode::{decode_words, first_processor};
use crate::machine::encode::{encode, encode_bytes, size};
use crate::machine::Processor;

/// One representative Operand: how it is written and the mode it is in.
/// `{b}` stands for a Label just above the line and `{f}` for one just below
/// it, so that an address is a back reference or a forward one and a branch
/// reaches a byte away.
#[derive(Debug, Clone)]
struct Representative {
    text: String,
    mode: Modes,
    /// An immediate's value.
    value: i64,
    /// An immediate written `#n.l`.
    long: bool,
}

impl Representative {
    fn of(text: &str, mode: Modes) -> Self {
        Self {
            text: text.to_string(),
            mode,
            value: 0,
            long: false,
        }
    }

    fn immediate(value: i64, long: bool) -> Self {
        Self {
            text: format!("#{value}{}", if long { ".l" } else { "" }),
            mode: Modes::IMMEDIATE,
            value,
            long,
        }
    }
}

/// The representatives of every mode but an immediate, whose value depends on
/// the instruction.
const REPRESENTATIVES: &[(Modes, &[&str])] = &[
    (Modes::DN, &["d1"]),
    (Modes::AN, &["a2"]),
    (Modes::INDIRECT, &["(a3)"]),
    (Modes::POSTINCREMENT, &["(a4)+"]),
    (Modes::PREDECREMENT, &["-(a5)"]),
    (Modes::DISPLACEMENT, &["-8(a6)"]),
    (Modes::INDEX, &["4(a0,d1.w)", "-4(a1,a2.l)"]),
    (Modes::ABSOLUTE, &["{b}", "{f}"]),
    (Modes::PC_DISPLACEMENT, &["{b}(pc)", "{f}(pc)"]),
    (Modes::PC_INDEX, &["{f}(pc,d1.w)", "(pc,a2.l)"]),
    (Modes::REGISTER_LIST, &["d0-d2/a0"]),
    (Modes::SR, &["sr"]),
    (Modes::CCR, &["ccr"]),
];

/// The representatives of every mode a position takes.
fn representatives(spec: &InstructionSpec, position: usize, modes: Modes) -> Vec<Representative> {
    let mut found = Vec::new();
    for (mode, texts) in REPRESENTATIVES {
        if modes.contains(*mode) {
            found.extend(texts.iter().map(|text| Representative::of(text, *mode)));
        }
    }
    if modes.contains(Modes::IMMEDIATE) {
        // Inside the value rule, which only the first Operand has: a quick
        // count, a shift count, a bit number, a `moveq` value. `trap #15` is
        // the one trap s68k assembles.
        let rule = spec.value_rule.filter(|_| position == 0);
        if spec.mnemonic == "trap" {
            found.push(Representative::immediate(15, false));
        } else {
            for value in [3, -2] {
                if rule.is_none_or(|rule| (rule.min..=rule.max.min(7)).contains(&value)) {
                    found.push(Representative::immediate(value, false));
                }
            }
            if rule.is_none() {
                found.push(Representative::immediate(3, true));
            }
        }
    }
    found
}

/// One generated line.
#[derive(Debug, Clone)]
struct Line {
    spec: &'static InstructionSpec,
    form: &'static Form,
    written: Option<SizeSuffix>,
    operands: Vec<Representative>,
}

impl Line {
    /// The line as source, its Labels `b<n>` and `f<n>`.
    fn text(&self, number: usize) -> String {
        let suffix = self
            .written
            .map_or(String::new(), |size| format!(".{}", size.letter()));
        let operands: Vec<String> = self
            .operands
            .iter()
            .map(|operand| {
                operand
                    .text
                    .replace("{b}", &format!("b{number}"))
                    .replace("{f}", &format!("f{number}"))
            })
            .collect();
        format!("    {}{suffix} {}", self.spec.mnemonic, operands.join(","))
    }
}

/// Every line of every implemented Form.
fn lines() -> Vec<Line> {
    let mut lines = Vec::new();
    for spec in TABLE.iter().filter(|spec| spec.is_implemented()) {
        for form in spec.forms {
            let mut sizes = vec![None];
            sizes.extend(form.sizes.allowed().iter().copied().map(Some));
            let mut combinations: Vec<Vec<Representative>> = vec![Vec::new()];
            for (position, modes) in form.operands.iter().enumerate() {
                let choices = representatives(spec, position, *modes);
                combinations = combinations
                    .into_iter()
                    .flat_map(|before| {
                        choices.iter().map(move |choice| {
                            let mut next = before.clone();
                            next.push(choice.clone());
                            next
                        })
                    })
                    .collect();
            }
            for written in &sizes {
                for operands in &combinations {
                    lines.push(Line {
                        spec,
                        form,
                        written: *written,
                        operands: operands.clone(),
                    });
                }
            }
        }
    }
    lines
}

/// The codes of the errors the analyzer raises on a line, by the rules no
/// position states.
fn refusals(line: &Line) -> BTreeSet<&'static str> {
    let mut codes = BTreeSet::new();
    let modes: Vec<Modes> = line.operands.iter().map(|operand| operand.mode).collect();
    let in_memory = |mode: &Modes| Modes::MEMORY.contains(*mode);
    if line.written == Some(SizeSuffix::Byte)
        && line.form.sizes == SizeRule::Any
        && modes.contains(&Modes::AN)
    {
        codes.insert("address_register_byte_size");
    }
    if line.form.combination == Combination::AtMostOneMemoryOperand
        && modes.len() == 2
        && modes.iter().all(in_memory)
    {
        codes.insert("both_operands_in_memory");
    }
    match (line.spec.family(), modes.as_slice()) {
        (Some(Family::AddSub { .. }), [Modes::AN, destination]) if in_memory(destination) => {
            codes.insert("invalid_addressing_mode");
        }
        (Some(Family::Bit(_)), [Modes::IMMEDIATE, Modes::IMMEDIATE]) => {
            codes.insert("invalid_addressing_mode");
        }
        _ => {}
    }
    codes
}

/// The variant EASy68K's order of forms gives a line of one of the
/// instructions that have several encodings, or `None` for the others.
fn expected_variant(line: &Line) -> Option<String> {
    let modes: Vec<Modes> = line.operands.iter().map(|operand| operand.mode).collect();
    let source = line.operands.first()?;
    let quick = source.mode == Modes::IMMEDIATE && !source.long && (1..=8).contains(&source.value);
    let mnemonic = line.spec.mnemonic;
    let [first, second] = modes.as_slice() else {
        return None;
    };
    let variant = match mnemonic {
        "move" => match (*first, *second) {
            (_, Modes::AN) => "MOVEA",
            (Modes::IMMEDIATE, Modes::DN)
                if line.written == Some(SizeSuffix::Long)
                    && !source.long
                    && (-128..=127).contains(&source.value) =>
            {
                "MOVEQ"
            }
            (Modes::SR, _) => "MOVEfromSR",
            (Modes::CCR, _) => "MOVEfromCCR",
            (_, Modes::SR) => "MOVEtoSR",
            (_, Modes::CCR) => "MOVEtoCCR",
            _ => "MOVE",
        },
        "add" | "sub" => {
            let stem = mnemonic.to_uppercase();
            return Some(match (*first, *second) {
                (_, Modes::AN) if quick => format!("{stem}Q"),
                (_, Modes::AN) => format!("{stem}A"),
                (Modes::IMMEDIATE, _) if quick => format!("{stem}Q"),
                (Modes::IMMEDIATE, _) => format!("{stem}I"),
                _ => stem,
            });
        }
        "adda" | "suba" | "addi" | "subi" => {
            let stem = mnemonic[..3].to_uppercase();
            return Some(match quick {
                true => format!("{stem}Q"),
                false => mnemonic.to_uppercase(),
            });
        }
        "cmp" => match (*first, *second) {
            (_, Modes::DN) => "CMP",
            (_, Modes::AN) => "CMPA",
            (Modes::IMMEDIATE, _) => "CMPI",
            _ => "CMPM",
        },
        "and" | "or" => {
            let stem = mnemonic.to_uppercase();
            return Some(match (*first, *second) {
                (_, Modes::DN) => stem,
                (Modes::IMMEDIATE, _) => format!("{stem}I"),
                _ => stem,
            });
        }
        "eor" => match *first {
            Modes::IMMEDIATE => "EORI",
            _ => "EOR",
        },
        _ => return None,
    };
    Some(variant.to_string())
}

/// The name of an Instruction's variant, as `Debug` writes it.
fn variant(instruction: &Instruction) -> String {
    let debug = format!("{instruction:?}");
    debug
        .split(['(', ' ', '{'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The size a branch's displacement has, if it is one.
fn branch_size(instruction: &Instruction) -> Option<BranchSize> {
    match instruction {
        Instruction::BRA(_, size) | Instruction::BSR(_, size) | Instruction::Bcc(_, _, size) => {
            Some(*size)
        }
        _ => None,
    }
}

/// Every line, laid out in one Program between its two Labels, with the
/// Diagnostics of each line by its index in `lines`.
struct Generated {
    lines: Vec<Line>,
    program: Program,
    /// The error codes raised on each line.
    errors: BTreeMap<usize, BTreeSet<&'static str>>,
    /// The Source line each generated line is on.
    source_lines: Vec<usize>,
}

fn generate() -> Generated {
    let lines = lines();
    let mut source = String::from("    org $1000\n");
    let mut source_lines = Vec::with_capacity(lines.len());
    for (number, line) in lines.iter().enumerate() {
        source.push_str(&format!("b{number}:\n"));
        source_lines.push(source.lines().count());
        source.push_str(&line.text(number));
        source.push_str(&format!("\nf{number}:\n"));
    }
    let files = Files::from_source(source);
    let (unit, found) = include::expand(&files, DEFAULT_ENTRY_PATH).expect("the source expands");
    let (program, diagnostics) = layout::lay_out(&unit, found);
    let by_line: BTreeMap<usize, usize> = source_lines
        .iter()
        .enumerate()
        .map(|(number, line)| (*line, number))
        .collect();
    let mut errors: BTreeMap<usize, BTreeSet<&'static str>> = BTreeMap::new();
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
    {
        let number = by_line
            .get(&diagnostic.location.line)
            .unwrap_or_else(|| panic!("an error on no generated line: {}", describe(diagnostic)));
        errors.entry(*number).or_default().insert(diagnostic.code());
    }
    Generated {
        lines,
        program,
        errors,
        source_lines,
    }
}

fn describe(diagnostic: &Diagnostic) -> String {
    format!(
        "line {}: {} [{}]",
        diagnostic.location.line,
        diagnostic.message(),
        diagnostic.code()
    )
}

/// The processor whose decoder reads `word`: the 68000's, unless only a
/// later one has it.
fn processor_for(word: u16) -> Processor {
    first_processor(word).unwrap_or(Processor::M68000)
}

#[test]
fn every_form_of_the_table_encodes_and_decodes_back() {
    let generated = generate();
    let instructions: BTreeMap<usize, &crate::assembler::program::AssembledInstruction> = generated
        .program
        .instructions()
        .iter()
        .map(|instruction| (instruction.location.line, instruction))
        .collect();
    let mut checked = 0;
    let mut refused = 0;
    let mut families = BTreeSet::new();
    for (number, line) in generated.lines.iter().enumerate() {
        let text = line.text(number);
        let expected = refusals(line);
        let found = generated.errors.get(&number).cloned().unwrap_or_default();
        assert_eq!(found, expected, "`{}`", text.trim());
        if !expected.is_empty() {
            refused += 1;
            continue;
        }
        let assembled = instructions
            .get(&generated.source_lines[number])
            .unwrap_or_else(|| panic!("`{}` assembled to nothing", text.trim()));
        let instruction = assembled.instruction;
        let address = assembled.address as u32;
        let words = encode(&instruction, address)
            .unwrap_or_else(|error| panic!("`{}` is {instruction:?}: {error}", text.trim()));
        assert_eq!(
            size(&instruction),
            Ok(words.len() as u32 * 2),
            "`{}`",
            text.trim()
        );
        let decoded = decode_words(&words, address, processor_for(words[0])).unwrap();
        assert_eq!(
            decoded.instruction,
            instruction,
            "`{}` as {words:04X?}",
            text.trim()
        );
        assert_eq!(decoded.size, words.len() as u32 * 2, "`{}`", text.trim());

        // The encoding EASy68K chooses.
        if let Some(expected) = expected_variant(line) {
            assert_eq!(variant(&instruction), expected, "`{}`", text.trim());
        }
        if let Some(size) = branch_size(&instruction) {
            let expected = match line.written {
                Some(SizeSuffix::Short | SizeSuffix::Byte) => BranchSize::Short,
                Some(_) => BranchSize::Word,
                // A back reference a word above is short, a forward one a word.
                None if line.operands[0].text == "{b}" => BranchSize::Short,
                None => BranchSize::Word,
            };
            assert_eq!(size, expected, "`{}`", text.trim());
        } else if !matches!(instruction, Instruction::DBcc(..)) {
            let debug = format!("{instruction:?}");
            let count = |text: &str| line.operands.iter().filter(|o| o.text == text).count();
            assert_eq!(
                debug.matches("width: Word").count(),
                count("{b}"),
                "`{}` is {debug}",
                text.trim()
            );
            assert_eq!(
                debug.matches("width: Long").count(),
                count("{f}"),
                "`{}` is {debug}",
                text.trim()
            );
        }
        families.insert(variant(&instruction));
        checked += 1;
    }
    // Every encoding the Assembler can choose comes up at least once.
    for variant in [
        "MOVE",
        "MOVEA",
        "MOVEQ",
        "MOVEM",
        "MOVEP",
        "ADD",
        "ADDA",
        "ADDI",
        "ADDQ",
        "ADDX",
        "SUB",
        "SUBA",
        "SUBI",
        "SUBQ",
        "SUBX",
        "CMP",
        "CMPA",
        "CMPI",
        "CMPM",
        "AND",
        "ANDI",
        "OR",
        "ORI",
        "EOR",
        "EORI",
        "MOVEtoCCR",
        "MOVEfromCCR",
        "MOVEtoSR",
        "MOVEfromSR",
        "ANDItoCCR",
        "ANDItoSR",
        "ORItoCCR",
        "ORItoSR",
        "EORItoCCR",
        "EORItoSR",
        "EXG",
        "EXT",
        "BRA",
        "BSR",
        "Bcc",
        "DBcc",
        "Scc",
        "BTST",
        "BSET",
        "BCLR",
        "BCHG",
        "ASd",
        "LSd",
        "ROd",
        "ROXd",
        "LINK",
        "UNLK",
        "TRAP",
        "CHK",
        "TAS",
        "NBCD",
        "ABCD",
        "SBCD",
    ] {
        assert!(families.contains(variant), "no line assembled as {variant}");
    }
    assert!(checked > 3000, "only {checked} lines were checked");
    assert!(refused > 50, "only {refused} lines were refused");
}

// ---------------------------------------------------------------------------
// GNU `as`
// ---------------------------------------------------------------------------

/// A register as GNU `as` reads it with `--register-prefix-optional`.
fn gnu_register(register: RegisterOperand) -> String {
    register.name()
}

fn gnu_index(index: IndexRegister) -> String {
    let size = match index.size {
        Size::Long => "l",
        _ => "w",
    };
    format!("{}.{size}", gnu_register(index.register))
}

fn gnu_size(size: Size) -> &'static str {
    match size {
        Size::Byte => ".b",
        Size::Word => ".w",
        Size::Long => ".l",
    }
}

/// A distance from the instruction, as GNU `as` writes one from `.`.
fn from_here(target: u32, address: u32) -> String {
    format!(".{:+}", target.wrapping_sub(address) as i32)
}

/// An Operand in GNU `as`'s Motorola syntax. A PC-relative one is written as
/// the address it reaches, from `.`, so that `as` works out the displacement
/// from where it puts the extension word; `pc_index_zero` writes a
/// displacement of 0 instead, which is the one `as` refuses as an address.
fn gnu_operand(operand: Operand, address: u32, pc_index_zero: bool) -> String {
    match operand {
        Operand::Immediate(value) => format!("#0x{value:x}"),
        Operand::Register(register) => gnu_register(register),
        Operand::Indirect(n) => format!("(a{n})"),
        Operand::PostIndirect(n) => format!("(a{n})+"),
        Operand::PreIndirect(n) => format!("-(a{n})"),
        Operand::IndirectDisplacement { offset, base } => {
            format!("({offset},{})", gnu_register(base))
        }
        Operand::IndirectIndex {
            base,
            offset,
            index,
        } => format!("({offset},{},{})", gnu_register(base), gnu_index(index)),
        Operand::Absolute {
            address,
            width: AddressWidth::Word,
        } => format!("({}).w", address as u32 as i32),
        Operand::Absolute {
            address,
            width: AddressWidth::Long,
        } => format!("(0x{:x}).l", address as u32),
        Operand::PcDisplacement { target } => format!("({},pc)", from_here(target, address)),
        Operand::PcIndex { target, index } => match pc_index_zero {
            true => format!("(0,pc,{})", gnu_index(index)),
            false => format!("({},pc,{})", from_here(target, address), gnu_index(index)),
        },
    }
}

/// The register list of a `movem` mask, one register at a time.
fn gnu_list(mask: u16, target: Operand) -> String {
    let mask = match target {
        Operand::PreIndirect(_) => mask.reverse_bits(),
        _ => mask,
    };
    (0..16)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| match bit {
            0..=7 => format!("d{bit}"),
            _ => format!("a{}", bit - 8),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The line GNU `as -m68000` assembles to the encoding of `instruction` at
/// `address`, with an explicit mnemonic, or `None` where it has none: `as`
/// writes `cmp #n,dn` and `and #n,dn` as `cmpi` and `andi`, and a `move.l`
/// of a byte as `moveq`, whatever is written; SIMHALT is EASy68K's; and the
/// 68010's and the 68020's instructions are not the 68000's.
fn gnu_line(instruction: &Instruction, address: u32, pc_index_zero: bool) -> Option<String> {
    use Instruction as I;
    let operand = |operand: Operand| gnu_operand(operand, address, pc_index_zero);
    let two = |mnemonic: &str, size: Size, source: Operand, destination: Operand| {
        format!(
            "{mnemonic}{} {},{}",
            gnu_size(size),
            operand(source),
            operand(destination)
        )
    };
    let immediate = |mnemonic: &str, value: u32, destination: Operand, size: Size| {
        two(mnemonic, size, Operand::Immediate(value), destination)
    };
    Some(match *instruction {
        I::CMP(Operand::Immediate(_), ..)
        | I::AND(Operand::Immediate(_), ..)
        | I::OR(Operand::Immediate(_), ..)
        | I::ADD(Operand::Immediate(_), ..)
        | I::SUB(Operand::Immediate(_), ..) => return None,
        I::MOVE(
            Operand::Immediate(value),
            Operand::Register(RegisterOperand::Data(_)),
            Size::Long,
        ) if (-128..=127).contains(&(value as i32)) => return None,
        I::SIMHALT
        | I::IllegalWord(_)
        | I::LINEA(_)
        | I::LINEF(_)
        | I::MOVEfromCCR(_)
        | I::EXT(_, Size::Byte, Size::Long) => return None,
        I::Bcc(_, _, BranchSize::Long)
        | I::BRA(_, BranchSize::Long)
        | I::BSR(_, BranchSize::Long) => return None,
        I::MOVE(source, destination, size) => two("move", size, source, destination),
        I::MOVEA(source, register, size) => two("movea", size, source, Operand::Register(register)),
        I::MOVEQ(value, register) => format!("moveq #{},{}", value as i8, gnu_register(register)),
        I::MOVEM {
            direction,
            size,
            registers_mask,
            target,
        } => match direction {
            TargetDirection::ToMemory => format!(
                "movem{} {},{}",
                gnu_size(size),
                gnu_list(registers_mask, target),
                operand(target)
            ),
            TargetDirection::FromMemory => format!(
                "movem{} {},{}",
                gnu_size(size),
                operand(target),
                gnu_list(registers_mask, target)
            ),
        },
        I::MOVEP {
            direction,
            size,
            register,
            target,
        } => match direction {
            TargetDirection::ToMemory => two("movep", size, Operand::Register(register), target),
            TargetDirection::FromMemory => two("movep", size, target, Operand::Register(register)),
        },
        I::ADD(source, destination, size) => two("add", size, source, destination),
        I::SUB(source, destination, size) => two("sub", size, source, destination),
        I::ADDA(source, register, size) => two("adda", size, source, Operand::Register(register)),
        I::SUBA(source, register, size) => two("suba", size, source, Operand::Register(register)),
        I::CMPA(source, register, size) => two("cmpa", size, source, Operand::Register(register)),
        I::ADDQ(count, destination, size) => {
            two("addq", size, Operand::Immediate(count as u32), destination)
        }
        I::SUBQ(count, destination, size) => {
            two("subq", size, Operand::Immediate(count as u32), destination)
        }
        I::ADDI(value, destination, size) => immediate("addi", value, destination, size),
        I::SUBI(value, destination, size) => immediate("subi", value, destination, size),
        I::ANDI(value, destination, size) => immediate("andi", value, destination, size),
        I::ORI(value, destination, size) => immediate("ori", value, destination, size),
        I::EORI(value, destination, size) => immediate("eori", value, destination, size),
        I::CMPI(value, destination, size) => immediate("cmpi", value, destination, size),
        I::ADDX(source, destination, size) => two("addx", size, source, destination),
        I::SUBX(source, destination, size) => two("subx", size, source, destination),
        I::CMPM(source, destination, size) => two("cmpm", size, source, destination),
        I::CMP(source, register, size) => two("cmp", size, source, Operand::Register(register)),
        I::AND(source, destination, size) => two("and", size, source, destination),
        I::OR(source, destination, size) => two("or", size, source, destination),
        I::EOR(source, destination, size) => two("eor", size, source, destination),
        I::DIVx(source, register, sign) => {
            let mnemonic = match sign {
                Sign::Signed => "divs",
                Sign::Unsigned => "divu",
            };
            two(mnemonic, Size::Word, source, Operand::Register(register))
        }
        I::MULx(source, register, sign) => {
            let mnemonic = match sign {
                Sign::Signed => "muls",
                Sign::Unsigned => "mulu",
            };
            two(mnemonic, Size::Word, source, Operand::Register(register))
        }
        I::CHK(bound, register) => two("chk", Size::Word, bound, Operand::Register(register)),
        I::SWAP(register) => format!("swap {}", gnu_register(register)),
        I::EXT(register, _, written) => {
            format!("ext{} {}", gnu_size(written), gnu_register(register))
        }
        I::EXG(first, second) => format!("exg {},{}", gnu_register(first), gnu_register(second)),
        I::LEA(source, register) => {
            format!("lea {},{}", operand(source), gnu_register(register))
        }
        I::PEA(source) => format!("pea {}", operand(source)),
        I::JMP(target) => format!("jmp {}", operand(target)),
        I::JSR(target) => format!("jsr {}", operand(target)),
        I::CLR(target, size) => format!("clr{} {}", gnu_size(size), operand(target)),
        I::NEG(target, size) => format!("neg{} {}", gnu_size(size), operand(target)),
        I::NEGX(target, size) => format!("negx{} {}", gnu_size(size), operand(target)),
        I::NOT(target, size) => format!("not{} {}", gnu_size(size), operand(target)),
        I::TST(target, size) => format!("tst{} {}", gnu_size(size), operand(target)),
        I::NBCD(target) => format!("nbcd {}", operand(target)),
        I::TAS(target) => format!("tas {}", operand(target)),
        I::ABCD(source, destination) => {
            format!("abcd {},{}", operand(source), operand(destination))
        }
        I::SBCD(source, destination) => {
            format!("sbcd {},{}", operand(source), operand(destination))
        }
        I::Scc(target, condition) => format!("s{} {}", condition.suffix(), operand(target)),
        I::DBcc(register, target, condition) => format!(
            "db{} {},{}",
            condition.suffix(),
            gnu_register(register),
            from_here(target, address)
        ),
        I::BRA(target, size) | I::BSR(target, size) | I::Bcc(target, _, size) => {
            let mnemonic = match instruction {
                I::BRA(..) => "bra".to_string(),
                I::BSR(..) => "bsr".to_string(),
                I::Bcc(_, condition, _) => format!("b{}", condition.suffix()),
                _ => unreachable!(),
            };
            let size = match size {
                BranchSize::Short => ".s",
                _ => ".w",
            };
            format!("{mnemonic}{size} {}", from_here(target, address))
        }
        I::LINK(register, displacement) => {
            format!("link {},#{}", gnu_register(register), displacement as i32)
        }
        I::UNLK(register) => format!("unlk {}", gnu_register(register)),
        I::TRAP(vector) => format!("trap #{vector}"),
        I::ASd(count, target, direction, size)
        | I::LSd(count, target, direction, size)
        | I::ROd(count, target, direction, size)
        | I::ROXd(count, target, direction, size) => {
            let stem = match instruction {
                I::ASd(..) => "as",
                I::LSd(..) => "ls",
                I::ROd(..) => "ro",
                _ => "rox",
            };
            let way = match direction {
                ShiftDirection::Left => "l",
                ShiftDirection::Right => "r",
            };
            match target {
                Operand::Register(_) => two(&format!("{stem}{way}"), size, count, target),
                _ => format!("{stem}{way}.w {}", operand(target)),
            }
        }
        I::BTST(bit, target) => format!("btst {},{}", operand(bit), operand(target)),
        I::BSET(bit, target) => format!("bset {},{}", operand(bit), operand(target)),
        I::BCLR(bit, target) => format!("bclr {},{}", operand(bit), operand(target)),
        I::BCHG(bit, target) => format!("bchg {},{}", operand(bit), operand(target)),
        I::MOVEtoCCR(source) => format!("move.w {},ccr", operand(source)),
        I::MOVEtoSR(source) => format!("move.w {},sr", operand(source)),
        I::MOVEfromSR(target) => format!("move.w sr,{}", operand(target)),
        I::ANDItoCCR(value) => format!("andi.b #0x{value:x},ccr"),
        I::ORItoCCR(value) => format!("ori.b #0x{value:x},ccr"),
        I::EORItoCCR(value) => format!("eori.b #0x{value:x},ccr"),
        I::ANDItoSR(value) => format!("andi.w #0x{value:x},sr"),
        I::ORItoSR(value) => format!("ori.w #0x{value:x},sr"),
        I::EORItoSR(value) => format!("eori.w #0x{value:x},sr"),
        I::STOP(value) => format!("stop #0x{value:x}"),
        I::MOVEUSP { to_usp, register } => match to_usp {
            true => format!("move.l {},usp", gnu_register(register)),
            false => format!("move.l usp,{}", gnu_register(register)),
        },
        I::RTE => "rte".to_string(),
        I::RESET => "reset".to_string(),
        I::RTS => "rts".to_string(),
        I::RTR => "rtr".to_string(),
        I::NOP => "nop".to_string(),
        I::TRAPV => "trapv".to_string(),
        I::ILLEGAL => "illegal".to_string(),
    })
}

/// What GNU `as -m68000` makes of one line: its bytes, or its complaint.
struct Gnu {
    prefix: String,
    directory: std::path::PathBuf,
}

impl Gnu {
    fn assemble(&self, line: &str) -> Result<Vec<u8>, String> {
        use std::process::Command;
        let (source, object, binary) = (
            self.directory.join("line.s"),
            self.directory.join("line.o"),
            self.directory.join("line.bin"),
        );
        std::fs::write(&source, format!("{line}\n")).unwrap();
        let assembled = Command::new(format!("{}as", self.prefix))
            .args(["-m68000", "--register-prefix-optional", "-o"])
            .arg(&object)
            .arg(&source)
            .output()
            .expect("GNU as runs");
        if !assembled.status.success() {
            return Err(String::from_utf8_lossy(&assembled.stderr).to_string());
        }
        let copied = Command::new(format!("{}objcopy", self.prefix))
            .args(["-O", "binary", "-j", ".text"])
            .arg(&object)
            .arg(&binary)
            .status()
            .expect("GNU objcopy runs");
        assert!(copied.success());
        Ok(std::fs::read(&binary).unwrap())
    }
}

/// The opt-in comparison with GNU `as`: every line the generator assembles,
/// lowered and encoded by s68k, against the same encoding written with an
/// explicit mnemonic and assembled by `as -m68000`, one line at a time.
///
/// `S68K_BINUTILS_PREFIX` names the m68k binutils as the decoder's own
/// opt-in test says (`machine::decode`, `the_hand_assembled_words_are_gnu_as_words`).
///
/// `cargo test --release gnu_as -- --ignored`
#[test]
#[ignore = "needs GNU as for m68k, named by S68K_BINUTILS_PREFIX"]
fn the_encodings_are_gnu_as_encodings() {
    let prefix = std::env::var("S68K_BINUTILS_PREFIX")
        .expect("S68K_BINUTILS_PREFIX names the m68k binutils, e.g. /usr/bin/m68k-linux-gnu-");
    let directory = std::env::temp_dir().join(format!("s68k-gnu-as-encode-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let gnu = Gnu { prefix, directory };
    let generated = generate();
    // One line per distinct GNU line and placement-free encoding: the same
    // instruction at another address is the same comparison.
    let mut seen = BTreeSet::new();
    let mut checked = 0;
    let mut mismatches = Vec::new();
    for assembled in generated.program.instructions() {
        let (instruction, address) = (assembled.instruction, assembled.address as u32);
        let Some(line) = gnu_line(&instruction, address, false) else {
            continue;
        };
        let ours = encode_bytes(&instruction, address).expect("the generator's lines encode");
        if !seen.insert((line.clone(), ours.clone())) {
            continue;
        }
        let theirs = match gnu.assemble(&line) {
            Ok(bytes) => bytes,
            // `as` refuses a PC-relative index that reaches its own extension
            // word, as it refuses a short branch of 0; the displacement is
            // then written as the number.
            Err(complaint) if complaint.contains("invalid byte branch offset") => {
                let line = gnu_line(&instruction, address, true).unwrap();
                gnu.assemble(&line)
                    .unwrap_or_else(|complaint| panic!("`{line}`: {complaint}"))
            }
            Err(complaint) => panic!("`{line}`: {complaint}"),
        };
        if theirs != ours {
            mismatches.push(format!(
                "`{line}` ({}): as {:02X?}, s68k {:02X?}",
                assembled.source.trim(),
                theirs,
                ours
            ));
        }
        checked += 1;
    }
    std::fs::remove_dir_all(&gnu.directory).ok();
    assert!(
        mismatches.is_empty(),
        "{} of {checked} lines differ:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert!(checked > 1000, "only {checked} lines were checked");
}

/// EASy68K's back-reference rule over a whole program: a value known above
/// the line, or on it, gets the short form; a forward one the long form,
/// whatever it turns out to be; and `#n.l` keeps the long form of a value
/// that is known.
#[test]
fn a_back_reference_decides_the_short_forms() {
    let source = "\
one equ 1
start:
    add.l #one,d0
    add.l #two,d0
    add.l #1.l,d0
    sub.w #8,a0
    adda.l #9,a0
    move.l #one,d1
    move.l #two,d1
    move #1,d1
    move.l #1.l,d1
    move.l #-128,d1
    move.l #128,d1
here: bra here
    bra start
    bra later
    bra.s later
    bra.l start
    bsr.b start
    jmp start
    jmp later
    jmp $7ffe
    jmp $8000
    jmp $ffff8000
    jmp start.l
    jmp later.w
    movem.w (pc,d1.w),d0
    btst #1,(pc,d1.w)
    btst d1,(pc,d1.w)
    exg a1,d0
later: nop
two equ 2
";
    let assembly = crate::assembler::assemble_source(source);
    let program = assembly.program.expect("the program assembles");
    let instructions: Vec<Instruction> = program
        .instructions()
        .iter()
        .map(|instruction| instruction.instruction)
        .collect();
    let d = |number| Operand::Register(RegisterOperand::Data(number));
    let a = |number| Operand::Register(RegisterOperand::Address(number));
    let address = |address: usize, width| Operand::Absolute { address, width };
    let index = IndexRegister {
        register: RegisterOperand::Data(1),
        size: Size::Word,
    };
    // Every instruction is four bytes, from `$1000`: `start` is `$1000`,
    // `here` `$102c` and `later` `$1070`.
    assert_eq!(
        instructions,
        [
            Instruction::ADDQ(1, d(0), Size::Long),
            Instruction::ADDI(2, d(0), Size::Long),
            Instruction::ADDI(1, d(0), Size::Long),
            Instruction::SUBQ(8, a(0), Size::Word),
            Instruction::ADDA(
                Operand::Immediate(9),
                RegisterOperand::Address(0),
                Size::Long
            ),
            Instruction::MOVEQ(1, RegisterOperand::Data(1)),
            Instruction::MOVE(Operand::Immediate(2), d(1), Size::Long),
            Instruction::MOVE(Operand::Immediate(1), d(1), Size::Word),
            Instruction::MOVE(Operand::Immediate(1), d(1), Size::Long),
            Instruction::MOVEQ(0x80, RegisterOperand::Data(1)),
            Instruction::MOVE(Operand::Immediate(128), d(1), Size::Long),
            // A Label on the line itself is above it.
            Instruction::BRA(0x102c, BranchSize::Short),
            Instruction::BRA(0x1000, BranchSize::Short),
            Instruction::BRA(0x1070, BranchSize::Word),
            Instruction::BRA(0x1070, BranchSize::Short),
            Instruction::BRA(0x1000, BranchSize::Word),
            Instruction::BSR(0x1000, BranchSize::Short),
            Instruction::JMP(address(0x1000, AddressWidth::Word)),
            Instruction::JMP(address(0x1070, AddressWidth::Long)),
            Instruction::JMP(address(0x7ffe, AddressWidth::Word)),
            Instruction::JMP(address(0x8000, AddressWidth::Long)),
            // Read in 32 bits, as EASy68K reads it, `$FFFF8000` is -32768.
            Instruction::JMP(address(0xffff_8000, AddressWidth::Word)),
            Instruction::JMP(address(0x1000, AddressWidth::Long)),
            Instruction::JMP(address(0x1070, AddressWidth::Word)),
            // `(pc,d1.w)` reaches its own extension word, which a mask or a
            // static bit number puts four bytes past the instruction.
            Instruction::MOVEM {
                direction: TargetDirection::FromMemory,
                size: Size::Word,
                registers_mask: 1,
                target: Operand::PcIndex {
                    target: 0x1064,
                    index
                },
            },
            Instruction::BTST(
                Operand::Immediate(1),
                Operand::PcIndex {
                    target: 0x1068,
                    index
                }
            ),
            Instruction::BTST(
                d(1),
                Operand::PcIndex {
                    target: 0x106a,
                    index
                }
            ),
            Instruction::EXG(RegisterOperand::Data(0), RegisterOperand::Address(1)),
            Instruction::NOP,
        ]
    );
    // Each of them encodes where it is and decodes back.
    for assembled in program.instructions() {
        let address = assembled.address as u32;
        let words = encode(&assembled.instruction, address).unwrap();
        let decoded = decode_words(&words, address, Processor::M68000).unwrap();
        assert_eq!(decoded.instruction, assembled.instruction);
    }
}
