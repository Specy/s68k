//! The 68020 dialect's addressing modes and long branches (ADR 0011; the
//! asm-editor's `docs/design/m68k-machine.md`, decision 3, and the plan's
//! milestone C13): the scaled index of the brief extension word, the full
//! extension word with its base and outer displacements, suppressed bases and
//! indexes and memory indirection, from an address register and from the
//! program counter.
//!
//! The words are checked both ways, every combination of base, index,
//! displacements and indirection encoded and decoded back; the syntax is
//! checked by assembling every form and reading the words it gives, and by
//! disassembling them and assembling the text again; and the addresses the
//! modes name are checked against the manual's sums, worked out by hand.

use crate::assembler::program::Program;
use crate::instructions::{
    DisplacementSize, ExtensionWord, IndexBase, IndexRegister, Indirection, Instruction, Operand,
    RegisterOperand, Size,
};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus};
use crate::machine::decode::{decode_words, MAX_SIZE};
use crate::machine::disassemble::disassemble;
use crate::machine::encode::{encode, size};
use crate::machine::Processor;

fn assemble(code: &str) -> Program {
    let assembly = crate::assembler::assemble_source(code);
    match assembly.program {
        Some(program) => program,
        None => panic!(
            "the program does not assemble: {:?}",
            assembly
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .map(|diagnostic| format!(
                    "line {}: {}",
                    diagnostic.location.line + 1,
                    diagnostic.message()
                ))
                .collect::<Vec<_>>()
        ),
    }
}

fn machine(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 10_000,
        }),
    )
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn a(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Address(register), Size::Long)
}

/// The words of the first instruction of `body`, assembled for the 68020 at
/// `$1000`.
fn words_of(body: &str) -> Vec<u16> {
    let program = assemble(&format!("    opt p=68020\n    org $1000\n{body}"));
    let instruction = &program.instructions()[0];
    instruction
        .bytes
        .chunks(2)
        .map(|word| u16::from_be_bytes([word[0], word[1]]))
        .collect()
}

const SIZES: [DisplacementSize; 3] = [
    DisplacementSize::Null,
    DisplacementSize::Word,
    DisplacementSize::Long,
];

/// A displacement of `size` that only that size holds, but for null.
fn value_of(size: DisplacementSize, negative: bool) -> i32 {
    let value = match size {
        DisplacementSize::Null => 0,
        DisplacementSize::Word => 0x1234,
        DisplacementSize::Long => 0x12_3456,
    };
    match negative {
        true => -value,
        false => value,
    }
}

/// Every full-format Operand of the bases and indexes below, each with every
/// width of base and outer displacement and every indirection, at an
/// instruction placed at `address`: what the round trips are over.
pub(super) fn every_full_operand(address: u32) -> Vec<Operand> {
    let bases = [
        (IndexBase::address(0), false),
        (IndexBase::address(5), false),
        (IndexBase::PC, false),
        (IndexBase::address(0), true),
        (IndexBase::address(3), true),
        (IndexBase::PC, true),
    ];
    let indexes = [
        (RegisterOperand::Data(1), Size::Word, 1),
        (RegisterOperand::Data(7), Size::Long, 4),
        (RegisterOperand::Address(2), Size::Word, 8),
        (RegisterOperand::Address(6), Size::Long, 2),
    ];
    let mut operands = Vec::new();
    for (base, base_suppressed) in bases {
        for (index_suppressed, &(register, size, scale)) in [false, true]
            .into_iter()
            .flat_map(|suppressed| indexes.iter().map(move |index| (suppressed, index)))
        {
            //a suppressed index's bits are clear, as an assembler writes them
            let (index, scale) = match index_suppressed {
                false => (IndexRegister { register, size }, scale),
                true => (
                    IndexRegister {
                        register: RegisterOperand::Data(0),
                        size: Size::Word,
                    },
                    1,
                ),
            };
            for indirection in [
                Indirection::None,
                Indirection::Preindexed,
                Indirection::Postindexed,
            ] {
                if index_suppressed && indirection == Indirection::Postindexed {
                    //with the index suppressed I/IS has no postindexed form
                    continue;
                }
                for base_size in SIZES {
                    let outer_sizes: &[DisplacementSize] = match indirection {
                        Indirection::None => &[DisplacementSize::Null],
                        _ => &SIZES,
                    };
                    for &outer_size in outer_sizes {
                        let word = ExtensionWord::full(
                            index,
                            scale,
                            base_suppressed,
                            index_suppressed,
                            base_size,
                            indirection,
                            outer_size,
                        );
                        let negative = operands.len() % 2 == 1;
                        let base_displacement = value_of(base_size, negative);
                        //from the program counter, the address the displacement reaches from
                        //the extension word, the word after the operation word here
                        let displacement = match base.is_pc() && !base_suppressed {
                            true => (address + 2).wrapping_add(base_displacement as u32) as i32,
                            false => base_displacement,
                        };
                        operands.push(Operand::Extended {
                            base,
                            word,
                            displacement,
                            outer: value_of(outer_size, !negative),
                        });
                    }
                }
            }
        }
    }
    operands
}

/// Every brief-format Operand with a scale: each base, each index and each
/// scale but 1, which is the 68000's word.
pub(super) fn every_brief_operand(address: u32) -> Vec<Operand> {
    let mut operands = Vec::new();
    for base in [IndexBase::address(0), IndexBase::address(7), IndexBase::PC] {
        for register in [RegisterOperand::Data(3), RegisterOperand::Address(4)] {
            for size in [Size::Word, Size::Long] {
                for scale in [2, 4, 8] {
                    let byte = match operands.len() % 3 {
                        0 => -128,
                        1 => 0,
                        _ => 127,
                    };
                    operands.push(Operand::Extended {
                        base,
                        word: ExtensionWord::brief(IndexRegister { register, size }, scale),
                        displacement: match base.is_pc() {
                            true => (address + 2).wrapping_add(byte as u32) as i32,
                            false => byte,
                        },
                        outer: 0,
                    });
                }
            }
        }
    }
    operands
}

mod words {
    use super::*;

    /// Every combination encodes to words that decode back, for the 68020,
    /// to the same Operand, and the encoder measures what it writes: the
    /// operation word, the extension word and the displacements.
    #[test]
    fn every_extended_operand_encodes_and_decodes_back() {
        let address = 0x2000;
        let mut checked = 0;
        for operand in every_full_operand(address)
            .into_iter()
            .chain(every_brief_operand(address))
        {
            let Operand::Extended { word, .. } = operand else {
                unreachable!()
            };
            for instruction in [
                Instruction::LEA(operand, RegisterOperand::Address(1)),
                Instruction::MOVE(
                    operand,
                    Operand::Register(RegisterOperand::Data(0)),
                    Size::Long,
                ),
            ] {
                let words = encode(&instruction, address)
                    .unwrap_or_else(|error| panic!("{instruction:?}: {error}"));
                assert_eq!(words.len() as u32 * 2, 2 + word.bytes(), "{instruction:?}");
                assert_eq!(size(&instruction), Ok(2 + word.bytes()));
                let decoded = decode_words(&words, address, Processor::M68020).unwrap();
                assert_eq!(decoded.instruction, instruction, "{words:04X?}");
                assert_eq!(decoded.size, 2 + word.bytes());
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    /// Two full-format Operands make the longest instruction any processor
    /// has: a `move` of 22 bytes.
    #[test]
    fn the_longest_instruction_is_a_move_of_two_memory_indirect_operands() {
        let word = ExtensionWord::full(
            IndexRegister {
                register: RegisterOperand::Data(0),
                size: Size::Long,
            },
            4,
            false,
            false,
            DisplacementSize::Long,
            Indirection::Postindexed,
            DisplacementSize::Long,
        );
        let operand = Operand::Extended {
            base: IndexBase::address(0),
            word,
            displacement: 0x10_0000,
            outer: -0x10_0000,
        };
        let instruction = Instruction::MOVE(operand, operand, Size::Long);
        let words = encode(&instruction, 0x1000).unwrap();
        assert_eq!(words.len() as u32 * 2, MAX_SIZE);
        assert_eq!(
            decode_words(&words, 0x1000, Processor::M68020)
                .unwrap()
                .instruction,
            instruction
        );
    }

    /// A word whose bits 10–8 are clear is the 68000's on every processor,
    /// and the words the 68020 reads as its own are read as the 68000 reads
    /// them, bits 10–8 ignored, by the 68000 and the 68010.
    #[test]
    fn the_68000_and_the_68010_read_the_same_words_as_the_68000_does() {
        // lea (4,a0,d1.w*4),a1
        let scaled = [0x43F0, 0x1404];
        assert_eq!(
            decode_words(&scaled, 0x1000, Processor::M68020)
                .unwrap()
                .instruction,
            Instruction::LEA(
                Operand::Extended {
                    base: IndexBase::address(0),
                    word: ExtensionWord(0x1400),
                    displacement: 4,
                    outer: 0,
                },
                RegisterOperand::Address(1)
            )
        );
        for processor in [Processor::M68000, Processor::M68010] {
            assert_eq!(
                decode_words(&scaled, 0x1000, processor)
                    .unwrap()
                    .instruction,
                Instruction::LEA(
                    Operand::IndirectIndex {
                        base: RegisterOperand::Address(0),
                        offset: 4,
                        index: IndexRegister {
                            register: RegisterOperand::Data(1),
                            size: Size::Word,
                        },
                    },
                    RegisterOperand::Address(1)
                ),
                "{processor:?}"
            );
        }
        // lea (4,a0,d1.w),a1 is the 68000's word on the 68020 as well
        assert!(matches!(
            decode_words(&[0x43F0, 0x1004], 0x1000, Processor::M68020)
                .unwrap()
                .instruction,
            Instruction::LEA(Operand::IndirectIndex { offset: 4, .. }, _)
        ));
        // a full word with a long displacement is two words longer on the 68020
        let full = [0x43F0, 0x1130, 0x0001, 0x0000];
        assert_eq!(
            decode_words(&full, 0x1000, Processor::M68020).unwrap().size,
            8
        );
        assert_eq!(
            decode_words(&full, 0x1000, Processor::M68000).unwrap().size,
            4
        );
    }

    /// GNU `as -m68020`'s words for the manual's forms, read here
    /// (`docs/research/m68k-cpu-variants.md` §4.7 in the asm-editor).
    #[test]
    fn gnu_as_words_decode_to_what_they_say() {
        let cases: [(&[u16], &str); 12] = [
            (&[0x43F0, 0x1404], "LEA 4(A0,D1.W*4),A1"),
            (&[0x43FB, 0x1400], "LEA $1002(PC,D1.W*4),A1"),
            (&[0x43F0, 0x1120, 0x03E8], "LEA (1000.W,A0,D1.W),A1"),
            (
                &[0x43F0, 0x1D30, 0x0001, 0x86A0],
                "LEA (100000.L,A0,D1.L*4),A1",
            ),
            (&[0x43F0, 0x0151], "LEA ([A0]),A1"),
            (
                &[0x43F0, 0x1526, 0x0004, 0x0008],
                "LEA ([4.W,A0],D1.W*4,8.W),A1",
            ),
            (
                &[0x43F0, 0x1522, 0x0004, 0x0008],
                "LEA ([4.W,A0,D1.W*4],8.W),A1",
            ),
            (&[0x43F0, 0x0990], "LEA (D0.L),A1"),
            (&[0x43F3, 0x0990], "LEA (ZA3,D0.L),A1"),
            (&[0x43FB, 0x01D1], "LEA ([ZPC]),A1"),
            (&[0x43F0, 0x0150], "LEA (A0,ZD0),A1"),
            (&[0x43FB, 0x1520, 0x0008], "LEA ($100A.W,PC,D1.W*4),A1"),
        ];
        for (words, text) in cases {
            let decoded = decode_words(words, 0x1000, Processor::M68020).unwrap();
            assert_eq!(decoded.size as usize, words.len() * 2, "{text}");
            assert_eq!(disassemble(&decoded.instruction, None), text);
            assert_eq!(encode(&decoded.instruction, 0x1000).unwrap(), words);
        }
    }

    /// The combinations the manual reserves decode bit by bit, as Musashi
    /// reads them, and print as their words, which no line writes.
    #[test]
    fn a_reserved_combination_is_read_bit_by_bit_and_printed_as_words() {
        // BD SIZE 00: a null displacement; I/IS 100: postindexed, no outer one
        let decoded = decode_words(&[0x43F0, 0x1104], 0x1000, Processor::M68020).unwrap();
        assert_eq!(decoded.size, 4);
        let Instruction::LEA(Operand::Extended { word, .. }, _) = decoded.instruction else {
            panic!("{decoded:?}")
        };
        assert_eq!(word.base_displacement_size(), DisplacementSize::Null);
        assert_eq!(word.indirection(), Indirection::Postindexed);
        assert!(!word.is_canonical());
        let lines = crate::machine::disassemble::disassemble_bytes(
            &[0x43, 0xF0, 0x11, 0x04],
            0x1000,
            Processor::M68020,
            None,
        );
        assert_eq!(lines[0].text, "DC.W $43F0,$1104");
    }
}

mod syntax {
    use super::*;

    /// Every form the manual writes, as `lea` takes it, against the words GNU
    /// `as -m68020` writes for the same line: the brief word with a scale,
    /// the full word with a base displacement, suppressed bases and indexes,
    /// and memory indirection.
    #[test]
    fn every_form_assembles_to_gnu_as_words() {
        let cases: &[(&str, &[u16])] = &[
            ("lea (4,a0,d1.w*4),a1", &[0x43F0, 0x1404]),
            ("lea 4(a0,d1.l*8),a1", &[0x43F0, 0x1E04]),
            ("lea (a0,d1.w*2),a1", &[0x43F0, 0x1200]),
            ("lea (0,a0,d1.w*4),a1", &[0x43F0, 0x1400]),
            ("lea (1000,a0,d1.w),a1", &[0x43F0, 0x1120, 0x03E8]),
            (
                "lea (100000,a0,d1.l*4),a1",
                &[0x43F0, 0x1D30, 0x0001, 0x86A0],
            ),
            ("lea ([a0]),a1", &[0x43F0, 0x0151]),
            ("lea ([4,a0]),a1", &[0x43F0, 0x0161, 0x0004]),
            ("lea ([0,a0]),a1", &[0x43F0, 0x0151]),
            ("lea ([0.w,a0]),a1", &[0x43F0, 0x0161, 0x0000]),
            ("lea ([4,a0],d1.w*4),a1", &[0x43F0, 0x1525, 0x0004]),
            (
                "lea ([4,a0],d1.w*4,8),a1",
                &[0x43F0, 0x1526, 0x0004, 0x0008],
            ),
            (
                "lea ([4,a0,d1.w*4],8),a1",
                &[0x43F0, 0x1522, 0x0004, 0x0008],
            ),
            (
                "lea ([4,a0,d1.w*4],1000000),a1",
                &[0x43F0, 0x1523, 0x0004, 0x000F, 0x4240],
            ),
            ("lea (d0.l),a1", &[0x43F0, 0x0990]),
            ("lea (d0.l*4),a1", &[0x43F0, 0x0D90]),
            ("lea (0,d0.l*4),a1", &[0x43F0, 0x0D90]),
            ("lea (1000,d0.l*4),a1", &[0x43F0, 0x0DA0, 0x03E8]),
            ("lea ([1000]),a1", &[0x43F0, 0x01E1, 0x03E8]),
            ("lea ([1000],d0.l),a1", &[0x43F0, 0x09A5, 0x03E8]),
            ("lea (zpc,d0.l*4),a1", &[0x43FB, 0x0D90]),
            ("lea ([zpc]),a1", &[0x43FB, 0x01D1]),
            ("lea (za3,d0.l),a1", &[0x43F3, 0x0990]),
            ("lea ([a0,zd0]),a1", &[0x43F0, 0x0151]),
            ("lea ([a0],zd0,4.w),a1", &[0x43F0, 0x0152, 0x0004]),
            ("lea (4.w,a0,d0.l),a1", &[0x43F0, 0x0920, 0x0004]),
            ("lea (4.l,a0,d0.l),a1", &[0x43F0, 0x0930, 0x0000, 0x0004]),
            (
                "lea ([4.l,a0],8.l),a1",
                &[0x43F0, 0x0173, 0x0000, 0x0004, 0x0000, 0x0008],
            ),
            ("lea (4.w,a0),a1", &[0x43E8, 0x0004]),
            ("lea (a0,zd0),a1", &[0x43F0, 0x0150]),
            ("lea (-2.w,za0,zd0),a1", &[0x43F0, 0x01E0, 0xFFFE]),
            ("lea (4.w,zpc,d0.w),a1", &[0x43FB, 0x01A0, 0x0004]),
            ("lea ([-4.l,zpc]),a1", &[0x43FB, 0x01F1, 0xFFFF, 0xFFFC]),
            ("lea (100.l,a0,zd0),a1", &[0x43F0, 0x0170, 0x0000, 0x0064]),
            // from the program counter, at $1000: the extension word is at $1002
            ("lea ($100A.w,pc,d1.w*4),a1", &[0x43FB, 0x1520, 0x0008]),
            (
                "lea ([$1010.l,pc],d1.w*4,8.w),a1",
                &[0x43FB, 0x1536, 0x0000, 0x000E, 0x0008],
            ),
            ("lea ($100A,pc,d1.w*4),a1", &[0x43FB, 0x1408]),
            ("lea ([pc],d1.w),a1", &[0x43FB, 0x1115]),
            // `(pc,d1.w*4)` reaches the instruction's first word, as EASy68K
            // assembles `(pc,d1.w)`, where GNU `as` writes a displacement of 0
            ("lea (pc,d1.w*4),a1", &[0x43FB, 0x14FE]),
            // an index with no size is a word, as on the 68000, where GNU `as`
            // makes it a long
            ("lea (d0),a1", &[0x43F0, 0x0190]),
        ];
        for (line, words) in cases {
            assert_eq!(words_of(&format!("    {line}")), *words, "{line}");
        }
    }

    /// The text the disassembler prints for every Operand an assembler writes
    /// assembles back to the same words, at the same address.
    #[test]
    fn what_the_disassembler_prints_assembles_back() {
        let address = 0x2000;
        let mut checked = 0;
        for operand in every_full_operand(address)
            .into_iter()
            .chain(every_brief_operand(address))
        {
            let instruction = Instruction::LEA(operand, RegisterOperand::Address(1));
            let text = disassemble(&instruction, None);
            if text.starts_with("DC.W") {
                continue;
            }
            let words = encode(&instruction, address).unwrap();
            let program = assemble(&format!(
                "    opt p=68020\n    org ${address:X}\n    {text}\n"
            ));
            let assembled = &program.instructions()[0];
            assert_eq!(assembled.instruction, instruction, "`{text}`");
            let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
            assert_eq!(assembled.bytes, bytes, "`{text}`");
            checked += 1;
        }
        assert!(checked > 300, "{checked}");
    }

    /// Under `opt p=68020` a line the 68000 encodes keeps the 68000's words;
    /// a back reference the 68000's field does not hold takes the 68020's
    /// base displacement, the smallest that holds it; a forward reference is
    /// held to the 68000's field, as pass 1 measured it.
    #[test]
    fn a_displacement_is_widened_only_where_the_68000_has_no_encoding() {
        let back = |line: &str| {
            words_of(&format!(
                "near equ 4\nfar equ 200\nvery equ 100000\n    {line}"
            ))
        };
        assert_eq!(back("move.l (near,a0,d0.w),d1"), [0x2230, 0x0004]);
        assert_eq!(back("move.l (far,a0,d0.w),d1"), [0x2230, 0x0120, 0x00C8]);
        assert_eq!(
            back("move.l (very,a0,d0.w),d1"),
            [0x2230, 0x0130, 0x0001, 0x86A0]
        );
        assert_eq!(back("move.l (far,a0),d1"), [0x2228, 0x00C8]);
        assert_eq!(
            back("move.l (very,a0),d1"),
            [0x2230, 0x0170, 0x0001, 0x86A0]
        );
        assert_eq!(
            back("move.l (40000,a0),d1"),
            [0x2230, 0x0170, 0x0000, 0x9C40]
        );
        assert_eq!(back("move.l ([far,a0]),d1"), [0x2230, 0x0161, 0x00C8]);
        assert_eq!(
            back("move.l ([very,a0],near),d1"),
            [0x2230, 0x0172, 0x0001, 0x86A0, 0x0004]
        );
        // the 68000's own words, for every processor
        for processor in ["68000", "68010", "68020"] {
            let program = assemble(&format!(
                "    opt p={processor}\n    org $1000\n    move.l (4,a0,d0.w),d1\n    move.l (-8,a0),d2\n"
            ));
            let bytes: Vec<u8> = program
                .instructions()
                .iter()
                .flat_map(|instruction| instruction.bytes.clone())
                .collect();
            assert_eq!(bytes, [0x22, 0x30, 0x00, 0x04, 0x24, 0x28, 0xFF, 0xF8]);
        }
        // a forward reference is measured at the 68000's field, and a memory
        // indirect one, which the 68000 has none of, at a long
        let forward = |line: &str| words_of(&format!("    {line}\nnear equ 4\n"));
        assert_eq!(forward("move.l (near,a0,d0.w),d1"), [0x2230, 0x0004]);
        assert_eq!(
            forward("move.l ([near,a0]),d1"),
            [0x2230, 0x0171, 0x0000, 0x0004]
        );
        let assembly = crate::assembler::assemble_source(
            "    opt p=68020\n    move.l (far,a0,d0.w),d1\nfar equ 200\n",
        );
        let errors: Vec<_> = assembly
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code(), "value_out_of_range");
        assert!(
            errors[0]
                .hint()
                .unwrap_or_default()
                .contains("(label.w,a0,d0)"),
            "{:?}",
            errors[0].hint()
        );
    }

    /// An unsized branch back further than a word reaches is long on the
    /// 68020, as vasm makes it, and one forward stays a word, whose hint is
    /// `.l`; `bra.l`, `bsr.l` and `b<cc>.l` are long.
    #[test]
    fn an_unsized_branch_back_too_far_for_a_word_is_long() {
        let program = assemble(
            "    opt p=68020
    org $1000
back:
    bra.l ahead
    bsr.l back
    beq.l ahead
    ds.b 40000
    bra back
    bne back
here:
    bra.s here
    bra here
ahead:
    rts
",
        );
        let sizes: Vec<usize> = program.instructions().iter().map(|i| i.size).collect();
        assert_eq!(sizes, [6, 6, 6, 6, 6, 2, 2, 2]);
        let assembly = crate::assembler::assemble_source(
            "    opt p=68020\n    org $1000\n    bra ahead\n    ds.b 40000\nahead:\n    rts\n",
        );
        let error = assembly
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.is_error())
            .expect("a branch forward past a word");
        assert_eq!(error.code(), "value_out_of_range");
        for processor in ["68000", "68010"] {
            let assembly = crate::assembler::assemble_source(&format!(
                "    opt p={processor}\n    org $1000\nback:\n    ds.b 40000\n    bra back\n"
            ));
            assert!(assembly.program.is_none(), "{processor}: no long branch");
        }
    }

    /// The 68020's forms are refused in a Program for the 68000 or the 68010,
    /// with the `opt p=68020` line as the fix; `(d0)` says the 68000's way
    /// too.
    #[test]
    fn the_68020s_modes_need_the_68020() {
        for processor in ["68000", "68010"] {
            for operand in [
                "(4,a0,d1.w*4)",
                "([4,a0],d1.w,8)",
                "(d0.l*4)",
                "(4.l,a0)",
                "(a0,zd0)",
                "(zpc,d0.w)",
                "(4,pc,d1.w*2)",
            ] {
                let assembly = crate::assembler::assemble_source(&format!(
                    "    opt p={processor}\n    lea {operand},a1\n"
                ));
                let error = assembly
                    .diagnostics
                    .iter()
                    .find(|diagnostic| diagnostic.is_error())
                    .unwrap_or_else(|| panic!("{operand} on the {processor}"));
                assert_eq!(error.code(), "needs_a_later_processor", "{operand}");
                assert_eq!(
                    error.message(),
                    format!(
                        "`{operand}` is a 68020 addressing mode, and this program is written \
                         for the {processor}."
                    )
                );
            }
        }
        //the 68000's way of writing `(dN)` names the register written, and has no fix that
        //writes it, as only the student knows which address register is free
        for (operand, hint) in [
            (
                "(d0)",
                "On the 68000, copy `d0` into an address register, `movea.w d0,a0`, and write \
                 `(a0)`, as a 68000 has no data register indirect mode; or add `opt p=68020` at \
                 the top of the program to write it for the 68020",
            ),
            (
                "(d3)",
                "On the 68000, copy `d3` into an address register, `movea.w d3,a3`, and write \
                 `(a3)`",
            ),
            (
                "(d3.w)",
                "On the 68000, copy `d3` into an address register, `movea.w d3,a3`, and write \
                 `(a3)`",
            ),
            (
                "(d0.l)",
                "On the 68000, copy `d0` into an address register, `movea.l d0,a0`, and write \
                 `(a0)`",
            ),
            (
                "(d7)",
                "On the 68000, copy `d7` into an address register, `movea.w d7,a6`, and write \
                 `(a6)`",
            ),
        ] {
            let assembly = crate::assembler::assemble_source(&format!("    lea {operand},a1\n"));
            let error = &assembly.diagnostics[0];
            let written = error.hint().unwrap_or_default();
            assert!(written.starts_with(hint), "{operand}: {written}");
            assert_eq!(error.fixes.len(), 1, "{operand}: only the `opt p=` line");
        }
        let assembly = crate::assembler::assemble_source("    lea (d0.w*4),a1\n");
        assert!(
            !assembly.diagnostics[0]
                .hint()
                .unwrap_or_default()
                .contains("copy"),
            "a scaled index has no 68000 equivalent"
        );
        // a scale of 1 is the 68000's word
        assert!(
            crate::assembler::assemble_source("    lea (4,a0,d1.w*1),a1\n")
                .program
                .is_some()
        );
    }
}

mod running {
    use super::*;

    /// The address every mode names, against the manual's sums worked out by
    /// hand: the base, the base displacement, the index scaled, and for
    /// memory indirection the long read on the way and the outer
    /// displacement.
    #[test]
    fn every_mode_names_the_address_the_manual_sums_to() {
        let mut interpreter = machine(
            "    opt p=68020
    org $1000
start:
    lea pointers-8,a0
    moveq #2,d1
    move.l #-1,d2
    lea (4,a0,d1.w*4),a1
    lea (d1.l*8),a2
    lea (100000,a0,d1.l),a3
    lea ([8,a0]),a4
    lea ([4,a0,d1.w*2],8),a5
    lea ([8,a0],d1.w*4,4),a6
    move.l a1,d3
    lea (-4,a1,d2.w*2),a1
    move.l a1,d4
    move.l ([pointers],d1.w*4),d5
    move.l ([pointers,pc],d1.l*2,-4),d6
    lea (pointers,zpc),a1
    move.l a1,d7
    simhalt
pointers:
    dc.l table
table:
    dc.l $11,$22,$33,$44,$55,$66
",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        let pointers = interpreter
            .get_program()
            .symbols()
            .get("pointers")
            .unwrap()
            .value as u32;
        let table = pointers + 4;
        let a0 = pointers - 8;
        assert_eq!(a(&interpreter, 0), a0);
        assert_eq!(d(&interpreter, 3), a0 + 4 + 2 * 4, "(4,a0,d1.w*4)");
        assert_eq!(a(&interpreter, 2), 16, "(d1.l*8)");
        assert_eq!(a(&interpreter, 3), a0 + 100_000 + 2, "(100000,a0,d1.l)");
        assert_eq!(a(&interpreter, 4), table, "([8,a0]) reads the pointer");
        assert_eq!(a(&interpreter, 5), table + 8, "([4,a0,d1.w*2],8)");
        assert_eq!(a(&interpreter, 6), table + 8 + 4, "([8,a0],d1.w*4,4)");
        assert_eq!(
            d(&interpreter, 4),
            a0 + 4 + 8 - 4 - 2,
            "-1 in d2.w, scaled by 2"
        );
        assert_eq!(d(&interpreter, 5), 0x33, "([pointers],d1.w*4)");
        assert_eq!(d(&interpreter, 6), 0x11, "([pointers,pc],d1.l*2,-4)");
        assert_eq!(
            d(&interpreter, 7),
            pointers,
            "(pointers,zpc) adds no program counter"
        );
        // and undo takes every step back
        while interpreter.can_undo() {
            interpreter.undo().unwrap();
        }
        assert_eq!(a(&interpreter, 5), 0);
    }

    /// The long branches run: `bsr.l` past 32 KB and back, `bra.l`, and
    /// `beq.l` taken and not taken, and an unsized branch back too far for a
    /// word, which is long.
    #[test]
    fn the_long_branches_run_past_a_word() {
        let mut interpreter = machine(
            "    opt p=68020
    org $1000
start:
    moveq #0,d0
    bsr.l far
    cmp.l #1,d0
    beq.l done
    simhalt
done:
    moveq #2,d1
    bra.l ahead
back:
    moveq #3,d2
    simhalt
    ds.b 40000
far:
    addq.l #1,d0
    rts
ahead:
    beq.l start
    bra back
",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(
            (d(&interpreter, 0), d(&interpreter, 1), d(&interpreter, 2)),
            (1, 2, 3)
        );
        let sizes: Vec<usize> = interpreter
            .get_program()
            .instructions()
            .iter()
            .map(|instruction| instruction.size)
            .collect();
        assert_eq!(
            sizes[sizes.len() - 2..],
            [6, 6],
            "`beq.l start` and `bra back`"
        );
    }

    /// A memory indirect Operand is written through, and read and written
    /// back by one instruction at the address its read found; a jump goes
    /// where its pointer says.
    #[test]
    fn a_memory_indirect_operand_is_written_and_jumped_through() {
        let mut interpreter = machine(
            "    opt p=68020
    org $1000
start:
    moveq #2,d1
    move.l #$99,([pointers],d1.w*4)
    addq.l #1,([pointers],d1.w*4)
    move.l (table+8),d2
    jmp ([jumps])
    simhalt
there:
    moveq #7,d3
    simhalt
pointers:
    dc.l table
jumps:
    dc.l there
table:
    dc.l 0,0,0,0
",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 2), 0x9A);
        assert_eq!(d(&interpreter, 3), 7);
    }
}
