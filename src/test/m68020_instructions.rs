//! The 68020 dialect's integer instructions (ADR 0011; the asm-editor's
//! `docs/design/m68k-machine.md`, decision 3, and the plan's milestone C13):
//! `chk.l`, `link.l`, the 68020's `tst`, `cmpi` and `cmp` against the program
//! counter, and `trapcc`.
//!
//! Each is checked the way the manual describes it, flags and edge cases
//! included: its words against GNU `as -m68020`'s, written down here from a
//! run of it, the words decoded back and printed as a line that assembles
//! to them again; what it does when it runs; and that a Program for an
//! earlier processor refuses the line and decodes the words as illegal
//! instructions. Undo puts back every step.

use crate::assembler::program::Program;
use crate::instructions::{Instruction, RegisterOperand, Size};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError};
use crate::machine::decode::{decode_words, is_legal};
use crate::machine::disassemble::disassemble;
use crate::machine::encode::encode;
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

/// The codes of the errors `code` raises.
fn errors(code: &str) -> Vec<String> {
    crate::assembler::assemble_source(code)
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code().to_string())
        .collect()
}

/// The messages and hints of the errors `code` raises.
fn messages(code: &str) -> Vec<String> {
    crate::assembler::assemble_source(code)
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| {
            format!(
                "{} {}",
                diagnostic.message(),
                diagnostic.hint().unwrap_or_default()
            )
        })
        .collect()
}

/// An Interpreter for `body`, written for the 68020, at `$1000`.
fn m68020(body: &str) -> Interpreter {
    Interpreter::new(
        assemble(&format!("    opt p=68020\n    org $1000\nstart:\n{body}")),
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

fn label(interpreter: &Interpreter, name: &str) -> u32 {
    interpreter
        .get_program()
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no label `{name}`"))
        .value as u32
}

/// The condition codes, `XNZVC` in the low five bits.
fn ccr(interpreter: &Interpreter) -> u16 {
    interpreter.get_cpu().get_sr() & 0x1F
}

/// Runs to the `simhalt` that ends every program here.
fn run_to_the_halt(interpreter: &mut Interpreter) {
    assert_eq!(
        interpreter.run(),
        Ok(InterpreterStatus::Paused),
        "{:?}",
        interpreter.get_termination()
    );
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

/// Steps `interpreter` to its end, then undoes every step and checks that
/// each undo puts back the registers, the status register, the program
/// counter and the memory it found.
fn undo_restores_every_step(mut interpreter: Interpreter) {
    let snapshot = |interpreter: &Interpreter| {
        (
            interpreter.get_cpu().get_register_values(),
            interpreter.get_cpu().get_sr(),
            interpreter.get_pc(),
            interpreter
                .get_memory()
                .read_bytes(0, 0x4000)
                .unwrap()
                .to_vec(),
        )
    };
    let mut snapshots = vec![snapshot(&interpreter)];
    loop {
        let stepped = interpreter.step();
        snapshots.push(snapshot(&interpreter));
        match stepped {
            Ok(InterpreterStatus::Running) => continue,
            _ => break,
        }
    }
    snapshots.pop();
    while let Some(expected) = snapshots.pop() {
        if snapshots.is_empty() {
            break;
        }
        interpreter.undo().unwrap();
        assert!(snapshot(&interpreter) == expected, "an undo differs");
    }
}

/// Every line of `lines` assembles under `opt p=68020` to the words GNU
/// `as -m68020` gave it, decodes back to the Instruction it was assembled
/// as, encodes to the same words, and prints as a line that assembles to
/// them again; and the first word is no instruction of the 68000's or the
/// 68010's.
fn check_words(lines: &[(&str, &[u16])]) {
    for (line, expected) in lines {
        let words = words_of(&format!("    {line}"));
        assert_eq!(&words, expected, "`{line}`");
        let decoded = decode_words(&words, 0x1000, Processor::M68020).unwrap();
        assert_eq!(decoded.size as usize, 2 * words.len(), "`{line}`");
        let program = assemble(&format!("    opt p=68020\n    org $1000\n    {line}"));
        assert_eq!(
            decoded.instruction,
            program.instructions()[0].instruction,
            "`{line}`"
        );
        assert_eq!(
            encode(&decoded.instruction, 0x1000).unwrap(),
            words,
            "`{line}`"
        );
        let printed = disassemble(&decoded.instruction, None);
        assert_eq!(words_of(&format!("    {printed}")), words, "`{printed}`");
        for processor in [Processor::M68000, Processor::M68010] {
            assert!(
                !is_legal(words[0], processor),
                "`{line}` is an illegal word on the {}",
                processor.name()
            );
            assert!(matches!(
                decode_words(&words, 0x1000, processor).unwrap().instruction,
                Instruction::IllegalWord(_)
            ));
        }
    }
}

mod chk_link_tst_cmpi {
    use super::*;

    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("chk.l d0,d1", &[0x4300]),
            ("chk.l #$12345678,d7", &[0x4F3C, 0x1234, 0x5678]),
            ("link.l a6,#-8", &[0x480E, 0xFFFF, 0xFFF8]),
            ("link.l a7,#$12345", &[0x480F, 0x0001, 0x2345]),
            ("tst.w a0", &[0x4A48]),
            ("tst.l a5", &[0x4A8D]),
            ("tst.l #5", &[0x4ABC, 0x0000, 0x0005]),
            ("tst.b #-1", &[0x4A3C, 0x00FF]),
            ("tst.w *(pc)", &[0x4A7A, 0xFFFE]),
            ("tst.b (pc,d0.w)", &[0x4A3B, 0x00FE]),
        ]);
        // `cmpi` and `cmp` of an immediate against the program counter, the
        // displacement measured from its own word, after the immediate's
        check_words(&[
            ("cmpi.w #1,*(pc)", &[0x0C7A, 0x0001, 0xFFFC]),
            ("cmpi.l #1,*(pc)", &[0x0CBA, 0x0000, 0x0001, 0xFFFA]),
            ("cmpi.b #1,(pc,d1.l)", &[0x0C3B, 0x0001, 0x18FC]),
        ]);
        assert_eq!(words_of("    cmp.w #1,*(pc)"), [0x0C7A, 0x0001, 0xFFFC]);
        // the 68000's lines keep the 68000's words
        assert_eq!(words_of("    chk d0,d1"), [0x4380]);
        assert_eq!(words_of("    link a6,#-8"), [0x4E56, 0xFFF8]);
        assert_eq!(words_of("    tst.w d0"), [0x4A40]);
    }

    #[test]
    fn a_68000_program_is_told_they_are_the_68020s() {
        for line in [
            "chk.l d0,d1",
            "link.l a6,#-8",
            "tst.w a0",
            "tst.l #5",
            "tst.b *(pc)",
            "cmpi.w #1,*(pc)",
            "cmp.w #1,*(pc)",
        ] {
            for processor in ["68000", "68010"] {
                let code = format!("    opt p={processor}\n    {line}\n");
                assert_eq!(errors(&code), ["needs_a_later_processor"], "{line}");
                assert!(
                    messages(&code)[0].starts_with(&format!("`{line}` requires the 68020 processor")),
                    "{:?}",
                    messages(&code)
                );
            }
        }
        // what the 68020 does not have either
        assert_eq!(
            errors("    opt p=68020\n    tst.b a0\n"),
            ["address_register_byte_size"]
        );
        assert_eq!(
            errors("    opt p=68020\n    chk.b d0,d1\n"),
            ["invalid_size"]
        );
        assert_eq!(
            errors("    opt p=68020\n    link a6,#$12345\n"),
            ["value_out_of_range"],
            "an unsized `link` is the 68000's word"
        );
        assert_eq!(
            errors("    opt p=68020\n    cmpi.w #1,#2\n"),
            ["invalid_addressing_mode"]
        );
    }

    /// `link.w` is the 68020 manual's spelling of `link`, which GNU `as` and
    /// vasm take: under `opt p=68020` it is the 68000's `link`, its words and
    /// its range. EASy68K refuses the size on the 68000 ("Invalid size
    /// code"), so a 68000 or a 68010 program is told it is the 68020's and
    /// offered the `link` it writes, without the size, beside `opt p=68020`.
    #[test]
    fn link_w_is_the_68020s_spelling_of_link() {
        assert_eq!(words_of("    link.w a6,#-8"), [0x4E56, 0xFFF8]);
        assert_eq!(words_of("    LINK.W A5,#$7FFF"), [0x4E55, 0x7FFF]);
        assert_eq!(
            errors("    opt p=68020\n    link.w a6,#$12345\n"),
            ["value_out_of_range"],
            "a word, as `link` is"
        );
        for processor in ["68000", "68010"] {
            let code = format!("    opt p={processor}\n    link.w a6,#-8\n");
            let assembly = crate::assembler::assemble_source(&code);
            let refused: Vec<_> = assembly
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .collect();
            assert_eq!(refused.len(), 1, "{processor}");
            assert_eq!(refused[0].code(), "needs_a_later_processor");
            assert!(
                refused[0]
                    .message()
                    .starts_with("`link.w a6,#-8` requires the 68020 processor"),
                "{}",
                refused[0].message()
            );
            let fixes: Vec<(&str, &str)> = refused[0]
                .fixes
                .iter()
                .map(|fix| (fix.title.as_str(), fix.text.as_str()))
                .collect();
            assert_eq!(
                fixes,
                [
                    (
                        format!("Change `opt p={processor}` to `opt p=68020`").as_str(),
                        "68020"
                    ),
                    ("Write `link a6,#-8`", ""),
                ],
                "{processor}"
            );
            let size = &refused[0].fixes[1].location;
            assert_eq!((size.column, size.end_column), (8, 10), "the `.w`");
        }
    }

    /// `chk.l` compares the whole register, signed, with 0 and the bound;
    /// out of bounds is vector 6, and the stacked program counter is the next
    /// instruction's. N is set below zero and clear above the bound; Z, V and
    /// C are undefined, and are what the 68020 leaves.
    #[test]
    fn chk_l_traps_outside_0_and_the_bound() {
        for (value, bound, traps, flags) in [
            (0u32, 0u32, false, 0b00100),
            (5, 5, false, 0b00000),
            (0x7FFF_FFFF, 0x7FFF_FFFF, false, 0b00000),
            //above a word, which `chk.w` would see as negative
            (0x0001_0000, 0x0002_0000, false, 0b00000),
            (6, 5, true, 0b00001),
            (0xFFFF_FFFF, 5, true, 0b01001),
            (0x8000_0000, 0x7FFF_FFFF, true, 0b01011),
            (1, 0xFFFF_FFFF, true, 0b00000),
            (0xFFFF_FFFE, 0xFFFF_FFFF, true, 0b01000),
        ] {
            let mut interpreter = m68020(&format!(
                "    move.l #handler,$18
    moveq #0,d6
    move.l #${value:X},d0
    move.l #${bound:X},d1
    move #0,ccr
    chk.l d1,d0
back:
    simhalt
handler:
    move.w sr,d6
    move.l 2(a7),d5
    rte"
            ));
            run_to_the_halt(&mut interpreter);
            match traps {
                true => {
                    assert_eq!(d(&interpreter, 6) & 0x1F, flags, "{value:X} {bound:X}");
                    assert_eq!(d(&interpreter, 5), label(&interpreter, "back"));
                }
                false => {
                    assert_eq!(d(&interpreter, 6), 0, "{value:X} {bound:X}");
                    assert_eq!(ccr(&interpreter) as u32, flags, "{value:X} {bound:X}");
                }
            }
        }
        undo_restores_every_step(m68020(
            "    move.l #handler,$18
    move.l #-1,d0
    chk.l #4,d0
    simhalt
handler:
    rte",
        ));
    }

    #[test]
    fn link_l_reserves_a_frame_past_a_word() {
        let mut interpreter = m68020(
            "    move.l #$11223344,a6
    move.l a7,d7
    link.l a6,#-$12340
    move.l a7,d1
    move.l a6,d2
    unlk a6
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        let top = d(&interpreter, 7);
        assert_eq!(d(&interpreter, 2), top - 4, "A6 is the frame pointer");
        assert_eq!(d(&interpreter, 1), top - 4 - 0x12340);
        assert_eq!(a(&interpreter, 6), 0x1122_3344, "`unlk` pops it back");
        assert_eq!(a(&interpreter, 7), top);
        undo_restores_every_step(m68020(
            "    link.l a6,#-$10
    unlk a6
    simhalt",
        ));
    }

    #[test]
    fn tst_reads_an_address_register_the_program_and_an_immediate() {
        let mut interpreter = m68020(
            "    move.l #$80000000,a0
    tst.l a0
    move.w sr,d1
    tst.w a0
    move.w sr,d2
    tst.b value(pc)
    move.w sr,d3
    tst.l #0
    move.w sr,d4
    simhalt
value:
    dc.b $80",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1) & 0x1F, 0b01000);
        assert_eq!(d(&interpreter, 2) & 0x1F, 0b00100, "the low word is 0");
        assert_eq!(d(&interpreter, 3) & 0x1F, 0b01000);
        assert_eq!(d(&interpreter, 4) & 0x1F, 0b00100);
    }

    #[test]
    fn cmpi_compares_with_the_program() {
        let mut interpreter = m68020(
            "    cmpi.w #$1234,value(pc)
    seq d1
    cmpi.l #$12345679,value(pc)
    move.w sr,d2
    cmp.b #$12,value(pc)
    seq d3
    moveq #1,d0
    cmpi.b #$56,value(pc,d0.w*2)
    seq d4
    simhalt
value:
    dc.l $12345678",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1) & 0xFF, 0xFF);
        assert_eq!(d(&interpreter, 2) & 0x1F, 0b01001, "a borrow, negative");
        assert_eq!(d(&interpreter, 3) & 0xFF, 0xFF);
        assert_eq!(d(&interpreter, 4) & 0xFF, 0xFF);
    }
}

mod trapcc {
    use super::*;
    use crate::instructions::{Condition, TrapOperand};

    /// The sixteen conditions and their two aliases, each with no operand,
    /// a word and a long: `$5xFC`, `$5xFA` and `$5xFB`.
    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("trapeq", &[0x57FC]),
            ("trapeq.w #1", &[0x57FA, 0x0001]),
            ("trapne.l #1", &[0x56FB, 0x0000, 0x0001]),
            ("trapcc", &[0x54FC]),
            ("trapt", &[0x50FC]),
            ("trapf.w #$FFFF", &[0x51FA, 0xFFFF]),
            ("traple.l #-1", &[0x5FFB, 0xFFFF, 0xFFFF]),
        ]);
        assert_eq!(words_of("    traphs"), [0x54FC]);
        assert_eq!(words_of("    traplo"), [0x55FC]);
        assert_eq!(
            words_of("    traphi #5"),
            [0x52FA, 0x0005],
            "a word with no size"
        );
        for condition in Condition::ALL {
            for operand in [
                TrapOperand::None,
                TrapOperand::Word(0x8001),
                TrapOperand::Long(0x8000_0001),
            ] {
                let instruction = Instruction::TRAPcc(condition, operand);
                let words = encode(&instruction, 0x1000).unwrap();
                assert_eq!(
                    decode_words(&words, 0x1000, Processor::M68020)
                        .unwrap()
                        .instruction,
                    instruction
                );
                let printed = disassemble(&instruction, None);
                assert_eq!(words_of(&format!("    {printed}")), words, "{printed}");
            }
        }
        assert_eq!(
            errors("    opt p=68020\n    trapeq.b #1\n"),
            ["invalid_size"]
        );
        assert_eq!(
            errors("    opt p=68020\n    trapeq.w #$10000\n"),
            ["immediate_out_of_range"]
        );
        assert_eq!(errors("    trapeq\n"), ["needs_a_later_processor"]);
    }

    /// A true condition is the exception of vector 7, the program counter
    /// stacked the next instruction's, past the operand; a false one does
    /// nothing. The flags are left as they were.
    #[test]
    fn a_true_condition_traps_through_vector_7() {
        let suffixes = [
            "t", "f", "hi", "ls", "cc", "cs", "ne", "eq", "vc", "vs", "pl", "mi", "ge", "lt", "gt",
            "le",
        ];
        for ccr_value in 0..16u16 {
            for (index, suffix) in suffixes.iter().enumerate() {
                for operand in ["", ".w #1", ".l #2"] {
                    let mut interpreter = m68020(&format!(
                        "    move.l #handler,$1C
    moveq #0,d6
    move #{ccr_value},ccr
    trap{suffix}{operand}
back:
    move.w sr,d7
    simhalt
handler:
    moveq #1,d6
    move.l 2(a7),d5
    rte"
                    ));
                    run_to_the_halt(&mut interpreter);
                    let expected =
                        interpreter.get_condition_value(&Condition::from_code(index as u16)) as u32;
                    assert_eq!(
                        d(&interpreter, 6),
                        expected,
                        "trap{suffix}{operand} with {ccr_value:04b}"
                    );
                    if expected == 1 {
                        assert_eq!(d(&interpreter, 5), label(&interpreter, "back"));
                    }
                    assert_eq!(d(&interpreter, 7) & 0x1F, ccr_value as u32);
                }
            }
        }
    }

    #[test]
    fn with_no_handler_the_program_ends_with_its_exception() {
        let mut interpreter = m68020("    trapt\n    simhalt");
        assert_eq!(interpreter.run(), Err(RuntimeError::ConditionalTrap));
        undo_restores_every_step(m68020(
            "    move.l #handler,$1C
    trapt.w #3
    simhalt
handler:
    rte",
        ));
    }
}

mod long_multiply_and_divide {
    use super::*;

    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("muls.l d0,d1:d2", &[0x4C00, 0x2C01]),
            ("muls.l d0,d1", &[0x4C00, 0x1800]),
            ("mulu.l (a0),d3", &[0x4C10, 0x3000]),
            ("mulu.l #$12345,d6:d7", &[0x4C3C, 0x7406, 0x0001, 0x2345]),
            ("divsl.l d0,d1:d2", &[0x4C40, 0x2801]),
            ("divs.l d0,d1:d2", &[0x4C40, 0x2C01]),
            ("divs.l d0,d1", &[0x4C40, 0x1801]),
            ("divu.l #7,d5", &[0x4C7C, 0x5005, 0x0000, 0x0007]),
            ("divul.l d2,d6:d7", &[0x4C42, 0x7006]),
            ("divu.l -(a1),d0:d3", &[0x4C61, 0x3400]),
        ]);
        // the word forms are the 68000's, a size or none
        assert_eq!(words_of("    muls d0,d1"), [0xC3C0]);
        assert_eq!(words_of("    divu.w d0,d1"), [0x82C0]);
        // a pair needs no size
        assert_eq!(words_of("    muls d0,d1:d2"), [0x4C00, 0x2C01]);
        assert_eq!(words_of("    divsl d0,d1:d2"), [0x4C40, 0x2801]);
        for line in [
            "muls.l d0,d1",
            "mulu.l d0,d1:d2",
            "divs.l d0,d1",
            "divul.l d0,d1:d2",
        ] {
            assert_eq!(
                errors(&format!("    opt p=68010\n    {line}\n")),
                ["needs_a_later_processor"],
                "{line}"
            );
        }
        for (line, code) in [
            ("muls.w d0,d1:d2", "invalid_size"),
            ("divsl.l d0,d1", "invalid_addressing_mode"),
            ("muls.l a0,d1", "invalid_addressing_mode"),
            ("muls.l d0,a1:d2", "malformed_operand"),
            ("muls.l d0,d1:", "malformed_operand"),
            ("divs.l d0,(a0):(a1)", "invalid_addressing_mode"),
        ] {
            assert_eq!(
                errors(&format!("    opt p=68020\n    {line}\n")),
                [code],
                "{line}"
            );
        }
    }

    /// The low 32 bits of the product, with V when they lost some, or all 64
    /// into `Dh:Dl`, N and Z of what is kept, C cleared, X kept.
    #[test]
    fn a_long_multiply_keeps_32_or_64_bits() {
        let mut interpreter = m68020(
            "    lea flags,a6
    move #$10,ccr
    moveq #-2,d2
    muls.l #3,d2
    move.w sr,(a6)+
    moveq #-1,d4
    mulu.l #2,d4
    move.w sr,(a6)+
    moveq #1,d7
    muls.l #-1,d6:d7
    move.w sr,(a6)+
    moveq #-1,d0
    mulu.l #-1,d1:d0
    move.w sr,(a6)+
    move.l #$40000000,d3
    muls.l #2,d3
    move.w sr,(a6)+
    simhalt
flags:
    ds.w 5",
        );
        run_to_the_halt(&mut interpreter);
        let flags = |index: u32| {
            let at = label(&interpreter, "flags") + 2 * index;
            interpreter.get_memory().read_word(at as usize).unwrap() & 0x1F
        };
        assert_eq!(d(&interpreter, 2), (-6i32) as u32);
        assert_eq!(flags(0), 0b11000, "X kept");
        assert_eq!(d(&interpreter, 4), 0xFFFF_FFFE);
        assert_eq!(flags(1), 0b11010, "unsigned, a lost carry");
        assert_eq!(
            (d(&interpreter, 6), d(&interpreter, 7)),
            (0xFFFF_FFFF, 0xFFFF_FFFF)
        );
        assert_eq!(flags(2), 0b11000);
        assert_eq!(d(&interpreter, 0), 1, "$FFFFFFFF squared, low");
        assert_eq!(d(&interpreter, 1), 0xFFFF_FFFE, "and high");
        assert_eq!(flags(3), 0b11000, "N is bit 63, unsigned too");
        assert_eq!(d(&interpreter, 3), 0x8000_0000);
        assert_eq!(flags(4), 0b11010, "signed, past 31 bits");
        // `$10000` squared is 0 in 32 bits, with V
        let mut interpreter = m68020(
            "    move.l #$10000,d0
    muls.l d0,d0
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0);
        assert_eq!(ccr(&interpreter), 0b00110);
        undo_restores_every_step(m68020(
            "    moveq #-3,d0
    muls.l #7,d1:d0
    mulu.l d0,d0
    simhalt",
        ));
    }

    /// The quotient into `Dq`, the remainder into `Dr`, of the dividend's
    /// sign; a quotient past 32 bits sets V and leaves the registers; 0 is
    /// vector 5.
    #[test]
    fn a_long_divide_keeps_the_quotient_and_the_remainder() {
        let mut interpreter = m68020(
            "    moveq #-20,d0
    divs.l #7,d0
    move.w sr,d7
    moveq #-20,d1
    moveq #0,d2
    divsl.l #7,d2:d1
    moveq #1,d3
    moveq #0,d4
    divu.l #2,d3:d4
    move.l #$12345678,d5
    move.l #$00000002,d6
    divu.l #1,d6:d5
    move.w sr,d3
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), (-2i32) as u32, "truncated toward 0");
        assert_eq!(d(&interpreter, 7) & 0x1F, 0b01000);
        assert_eq!(d(&interpreter, 1), (-2i32) as u32);
        assert_eq!(d(&interpreter, 2), (-6i32) as u32, "the dividend's sign");
        assert_eq!(d(&interpreter, 4), 0x8000_0000, "$1_00000000 by 2");
        assert_eq!(
            (d(&interpreter, 5), d(&interpreter, 6)),
            (0x1234_5678, 2),
            "an overflow leaves both"
        );
        assert_eq!(
            d(&interpreter, 3) & 0x1F,
            0b00010,
            "V, N and Z of the low half"
        );
        // $80000000 by -1 is no 32-bit quotient
        let mut interpreter = m68020(
            "    move.l #$80000000,d0
    divs.l #-1,d0
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x8000_0000);
        assert_eq!(ccr(&interpreter), 0b01010);
        // a division by zero, signed and unsigned, with the flags it stacks
        for (mnemonic, flags) in [("divs.l", 0b00100), ("divu.l", 0b00110)] {
            let mut interpreter = m68020(&format!(
                "    move.l #handler,$14
    moveq #0,d0
    {mnemonic} d0,d0
back:
    simhalt
handler:
    move.w (a7),d7
    move.l 2(a7),d6
    rte"
            ));
            run_to_the_halt(&mut interpreter);
            assert_eq!(d(&interpreter, 7) & 0x1F, flags, "{mnemonic}");
            assert_eq!(d(&interpreter, 6), label(&interpreter, "back"));
        }
        let mut interpreter = m68020("    divul.l #0,d0:d1\n    simhalt");
        assert_eq!(interpreter.run(), Err(RuntimeError::DivisionByZero));
        undo_restores_every_step(m68020(
            "    moveq #-20,d1
    moveq #-1,d2
    divs.l #3,d2:d1
    divsl.l #5,d3:d1
    simhalt",
        ));
    }
}

mod pack_and_unpack {
    use super::*;

    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("pack -(a0),-(a1),#5", &[0x8348, 0x0005]),
            ("pack d0,d1,#5", &[0x8340, 0x0005]),
            ("unpk d0,d1,#$3030", &[0x8380, 0x3030]),
            ("unpk -(a7),-(a6),#-1", &[0x8D8F, 0xFFFF]),
        ]);
        for (line, code) in [
            ("pack d0,-(a1),#0", "invalid_addressing_mode"),
            ("pack.w d0,d1,#0", "invalid_size"),
            ("pack d0,d1,#$10000", "immediate_out_of_range"),
            ("unpk d0,d1", "wrong_operand_count"),
        ] {
            assert_eq!(
                errors(&format!("    opt p=68020\n    {line}\n")),
                [code],
                "{line}"
            );
        }
        assert_eq!(errors("    unpk d0,d1,#0\n"), ["needs_a_later_processor"]);
    }

    /// `pack` takes the low digit of each byte of a word plus the
    /// adjustment; `unpk` spreads a byte's two digits into a word and adds
    /// it: ASCII and back. In memory `pack` reads the word below `Ax`, two
    /// bytes whichever register, and `unpk` writes one below `Ay`.
    #[test]
    fn pack_and_unpk_turn_ascii_digits_into_decimal_and_back() {
        let mut interpreter = m68020(
            "    move.l #$FFFF3132,d0
    move.l #$AAAAAAAA,d1
    pack d0,d1,#0
    move.l #$55,d2
    move.l #$BBBBBBBB,d3
    unpk d2,d3,#$3030
    lea text+2,a0
    lea packed+1,a1
    pack -(a0),-(a1),#0
    lea packed+1,a2
    lea spread+2,a3
    unpk -(a2),-(a3),#$3030
    move.l #$12345678,d6
    move #$1F,ccr
    pack d6,d7,#-1
    move.w sr,d5
    simhalt
text:
    dc.b '98'
packed:
    dc.b 0
spread:
    dc.w 0",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 0xAAAA_AA12, "the low byte only");
        assert_eq!(d(&interpreter, 3), 0xBBBB_3535, "the low word only");
        let packed = label(&interpreter, "packed");
        let memory = interpreter.get_memory();
        assert_eq!(memory.read_bytes(packed as usize, 1).unwrap()[..], [0x98]);
        assert_eq!(
            memory
                .read_bytes(label(&interpreter, "spread") as usize, 2)
                .unwrap()[..],
            [0x39, 0x38]
        );
        assert_eq!(a(&interpreter, 0), label(&interpreter, "text"));
        assert_eq!(a(&interpreter, 1), packed);
        assert_eq!(d(&interpreter, 7) & 0xFF, 0x67, "$5678 - 1 is $5677");
        assert_eq!(d(&interpreter, 5) & 0x1F, 0x1F, "the flags are left");
        // A7 moves by two for one byte, and two for `pack`'s word
        let mut interpreter = m68020(
            "    lea $3000,a7
    lea $2000,a6
    pack -(a7),-(a6),#0
    lea $3000,a5
    unpk -(a5),-(a7),#0
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(a(&interpreter, 6), 0x1FFF);
        assert_eq!(a(&interpreter, 5), 0x2FFF);
        assert_eq!(a(&interpreter, 7), 0x2FFC);
        undo_restores_every_step(m68020(
            "    lea text+2,a0
    lea text+4,a1
    pack -(a0),-(a1),#0
    unpk -(a1),-(a0),#$3030
    simhalt
text:
    dc.b '1234'",
        ));
    }
}

mod chk2_and_cmp2 {
    use super::*;

    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("chk2.w (a0),d1", &[0x02D0, 0x1800]),
            ("cmp2.l (a0),a1", &[0x04D0, 0x9000]),
            ("chk2.b *(pc),a7", &[0x00FA, 0xF800, 0xFFFC]),
            ("cmp2.w (8,a2,d3.l*4),d0", &[0x02F2, 0x0000, 0x3C08]),
        ]);
        assert_eq!(words_of("    cmp2 $1234,d2"), [0x02F8, 0x2000, 0x1234]);
        for (line, code) in [
            ("chk2.w (a0)+,d1", "invalid_addressing_mode"),
            ("chk2.w d0,d1", "invalid_addressing_mode"),
            ("cmp2.w #1,d1", "invalid_addressing_mode"),
        ] {
            assert_eq!(
                errors(&format!("    opt p=68020\n    {line}\n")),
                [code],
                "{line}"
            );
        }
        assert_eq!(errors("    chk2.w (a0),d1\n"), ["needs_a_later_processor"]);
    }

    /// The register against the bounds, signed when the lower is not above
    /// the upper and the wrapping, unsigned, range otherwise; a data
    /// register at the size, an address register whole against bounds sign
    /// extended. Z says it is on a bound and C outside them; `chk2` traps
    /// through vector 6 there.
    #[test]
    fn a_register_is_checked_against_a_bounds_pair() {
        //(size, lower, upper, register, in an address register, Z, C)
        for (size, lower, upper, value, address, z, c) in [
            ("w", -0x1000, 0x7000, 0x6000u32, false, false, false),
            ("w", -0x1000, 0x7000, 0xF000, false, true, false),
            ("w", -0x1000, 0x7000, 0xE000, false, false, true),
            ("w", -0x1000, 0x7000, 0x7001, false, false, true),
            ("w", -0x1000, 0x7000, 0x1234_6000, false, false, false),
            ("w", -0x1000, 0x7000, 0x6000, true, false, false),
            ("w", -0x1000, 0x7000, 0x1_6000, true, false, true),
            ("w", -0x1000, 0x7000, 0xFFFF_F000, true, true, false),
            //unsigned bounds, the lower above the upper as numbers
            ("b", 0x10, 0xF0, 0x80, false, false, false),
            ("b", 0x10, 0xF0, 0xF1, false, false, true),
            ("b", 0x10, 0xF0, 0x0F, false, false, true),
            ("l", 0x10, 0x7000_0000, 0x7000_0000, false, true, false),
            ("l", 0x10, 0x7000_0000, 0x8000_0000, true, false, true),
        ] {
            let directive = match size {
                "b" => "dc.b",
                "w" => "dc.w",
                _ => "dc.l",
            };
            let register = match address {
                true => "a2",
                false => "d2",
            };
            let code = |mnemonic: &str| {
                format!(
                    "    move.l #handler,$18
    moveq #0,d6
    move.l #${value:X},{register}
    move #$1F,ccr
    {mnemonic}.{size} bounds,{register}
    move.w sr,d7
    simhalt
handler:
    moveq #1,d6
    rte
bounds:
    {directive} {lower},{upper}"
                )
            };
            let mut interpreter = m68020(&code("cmp2"));
            run_to_the_halt(&mut interpreter);
            let flags = d(&interpreter, 7);
            assert_eq!(
                (flags & 4 != 0, flags & 1 != 0),
                (z, c),
                "cmp2.{size} {lower},{upper} with {value:X}"
            );
            assert_eq!(flags & 0x10, 0x10, "X is kept");
            assert_eq!(d(&interpreter, 6), 0);
            let mut interpreter = m68020(&code("chk2"));
            run_to_the_halt(&mut interpreter);
            assert_eq!(
                d(&interpreter, 6),
                c as u32,
                "chk2 traps exactly when C is set"
            );
        }
        let mut interpreter = m68020(
            "    moveq #9,d0
    chk2.b bounds,d0
    simhalt
bounds:
    dc.b 0,8",
        );
        assert_eq!(
            interpreter.run(),
            Err(RuntimeError::Chk2OutOfBounds {
                value: 9,
                lower: 0,
                upper: 8
            })
        );
        undo_restores_every_step(m68020(
            "    move.l #handler,$18
    moveq #9,d0
    chk2.b bounds,d0
    simhalt
handler:
    rte
bounds:
    dc.b 0,8",
        ));
    }

    /// N and V, which Motorola leaves undefined, are the 68020's, as WinUAE
    /// has them from cputester: a few of its cases, worked by hand.
    #[test]
    fn n_and_v_are_the_68020s() {
        //(lower, upper, value, N, V)
        for (lower, upper, value, n, v) in [
            (-10, 10, -11, true, false),
            (-10, 10, 5, true, false),
            (-10, 10, 11, false, false),
            (-10, 10, 10, false, false),
            (10, -10, 5, true, false),
            (10, 20, 5, true, false),
            (10, 20, 30, true, false),
            (10, 20, -5, false, false),
            (-20, -10, -30, true, false),
            (-20, -10, -5, true, false),
            (-0x7FFF_FFFF, 10, 0x7FFF_FFFF, true, true),
            (10, 20, -0x8000_0000, true, true),
            (-20, -10, 0x7FFF_FFFF, true, true),
        ] {
            let mut interpreter = m68020(&format!(
                "    move.l #{value},d0
    cmp2.l bounds,d0
    move.w sr,d7
    simhalt
bounds:
    dc.l {lower},{upper}"
            ));
            run_to_the_halt(&mut interpreter);
            let flags = d(&interpreter, 7);
            assert_eq!(
                (flags & 8 != 0, flags & 2 != 0),
                (n, v),
                "{lower},{upper} with {value}"
            );
        }
    }
}

mod cas_and_cas2 {
    use super::*;

    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("cas.w d0,d1,(a0)", &[0x0CD0, 0x0040]),
            ("cas.b d7,d6,(a5)+", &[0x0ADD, 0x0187]),
            ("cas.l d2,d3,$1234", &[0x0EF8, 0x00C2, 0x1234]),
            ("cas2.l d0:d1,d2:d3,(a0):(d4)", &[0x0EFC, 0x8080, 0x40C1]),
            ("cas2.w d7:d6,d5:d4,(d3):(a2)", &[0x0CFC, 0x3147, 0xA106]),
        ]);
        for (line, code) in [
            ("cas.w d0,d1,d2", "invalid_addressing_mode"),
            ("cas.w d0,a1,(a0)", "invalid_addressing_mode"),
            ("cas.w d0,d1,label(pc)", "invalid_addressing_mode"),
            ("cas2.b d0:d1,d2:d3,(a0):(a1)", "invalid_size"),
            ("cas2.w d0:d1,d2:d3,(a0)", "invalid_addressing_mode"),
            ("cas2.w d0:d1,d2:d3,(a0):d1", "malformed_operand"),
        ] {
            assert_eq!(
                errors(&format!("    opt p=68020\nlabel:\n    {line}\n")),
                [code],
                "{line}"
            );
        }
        assert_eq!(
            errors("    cas.w d0,d1,(a0)\n"),
            ["needs_a_later_processor"]
        );
        assert_eq!(
            errors("    cas2.w d0:d1,d2:d3,(a0):(a1)\n"),
            ["needs_a_later_processor"]
        );
    }

    /// Equal, the update goes to memory; not, memory goes into `Dc`, at the
    /// size; the flags are `cmp`'s; `(An)+` moves its register once.
    #[test]
    fn cas_swaps_when_the_operand_is_the_compared_value() {
        let mut interpreter = m68020(
            "    lea value,a0
    move.l #$11112222,d0
    move.l #$33334444,d1
    cas.w d0,d1,(a0)+
    move.w sr,d7
    move.l #$AAAA0000,d2
    cas.w d2,d1,(a0)
    move.w sr,d6
    simhalt
value:
    dc.w $2222,$8000",
        );
        run_to_the_halt(&mut interpreter);
        let value = label(&interpreter, "value");
        let memory = interpreter.get_memory();
        assert_eq!(memory.read_word(value as usize).unwrap(), 0x4444);
        assert_eq!(d(&interpreter, 7) & 0x1F, 0b00100);
        assert_eq!(a(&interpreter, 0), value + 2);
        assert_eq!(d(&interpreter, 2), 0xAAAA_8000, "the low word only");
        assert_eq!(
            d(&interpreter, 6) & 0x1F,
            0b01000,
            "$8000 - 0 borrows nothing, is N"
        );
        assert_eq!(memory.read_word(value as usize + 2).unwrap(), 0x8000);
        undo_restores_every_step(m68020(
            "    lea value,a0
    moveq #5,d0
    moveq #7,d1
    cas.b d0,d1,(a0)
    cas.b d0,d1,(a0)
    simhalt
value:
    dc.b 5",
        ));
    }

    /// Both equal, both updates are written; either not, both operands go
    /// into the compare registers, the first last; the flags are the last
    /// comparison's.
    #[test]
    fn cas2_swaps_two_operands_or_neither() {
        let mut interpreter = m68020(
            "    lea first,a0
    lea second,a1
    move.l a1,d4
    moveq #1,d0
    moveq #2,d1
    moveq #10,d2
    moveq #20,d3
    cas2.l d0:d1,d2:d3,(a0):(d4)
    move.w sr,d7
    moveq #10,d5
    moveq #21,d6
    cas2.l d5:d6,d2:d3,(a0):(a1)
    move.w sr,d4
    simhalt
first:
    dc.l 1
second:
    dc.l 2",
        );
        run_to_the_halt(&mut interpreter);
        let (first, second) = (label(&interpreter, "first"), label(&interpreter, "second"));
        let memory = interpreter.get_memory();
        assert_eq!(memory.read_long(first as usize).unwrap(), 10);
        assert_eq!(memory.read_long(second as usize).unwrap(), 20);
        assert_eq!(d(&interpreter, 7) & 0x1F, 0b00100);
        assert_eq!(
            (d(&interpreter, 5), d(&interpreter, 6)),
            (10, 20),
            "both read back"
        );
        assert_eq!(d(&interpreter, 4) & 0x1F, 0b01001, "20 - 21 borrows");
        // one register for both compare values keeps the first operand
        let mut interpreter = m68020(
            "    lea first,a0
    lea second,a1
    moveq #0,d0
    cas2.w d0:d0,d1:d1,(a0):(a1)
    simhalt
first:
    dc.w 7
second:
    dc.w 8",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 7);
        undo_restores_every_step(m68020(
            "    lea first,a0
    lea second,a1
    moveq #7,d0
    moveq #8,d1
    moveq #1,d2
    moveq #2,d3
    cas2.w d0:d1,d2:d3,(a0):(a1)
    cas2.w d0:d1,d2:d3,(a0):(a1)
    simhalt
first:
    dc.w 7
second:
    dc.w 8",
        ));
    }
}

mod bit_fields {
    use super::*;
    use crate::machine::Dialect;

    #[test]
    fn the_words_are_gnu_as_words() {
        check_words(&[
            ("bfextu d0{4:8},d1", &[0xE9C0, 0x1108]),
            ("bfextu (a0){d1:d2},d3", &[0xE9D0, 0x3862]),
            ("bfextu (a0){d1:32},d3", &[0xE9D0, 0x3840]),
            ("bftst (8,a0){3:1}", &[0xE8E8, 0x00C1, 0x0008]),
            ("bfexts *(pc){0:31},d7", &[0xEBFA, 0x701F, 0xFFFC]),
            ("bfffo $1234{d7:d6},d0", &[0xEDF8, 0x09E6, 0x1234]),
            ("bfchg d5{31:1}", &[0xEAC5, 0x07C1]),
            ("bfclr (a6){0:32}", &[0xECD6, 0x0000]),
            ("bfset (4,a1,d2.w){d0:4}", &[0xEEF1, 0x0804, 0x2004]),
            ("bfins d4,(a2){7:9}", &[0xEFD2, 0x41C9]),
        ]);
        // a `#` is allowed before each number, as GNU `as` reads it
        assert_eq!(words_of("    bfextu d0{#4:#8},d1"), [0xE9C0, 0x1108]);
        for (line, code) in [
            ("bfextu d0,d1", "invalid_addressing_mode"),
            ("bfextu (a0)+{0:8},d1", "invalid_addressing_mode"),
            ("bfset *(pc){0:8}", "invalid_addressing_mode"),
            ("bfextu d0{0:8},a1", "invalid_addressing_mode"),
            ("move.l d0{0:8},d1", "invalid_addressing_mode"),
            ("bfextu d0{32:8},d1", "value_out_of_range"),
            ("bfextu d0{-1:8},d1", "value_out_of_range"),
            ("bfextu d0{0:33},d1", "value_out_of_range"),
            ("bfextu d0{0:0},d1", "value_out_of_range"),
            ("bfextu d0{a0:8},d1", "malformed_operand"),
            ("bfextu d0{0 8},d1", "malformed_operand"),
            ("bfextu d0{0:8,d1", "malformed_operand"),
            ("bftst.l d0{0:8}", "invalid_size"),
        ] {
            assert_eq!(
                errors(&format!("    opt p=68020\n    {line}\n")),
                [code],
                "{line}"
            );
        }
    }

    /// The bit fields are the 68020's, and EASy68K's `opt bit` gives them to
    /// a 68000 or a 68010 Program, with the 68000's addressing modes; the
    /// decoder reads their words for such a Program as for the 68020.
    #[test]
    fn opt_bit_gives_an_earlier_program_the_bit_fields() {
        for processor in ["68000", "68010"] {
            let code = format!("    opt p={processor}\n    bftst d0{{0:8}}\n");
            assert_eq!(errors(&code), ["needs_a_later_processor"]);
            let found = crate::assembler::assemble_source(&code).diagnostics;
            let titles: Vec<&str> = found[0]
                .fixes
                .iter()
                .map(|fix| fix.title.as_str())
                .collect();
            assert_eq!(
                titles,
                [
                    format!("Change `opt p={processor}` to `opt p=68020`").as_str(),
                    format!("Write `opt p={processor},bit`").as_str()
                ]
            );
            assert!(
                errors(&format!("    opt p={processor},bit\n    bftst d0{{0:8}}\n")).is_empty()
            );
        }
        let found = crate::assembler::assemble_source("    bfset (a0){0:8}\n").diagnostics;
        assert_eq!(
            found[0].fixes[1].title,
            "Add `opt bit` at the top of the program"
        );
        assert_eq!(found[0].fixes[1].text, "    opt bit\n");
        assert!(found[0].hint().unwrap().contains("`opt bit` before the first instruction"));
        assert!(errors("    opt bit\n    bfset (8,a0,d1.w){0:8}\n").is_empty());
        assert_eq!(
            errors("    opt bit\n    bfset (8,a0,d1.w*4){0:8}\n"),
            ["needs_a_later_processor"],
            "the 68020's modes are still the 68020's"
        );
        let program = assemble("    opt bit\n    org $1000\n    bfextu d0{4:8},d1\n    simhalt");
        assert_eq!(program.processor(), Processor::M68000);
        assert_eq!(
            program.dialect(),
            Dialect {
                processor: Processor::M68000,
                bit_fields: true
            }
        );
        for first in [0xE8C0u16, 0xE9D0, 0xEFC0] {
            assert!(!is_legal(first, Processor::M68000));
            assert!(!is_legal(first, Processor::M68010));
            assert!(is_legal(first, program.dialect()));
            assert!(is_legal(first, Processor::M68020));
        }
        // and the Interpreter decodes them for it, read from memory
        let mut interpreter = Interpreter::new(program, None);
        interpreter.set_register_value(RegisterOperand::Data(0), 0x1234_5678, Size::Long);
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 0x23);
        assert_eq!(interpreter.disassemble_at(0x1000).text, "BFEXTU D0{4:8},D1");
    }

    /// A field of a data register starts `offset` bits below the most
    /// significant bit, modulo 32, and wraps round past bit 0; in memory it
    /// starts at the byte the operand names plus the offset over eight,
    /// rounded down, any distance either way, and covers up to five bytes.
    #[test]
    fn a_field_is_found_where_the_manual_says() {
        let mut interpreter = m68020(
            "    move.l #$12345678,d0
    bfextu d0{4:8},d1
    move.l #$F0000000,d0
    bfexts d0{0:4},d2
    move.l #$80000001,d0
    bfextu d0{31:2},d3
    moveq #-1,d4
    bfextu d0{d4:2},d5
    lea bytes,a0
    bfextu (a0){4:32},d6
    lea bytes+2,a1
    moveq #-12,d4
    bfextu (a1){d4:8},d7
    simhalt
bytes:
    dc.b $12,$34,$56,$78,$9A",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 0x23);
        assert_eq!(d(&interpreter, 2), 0xFFFF_FFFF, "sign extended");
        assert_eq!(d(&interpreter, 3), 3, "bit 0, then bit 31");
        assert_eq!(d(&interpreter, 5), 3, "-1 from a register is 31 modulo 32");
        assert_eq!(d(&interpreter, 6), 0x2345_6789, "five bytes");
        assert_eq!(
            d(&interpreter, 7),
            0x23,
            "12 bits back is the byte before, bit 4"
        );
    }

    /// N is the field's most significant bit and Z says it is zeros, of the
    /// field before it is changed, or of the value `bfins` inserts; V and C
    /// are cleared and X kept. Only the bytes of the field are written, and
    /// only its bits change.
    #[test]
    fn each_instruction_changes_the_field_and_sets_the_flags() {
        let mut interpreter = m68020(
            "    lea flags,a6
    move #$1F,ccr
    move.l #$00FF0000,d0
    bftst d0{8:8}
    move.w sr,(a6)+
    bfchg d0{4:8}
    move.w sr,(a6)+
    move.l #$FFFFFFFF,d1
    bfclr d1{30:4}
    move.w sr,(a6)+
    moveq #0,d2
    bfset d2{28:8}
    move.w sr,(a6)+
    move.l #$AB,d3
    moveq #0,d4
    bfins d3,d4{8:8}
    move.w sr,(a6)+
    lea bytes,a0
    bfset (a0){12:8}
    bfclr (a0){28:16}
    bfins d3,(4,a0){4:4}
    simhalt
flags:
    ds.w 5
bytes:
    dc.b $00,$00,$00,$FF,$FF,$FF",
        );
        run_to_the_halt(&mut interpreter);
        let flags = |index: u32| {
            let at = label(&interpreter, "flags") + 2 * index;
            interpreter.get_memory().read_word(at as usize).unwrap() & 0x1F
        };
        assert_eq!(flags(0), 0b11000, "$FF: N, X kept, V and C cleared");
        assert_eq!(d(&interpreter, 0), 0x0F0F_0000, "$0F,$F0 complemented");
        assert_eq!(flags(1), 0b10000);
        assert_eq!(d(&interpreter, 1), 0x3FFF_FFFC, "bits 1, 0, 31 and 30");
        assert_eq!(flags(2), 0b11000);
        assert_eq!(d(&interpreter, 2), 0xF000_000F);
        assert_eq!(flags(3), 0b10100, "it was zeros");
        assert_eq!(d(&interpreter, 4), 0x00AB_0000);
        assert_eq!(flags(4), 0b11000, "the value inserted, $AB");
        let bytes = label(&interpreter, "bytes") as usize;
        assert_eq!(
            interpreter.get_memory().read_bytes(bytes, 6).unwrap()[..],
            [0x00, 0x0F, 0xF0, 0xF0, 0x0B, 0x0F]
        );
        undo_restores_every_step(m68020(
            "    lea bytes,a0
    moveq #-1,d3
    bfset (a0){4:32}
    bfins d3,(a0){0:4}
    bfclr d3{0:16}
    simhalt
bytes:
    dc.b 0,0,0,0,0",
        ));
    }

    /// `bfffo` writes the offset of the field's first set bit, the offset as
    /// written, whole, plus how far in it is; or offset plus width.
    #[test]
    fn bfffo_finds_the_first_set_bit() {
        let mut interpreter = m68020(
            "    move.l #$00100000,d0
    bfffo d0{0:32},d1
    moveq #100,d7
    bfffo d0{d7:16},d2
    bfffo d0{0:8},d3
    move.w sr,d4
    lea bytes,a0
    moveq #-8,d7
    bfffo (a0){d7:16},d5
    simhalt
    dc.b $00
bytes:
    dc.b $40",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 11);
        assert_eq!(d(&interpreter, 2), 107, "100, which is 4, plus 7");
        assert_eq!(d(&interpreter, 3), 8, "none: the offset plus the width");
        assert_eq!(d(&interpreter, 4) & 0x1F, 0b00100);
        assert_eq!(d(&interpreter, 5), 1, "-8 plus 9");
    }
}

/// The bit fields of a 68000 Program under `opt bit` take EASy68K's cycles:
/// the 68020's cache case and the 68000's effective address time, more for a
/// field in five bytes. A 68020 Program's take the 68020's own
/// (`src/test/cycles.rs`).
#[test]
fn the_bit_fields_take_easy68ks_cycles() {
    let mut interpreter = Interpreter::new(
        assemble(
            "    opt bit
    org $1000
    lea bytes,a0
    bftst d0{0:8}
    bfextu (a0){0:8},d1
    bfchg (a0){4:32}
    bfffo 4(a0){0:8},d2
    simhalt
bytes:
    dc.b 1,2,3,4,5,6,7,8",
        ),
        None,
    );
    let mut cycles = Vec::new();
    for _ in 0..5 {
        let before = interpreter.get_cycles();
        interpreter.step().unwrap();
        cycles.push(interpreter.get_cycles() - before);
    }
    assert_eq!(cycles[1..], [7, 13 + 4, 24 + 4, 24 + 8]);
}
