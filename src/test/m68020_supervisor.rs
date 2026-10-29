//! The 68020's supervisor model (ADR 0011; the asm-editor's
//! `docs/design/m68k-machine.md`, decision 3, and the plan's milestone C13):
//! its status register with T1, T0 and M, its three stack pointers and the
//! throwaway frame of an interrupt in the master state, `movec` of MSP, ISP,
//! CACR and CAAR, its frames, `$0`, `$1`, `$2`, `$A` and `$B`, and `rte`'s
//! check of them, words and longs at odd addresses on every path into memory,
//! and line F with no coprocessor.
//!
//! Every program here is written for the 68020 with `opt p=68020` but where a
//! test sets an earlier processor beside it. Undo is checked across the stack
//! pointers and the frames.

use crate::assembler::program::Program;
use crate::debugger::{MutationOperation, SystemRegister};
use crate::instructions::{Bytes, Instruction, InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    CodeWrite, Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, StackPointer,
    Termination, WatchedWrite,
};
use crate::machine::decode::decode_words;
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

/// An Interpreter for `body`, written for `processor`, at `$1000`.
fn on(processor: &str, body: &str) -> Interpreter {
    machine(&format!(
        "    opt p={processor}\n    org $1000\nstart:\n{body}"
    ))
}

/// An Interpreter for `body`, written for the 68020, at `$1000`.
fn m68020(body: &str) -> Interpreter {
    on("68020", body)
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

fn word(interpreter: &Interpreter, address: u32) -> u16 {
    let bytes = bytes(interpreter, address, 2);
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn long(interpreter: &Interpreter, address: u32) -> u32 {
    let bytes = bytes(interpreter, address, 4);
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn bytes(interpreter: &Interpreter, address: u32, length: usize) -> Vec<u8> {
    interpreter
        .get_memory()
        .read_bytes(address as usize, length)
        .unwrap()
        .to_vec()
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

/// Everything the programs here change, to compare across undo.
#[derive(Debug, PartialEq)]
struct Snapshot {
    registers: Vec<u32>,
    system: crate::interpreter::SystemRegisters,
    stack_top: usize,
    pc: usize,
    status: InterpreterStatus,
    termination: Option<Termination>,
    memory: Vec<u8>,
}

fn snapshot(interpreter: &Interpreter) -> Snapshot {
    let mut memory = Vec::new();
    for (start, end) in [
        (0x0, 0x400),
        (0x1000, 0x1400),
        (0x7F00, 0x8100),
        (0xFFFF00, 0x1000000),
    ] {
        memory.extend_from_slice(&bytes(interpreter, start, end - start as usize));
    }
    Snapshot {
        registers: interpreter.get_cpu().get_register_values(),
        system: interpreter.get_system(),
        stack_top: interpreter.get_stack_top(),
        pc: interpreter.get_pc(),
        status: *interpreter.get_status(),
        termination: interpreter.get_termination().cloned(),
        memory,
    }
}

/// Steps `interpreter` to its end, a snapshot before every step, then undoes
/// every step and checks that each undo puts back the snapshot taken before
/// the step it undid.
fn undo_restores_every_step(mut interpreter: Interpreter) {
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
        assert_eq!(snapshot(&interpreter), expected);
    }
}

mod status_register {
    use super::*;

    /// The 68020's status register has T0 and M beside the 68000's bits, so
    /// `$FFFF` reads back `$F71F`; the 68000's and the 68010's read `$A71F`.
    #[test]
    fn the_68020_keeps_t0_and_m() {
        for (processor, expected) in [("68000", 0xA71F), ("68010", 0xA71F), ("68020", 0xF71F)] {
            let mut interpreter = on(processor, "    simhalt");
            interpreter.set_sr(0xFFFF);
            assert_eq!(interpreter.get_sr(), expected, "{processor}");
            let mut interpreter = on(processor, "    move #$3FFF,sr\n    move sr,d0\n    simhalt");
            run_to_the_halt(&mut interpreter);
            assert_eq!(
                d(&interpreter, 0),
                (expected & 0x3FFF) as u32,
                "{processor}"
            );
        }
    }

    /// M makes A7 the master stack pointer in supervisor mode, and clearing
    /// it the interrupt stack pointer again, each keeping its value; `movec`
    /// reads and writes both, A7 itself when it is the one A7 is, and the
    /// memory view follows the stack A7 is on.
    #[test]
    fn m_chooses_the_master_or_the_interrupt_stack_pointer() {
        let body = "    move.l #$8000,d0
    movec d0,msp
    movec msp,d1
    move.l a7,d2
    ori #$1000,sr
    move.l a7,d3
    move.l #$12345678,-(a7)
    movec msp,d4
    movec isp,d5
    move.l #$9000,d0
    movec d0,isp
    andi #$EFFF,sr
    move.l a7,d6
    andi #$DFFF,sr
    move.l a7,d7
    simhalt";
        let mut interpreter = m68020(body);
        for _ in 0..5 {
            interpreter.step().unwrap();
        }
        assert_eq!(interpreter.get_stack_top(), 0x8000, "the master stack's");
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 0x8000);
        assert_eq!(
            d(&interpreter, 2),
            0x0100_0000,
            "A7 is the ISP while M is clear"
        );
        assert_eq!(d(&interpreter, 3), 0x8000, "and the MSP while it is set");
        assert_eq!(d(&interpreter, 4), 0x7FFC, "`movec msp` reads A7");
        assert_eq!(d(&interpreter, 5), 0x0100_0000);
        assert_eq!(d(&interpreter, 6), 0x9000, "the ISP `movec` wrote aside");
        assert_eq!(d(&interpreter, 7), 0x00FF_0000, "the USP in user mode");
        assert_eq!(long(&interpreter, 0x7FFC), 0x1234_5678);
        let system = interpreter.get_system();
        assert_eq!(
            (system.ssp, system.isp, system.msp, system.usp),
            (0x9000, Some(0x9000), Some(0x7FFC), 0x00FF_0000)
        );
        undo_restores_every_step(m68020(body));
    }

    /// A Poke of the 68020's stack pointers is journaled like any other, the
    /// one A7 is included, and a processor without them refuses it.
    #[test]
    fn the_host_pokes_isp_msp_cacr_and_caar() {
        let mut interpreter = m68020("    ori #$1000,sr\n    simhalt");
        interpreter.step().unwrap();
        let before = snapshot(&interpreter);
        interpreter.begin_poke().unwrap();
        for (register, value) in [
            (SystemRegister::Msp, 0x8000),
            (SystemRegister::Isp, 0x9000),
            (SystemRegister::Cacr, 0xFF),
            (SystemRegister::Caar, 0x1234),
        ] {
            interpreter.set_control_register(register, value).unwrap();
        }
        assert!(interpreter.end_poke().unwrap());
        assert_eq!(a(&interpreter, 7), 0x8000, "A7 is the MSP");
        let system = interpreter.get_system();
        assert_eq!(
            (system.ssp, system.msp, system.cacr, system.caar),
            (0x9000, Some(0x8000), Some(3), Some(0x1234))
        );
        interpreter.undo().unwrap();
        assert_eq!(snapshot(&interpreter), before);
        //the System file of a 68010 has none of them, and its setter refuses them
        let mut interpreter = on("68010", "    simhalt");
        let json = serde_json::to_value(interpreter.get_system()).unwrap();
        assert!(
            json.get("msp").is_none() && json.get("cacr").is_none(),
            "{json}"
        );
        assert!(matches!(
            interpreter.set_control_register(SystemRegister::Msp, 0),
            Err(RuntimeError::InvalidArgument(_))
        ));
        assert!(matches!(
            interpreter.set_control_register(SystemRegister::Ssp, 0),
            Err(RuntimeError::InvalidArgument(_))
        ));
    }
}

mod control_registers {
    use super::*;

    /// CACR keeps E and F, its two bits that are not commands to a cache
    /// s68k does not have; CAAR keeps every bit.
    #[test]
    fn movec_keeps_cacr_and_caar() {
        let body = "    moveq #-1,d0
    movec d0,cacr
    movec cacr,d1
    movec d0,caar
    movec caar,a2
    moveq #2,d0
    movec d0,cacr
    movec cacr,d3
    simhalt";
        let mut interpreter = m68020(body);
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 3);
        assert_eq!(a(&interpreter, 2), 0xFFFF_FFFF);
        assert_eq!(d(&interpreter, 3), 2);
        undo_restores_every_step(m68020(body));
    }

    /// The 68020's control registers are illegal instructions on a 68010, by
    /// their words, both ways; `movec` is privileged on the 68020 too.
    #[test]
    fn a_68010_has_none_of_them() {
        for code in [0x002u16, 0x802, 0x803, 0x804] {
            for operation in [0x4E7A, 0x4E7B] {
                let mut interpreter = on(
                    "68010",
                    &format!("    dc.w ${operation:04X},${code:04X}\n    simhalt"),
                );
                assert_eq!(
                    interpreter.run(),
                    Err(RuntimeError::IllegalInstruction),
                    "{operation:04X} {code:03X}"
                );
                let mut interpreter = m68020(&format!(
                    "    dc.w ${operation:04X},${code:04X}\n    simhalt"
                ));
                run_to_the_halt(&mut interpreter);
            }
        }
        let mut interpreter = m68020("    andi #$DFFF,sr\n    movec msp,d0\n    simhalt");
        assert_eq!(interpreter.run(), Err(RuntimeError::PrivilegeViolation));
    }

    /// The four assemble, encode and print as their codes, and a Program for
    /// an earlier processor is told that they are the 68020's.
    #[test]
    fn they_assemble_for_the_68020_alone() {
        let program = assemble(
            "    opt p=68020\n    org $1000\n    movec d0,cacr\n    movec caar,a1\n    \
             movec a7,msp\n    movec isp,d7\n",
        );
        let words: Vec<Vec<u8>> = program
            .instructions()
            .iter()
            .map(|instruction| instruction.bytes.clone())
            .collect();
        assert_eq!(
            words,
            [
                vec![0x4E, 0x7B, 0x00, 0x02],
                vec![0x4E, 0x7A, 0x98, 0x02],
                vec![0x4E, 0x7B, 0xF8, 0x03],
                vec![0x4E, 0x7A, 0x78, 0x04],
            ]
        );
        let decoded = decode_words(&[0x4E7B, 0x0002], 0x1000, Processor::M68020).unwrap();
        assert_eq!(
            crate::machine::disassemble::disassemble(&decoded.instruction, None),
            "MOVEC D0,CACR"
        );
        let assembly = crate::assembler::assemble_source("    opt p=68010\n    movec d0,msp\n");
        let error = assembly
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.is_error())
            .expect("an error");
        assert_eq!(error.code(), "needs_a_later_processor");
        assert_eq!(
            error.message(),
            "`msp` is a 68020 control register, and this program is written for the 68010."
        );
        let assembly = crate::assembler::assemble_source("cacr:\n    nop\n");
        assert!(
            assembly
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code() == "reserved_name_as_symbol"),
            "`cacr` is a register's name in every Program"
        );
    }
}

mod frames {
    use super::*;

    /// The frame `instruction` pushes through `vector`, whose handler keeps
    /// A7 in a6 and the stacked program counter in d6, and returns past the
    /// instruction: its words from the stack pointer up, as it pushed them.
    fn frame_of(
        setup: &str,
        instruction: &str,
        vector: u8,
        words: usize,
    ) -> (Interpreter, Vec<u16>) {
        let mut interpreter = m68020(&format!(
            "    move.l #handler,{}
{setup}
here:
    {instruction}
back:
    simhalt
handler:
    move.l a7,a6
    move.l 2(a7),d6
    move.l #back,2(a7)
    rte",
            vector as u32 * 4
        ));
        run_to_the_halt(&mut interpreter);
        let frame = a(&interpreter, 6);
        let mut frame: Vec<u16> = (0..words as u32)
            .map(|index| word(&interpreter, frame + 2 * index))
            .collect();
        frame[1] = (d(&interpreter, 6) >> 16) as u16;
        frame[2] = d(&interpreter, 6) as u16;
        assert_eq!(
            a(&interpreter, 7),
            0x0100_0000,
            "`rte` pops the {words} words of {instruction}"
        );
        (interpreter, frame)
    }

    /// `chk`, `chk2`, `trapv`, `trapcc` and a division by zero push the six
    /// words of format `$2`, which add the address of the instruction to the
    /// next one's.
    #[test]
    fn the_instruction_traps_push_format_2() {
        for (setup, instruction, vector) in [
            ("    moveq #9,d0", "chk.w #5,d0", 6u8),
            ("    moveq #9,d0", "chk2.b bounds,d0", 6),
            ("    move #2,ccr", "trapv", 7),
            ("", "trapt", 7),
            ("    moveq #0,d1", "divu.w d1,d0", 5),
        ] {
            let setup = format!("{setup}\n    bra.s skip\nbounds:\n    dc.b 1,5\nskip:");
            let (interpreter, frame) = frame_of(&setup, instruction, vector, 6);
            let here = label(&interpreter, "here");
            assert_eq!(frame[3], 0x2000 | vector as u16 * 4, "{instruction}");
            assert_eq!(
                (frame[4] as u32) << 16 | frame[5] as u32,
                here,
                "{instruction}: the address of the instruction"
            );
            let next = (frame[1] as u32) << 16 | frame[2] as u32;
            assert_eq!(
                next,
                label(&interpreter, "back"),
                "{instruction}: and the next one's"
            );
        }
    }

    /// `trap`, an illegal instruction, line A and line F push format `$0`, the
    /// first stacking the next instruction, the others their own.
    #[test]
    fn the_others_push_format_0() {
        for (instruction, vector, own) in [
            ("trap #3", 35u8, false),
            ("illegal", 4, true),
            ("dc.w $A123", 10, true),
            ("dc.w $F200,$0000", 11, true),
        ] {
            let (interpreter, frame) = frame_of("", instruction, vector, 4);
            assert_eq!(frame[3], vector as u16 * 4, "{instruction}");
            let pc = (frame[1] as u32) << 16 | frame[2] as u32;
            let here = label(&interpreter, "here");
            match own {
                true => assert_eq!(pc, here, "{instruction}"),
                false => assert_eq!(pc, here + 2, "{instruction}"),
            }
        }
    }

    /// Line F with no coprocessor: every word of `$F000` to `$FFFF` decodes
    /// as line F on the 68020, the coprocessor's and the FPU's included, one
    /// word long, and takes vector 11.
    #[test]
    fn line_f_is_vector_11_with_no_coprocessor() {
        for first in [0xF200u16, 0xF280, 0xF300, 0xF340, 0xF048, 0xF27C, 0xF600] {
            let decoded =
                decode_words(&[first, 0x4800, 0, 0, 0, 0], 0x1000, Processor::M68020).unwrap();
            assert_eq!(
                decoded.instruction,
                Instruction::LINEF(first),
                "{first:04X}"
            );
            assert_eq!(decoded.size, 2, "{first:04X}");
        }
        //`fmove.x fp0,fp1`, which a 68881 would run
        let (_, frame) = frame_of("", "dc.w $F200,$0080", 11, 4);
        assert_eq!(frame[3], 0x002C);
    }

    /// A trace exception pushes format `$2` too, with the address of the
    /// instruction traced.
    #[test]
    fn trace_pushes_format_2() {
        let mut interpreter = m68020(
            "    move.l #trace,$24
    ori #$8000,sr
traced:
    nop
after:
    simhalt
trace:
    move.l a7,a6
    andi #$7FFF,(a7)
    rte",
        );
        run_to_the_halt(&mut interpreter);
        let frame = a(&interpreter, 6);
        assert_eq!(word(&interpreter, frame + 6), 0x2024);
        assert_eq!(long(&interpreter, frame + 2), label(&interpreter, "after"));
        assert_eq!(long(&interpreter, frame + 8), label(&interpreter, "traced"));
    }

    /// A bus error on a data access is in the middle of an instruction, and
    /// pushes the long bus fault frame, 46 words: the instruction's address,
    /// the special status word and the fault address, the rest zeros. `rte`
    /// runs the instruction again, which reads where the handler moved a0.
    #[test]
    fn a_data_bus_error_pushes_format_b_and_rte_runs_the_instruction_again() {
        let mut interpreter = on(
            "68020\n    memory invalid $9000,$9FFF",
            "    move.l #handler,$8
    lea $FFFF00,a7
    lea $9000,a0
    move.w #$4321,data
faulty:
    move.w (a0),d0
    simhalt
handler:
    move.l a7,a5
    move.l a7,a4
    lea data,a0
    rte
data:
    dc.w 0",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x4321, "run again, from the mended a0");
        let frame = a(&interpreter, 5);
        assert_eq!(frame, 0xFFFF00 - 92, "46 words");
        assert_eq!(long(&interpreter, frame + 2), label(&interpreter, "faulty"));
        assert_eq!(word(&interpreter, frame + 6), 0xB008, "format $B, vector 2");
        assert_eq!(
            word(&interpreter, frame + 0x0A),
            0x0100 | 0x0040 | 0x0020 | 5,
            "a data read of a word in supervisor data space"
        );
        assert_eq!(long(&interpreter, frame + 0x10), 0x9000);
        for offset in (0x14..92).step_by(2) {
            assert_eq!(word(&interpreter, frame + offset), 0, "+${offset:X}");
        }
    }

    /// A write that faults says so, with the size of a long.
    #[test]
    fn a_write_bus_error_says_it_was_a_write() {
        let mut interpreter = on(
            "68020\n    memory read $9000,$9FFF",
            "    move.l #handler,$8
    move.l d0,$9000
    simhalt
handler:
    move.l a7,a5
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        let frame = a(&interpreter, 5);
        assert_eq!(word(&interpreter, frame + 6), 0xB008);
        assert_eq!(
            word(&interpreter, frame + 0x0A),
            0x0100 | 5,
            "a write of a long"
        );
    }

    /// A bus error on the fetch of an instruction, before any of it ran, is
    /// at an instruction boundary, and pushes the short bus fault frame, 16
    /// words, stacking the instruction as the next one to run.
    #[test]
    fn a_fetch_bus_error_pushes_format_a() {
        let mut interpreter = on(
            "68020\n    memory invalid $9000,$9FFF",
            "    move.l #handler,$8
    jmp $9000
handler:
    move.l a7,a5
    move.l 2(a7),d6
    move.l #done,2(a7)
    rte
done:
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        let frame = a(&interpreter, 5);
        assert_eq!(frame, 0x0100_0000 - 32, "16 words");
        assert_eq!(
            d(&interpreter, 6),
            0x9000,
            "the instruction whose fetch faulted"
        );
        assert_eq!(word(&interpreter, frame + 6), 0xA008);
        assert_eq!(
            word(&interpreter, frame + 0x0A),
            0x0040 | 0x0020 | 6,
            "a fetch: a read of a word in supervisor program space"
        );
        assert_eq!(long(&interpreter, frame + 0x10), 0x9000);
        assert_eq!(a(&interpreter, 7), 0x0100_0000, "`rte` pops the 32 bytes");
    }

    /// An instruction fetched from an odd address is still an address error
    /// on the 68020, which pushes the long frame, as WinUAE's does, stacking
    /// the jump that faulted.
    #[test]
    fn a_jump_to_an_odd_address_pushes_format_b() {
        let mut interpreter = m68020(
            "    move.l #handler,$C
    lea $1001,a0
jump:
    jmp (a0)
handler:
    move.l a7,a5
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        let frame = a(&interpreter, 5);
        assert_eq!(long(&interpreter, frame + 2), label(&interpreter, "jump"));
        assert_eq!(word(&interpreter, frame + 6), 0xB00C, "format $B, vector 3");
        assert_eq!(word(&interpreter, frame + 0x0A), 0x0066);
        assert_eq!(long(&interpreter, frame + 0x10), 0x1001);
    }

    /// The frames of the 68020 are pushed on an odd stack pointer, which a
    /// 68000 halts on.
    #[test]
    fn a_frame_on_an_odd_stack_pointer_is_no_double_fault() {
        let body = "    move.l #handler,$80
    lea $5001,a7
    trap #0
    simhalt
handler:
    move.l a7,d0
    rte";
        let mut interpreter = m68020(body);
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x5001 - 8);
        assert_eq!(a(&interpreter, 7), 0x5001);
        let mut interpreter = on("68010", body);
        assert!(interpreter.run().is_err());
        assert!(matches!(
            interpreter.get_termination(),
            Some(Termination::Halted { .. })
        ));
    }
}

mod return_from_exception {
    use super::*;

    /// A frame `rte` takes, written by hand at the stack pointer, 92 bytes
    /// below where it was, kept in a6: format `format`, returning to `back`,
    /// the version number of a format `$B` frame being `version`.
    fn returned(format: u16, version: u16) -> Interpreter {
        let mut interpreter = m68020(&format!(
            "    move.l #formaterror,$38
    move.l a7,a6
    suba.l #92,a7
    move.w #$2004,(a7)
    move.l #back,2(a7)
    move.w #${format:X}000,6(a7)
    move.w #${version:X}000,$36(a7)
    move #$0B,ccr
bad:
    rte
back:
    simhalt
formaterror:
    move.l a7,a5
    simhalt"
        ));
        run_to_the_halt(&mut interpreter);
        interpreter
    }

    /// Formats `$0`, `$2`, `$A` and `$B` pop their 4, 6, 16 and 46 words and
    /// return to the program counter they stack.
    #[test]
    fn rte_pops_the_68020s_formats() {
        for (format, words) in [(0x0u16, 4u32), (0x2, 6), (0xA, 16), (0xB, 46)] {
            let interpreter = returned(format, 0);
            assert_eq!(
                interpreter.get_pc() as u32,
                label(&interpreter, "back") + 4,
                "format ${format:X}: past the `simhalt` at `back`"
            );
            assert_eq!(
                a(&interpreter, 7),
                a(&interpreter, 6) - 92 + words * 2,
                "format ${format:X} pops {words} words"
            );
            assert_eq!(interpreter.get_sr(), 0x2004);
        }
    }

    /// A frame of another format, the 68010's `$8` among them, or a long bus
    /// fault frame of a version s68k does not write, is a format error: the
    /// frame stays, the format error's own stacks the `rte`, and the flags
    /// are left as they were, as the 68020 leaves them.
    #[test]
    fn rte_of_another_format_is_a_format_error() {
        for (format, version) in [(0x8u16, 0u16), (0x3, 0), (0x9, 0), (0xC, 0), (0xB, 1)] {
            let interpreter = returned(format, version);
            let frame = a(&interpreter, 5);
            assert_eq!(
                frame,
                a(&interpreter, 6) - 92 - 8,
                "${format:X}: over the frame"
            );
            assert_eq!(long(&interpreter, frame + 2), label(&interpreter, "bad"));
            assert_eq!(word(&interpreter, frame + 6), 0x0038);
            assert_eq!(
                word(&interpreter, frame) & 0x1F,
                0x0B,
                "${format:X}: the flags"
            );
        }
    }

    /// An `rte` through a throwaway frame a program built by hand swaps the
    /// stack pointers twice, to the master stack its status register names
    /// and then to the user stack of the frame there, and the history says
    /// which each swap made A7, in order, as a host narrates it.
    #[test]
    fn the_history_says_which_stack_each_swap_of_an_rte_made_a7() {
        let mut interpreter = m68020(
            "    move.l #$8000-8,d0
    movec d0,msp
    move.w #$0000,$8000-8
    move.l #back,$8000-6
    move.w #$0000,$8000-2
    move.w #$1068,-(a7)
    pea back
    move.w #$3000,-(a7)
    rte
back:
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(interpreter.get_sr(), 0x0000, "user mode");
        let rte = interpreter.get_last_steps(2)[1].clone();
        let swaps: Vec<StackPointer> = rte
            .get_mutations()
            .iter()
            .filter_map(|mutation| match mutation {
                MutationOperation::WriteStackPointers { a7, .. } => Some(*a7),
                _ => None,
            })
            .collect();
        assert_eq!(swaps, [StackPointer::Master, StackPointer::User]);
        let json = serde_json::to_value(rte.get_mutations()).unwrap();
        let named: Vec<&serde_json::Value> = json
            .as_array()
            .unwrap()
            .iter()
            .filter(|mutation| mutation["type"] == "WriteStackPointers")
            .map(|mutation| &mutation["value"]["a7"])
            .collect();
        assert_eq!(named, ["msp", "usp"]);
    }
}

mod interrupts {
    use super::*;

    /// An interrupt in the master state pushes its format `$0` frame on the
    /// master stack, clears M and pushes the throwaway frame on the interrupt
    /// stack, where the handler runs; `rte` pops the throwaway frame, goes
    /// back to the master stack with its status register, and pops the frame
    /// there. Undo takes each step back.
    #[test]
    fn an_interrupt_in_the_master_state_pushes_a_throwaway_frame() {
        for (sr, back_on) in [(0x3000u16, 0x8000u32), (0x1000, 0x00FF_0000)] {
            let mut interpreter = m68020(&format!(
                "    move.l #irq,$68
    move.l #$8000,d0
    movec d0,msp
    move.w #${sr:04X},sr
spin:
    move.l a7,d2
    simhalt
irq:
    move.l a7,a3
    movec msp,a4
    move sr,d4
    rte"
            ));
            for _ in 0..4 {
                interpreter.step().unwrap();
            }
            let spin = label(&interpreter, "spin");
            interpreter.raise_irq(2).unwrap();
            let before = snapshot(&interpreter);
            interpreter.step().unwrap();
            let after = snapshot(&interpreter);
            interpreter.undo().unwrap();
            assert_eq!(snapshot(&interpreter), before, "undo of the IRQ");
            interpreter.raise_irq(2).unwrap();
            interpreter.step().unwrap();
            assert_eq!(snapshot(&interpreter), after);
            //the master frame, and the throwaway frame with S and M set
            assert_eq!(word(&interpreter, 0x7FF8), sr);
            assert_eq!(long(&interpreter, 0x7FFA), spin);
            assert_eq!(word(&interpreter, 0x7FFE), 0x0068);
            assert_eq!(word(&interpreter, 0x00FF_FFF8), sr | 0x2000);
            assert_eq!(long(&interpreter, 0x00FF_FFFA), spin);
            assert_eq!(word(&interpreter, 0x00FF_FFFE), 0x1068);
            for _ in 0..3 {
                interpreter.step().unwrap();
            }
            assert_eq!(a(&interpreter, 3), 0x00FF_FFF8, "the handler is on the ISP");
            assert_eq!(a(&interpreter, 4), 0x7FF8);
            assert_eq!(d(&interpreter, 4), 0x2200, "M clear, the mask at the level");
            let before = snapshot(&interpreter);
            interpreter.step().unwrap();
            let after = snapshot(&interpreter);
            interpreter.undo().unwrap();
            assert_eq!(snapshot(&interpreter), before, "undo of the `rte`");
            interpreter.step().unwrap();
            assert_eq!(snapshot(&interpreter), after);
            run_to_the_halt(&mut interpreter);
            assert_eq!(d(&interpreter, 2), back_on, "${sr:04X}");
            assert_eq!(interpreter.get_sr(), sr);
            let system = interpreter.get_system();
            assert_eq!((system.ssp, system.msp), (0x0100_0000, Some(0x8000)));
            assert!(interpreter.get_pretty_call_stack().is_empty());
        }
    }

    /// With M clear, an interrupt pushes one frame, on the interrupt stack.
    #[test]
    fn an_interrupt_in_the_interrupt_state_pushes_one_frame() {
        let mut interpreter = m68020(
            "    move.l #irq,$68
spin:
    simhalt
irq:
    move.l a7,a3
    rte",
        );
        interpreter.step().unwrap();
        interpreter.raise_irq(2).unwrap();
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        assert_eq!(a(&interpreter, 3), 0x0100_0000 - 8);
        assert_eq!(word(&interpreter, 0x0100_0000 - 2), 0x0068);
    }
}

mod tracing {
    use super::*;

    /// T0 traces the instructions that change the flow of the program: a
    /// branch taken, `bra` and `bsr`, a jump, a return and a write of the
    /// status register, the condition codes' `andi` among them. Not one that
    /// runs on, a branch not taken, `move` to CCR, nor an instruction that
    /// raises an exception, which clears T0 as it is taken.
    #[test]
    fn t0_traces_the_changes_of_flow() {
        let mut interpreter = m68020(
            "    move.l #trace,$24
    move.l #trap,$80
    lea log,a5
    ori #$4000,sr
    nop
    moveq #1,d0
c:  bra.s d
    nop
d:  cmp.w #1,d0
e:  beq.s f
    nop
f:  bne.s g
    nop
g:  moveq #1,d1
h:  dbra d1,h
i:  bsr sub
    move #$04,ccr
k:  andi #$FF,ccr
    trap #0
m:  move #$6000,sr
n:  andi #$BFFF,sr
    bra.s p
    nop
p:  move.l a5,d7
    simhalt
sub:
    nop
r:  rts
trap:
    rte
trace:
    move.l 8(a7),(a5)+
    rte
log:
    ds.l 16",
        );
        run_to_the_halt(&mut interpreter);
        let log = label(&interpreter, "log");
        let traced: Vec<u32> = (log..d(&interpreter, 7))
            .step_by(4)
            .map(|at| long(&interpreter, at))
            .collect();
        let expected: Vec<u32> = ["c", "e", "h", "i", "r", "k", "m", "n"]
            .iter()
            .map(|name| label(&interpreter, name))
            .collect();
        assert_eq!(traced, expected);
    }

    /// T1 traces every instruction, as the 68000's T does, and a traced
    /// `stop` does not stop; with T0 alone `stop` is no change of flow, and
    /// stops.
    #[test]
    fn t1_traces_everything_and_t0_lets_stop_stop() {
        let mut interpreter = m68020(
            "    move.l #trace,$24
    moveq #0,d7
    ori #$8000,sr
    nop
    nop
    stop #$A000
    simhalt
trace:
    addq.l #1,d7
    rte",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 7), 3, "two `nop`s and the `stop`");
        let mut interpreter = m68020(
            "    move.l #trace,$24
    ori #$4000,sr
    stop #$6000
    simhalt
trace:
    rte",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
    }

    /// The 68000 has no T0: it reads as 0, and traces nothing.
    #[test]
    fn the_68000_has_no_t0() {
        let mut interpreter = on(
            "68000",
            "    ori #$4000,sr\n    bra.s next\n    nop\nnext:\n    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(interpreter.get_sr(), 0x2000);
    }
}

mod unaligned {
    use super::*;

    /// A 68020 reads and writes words and longs at odd addresses, through
    /// every mode and the stack, with `movem`, `cmpm` and an operand relative
    /// to the program counter; undo takes it all back.
    #[test]
    fn words_and_longs_at_odd_addresses() {
        let body = "    lea data+1,a0
    move.l #$11223344,(a0)
    move.w (a0),d0
    move.l (a0)+,d1
    move.w #$5566,-(a0)
    addq.l #1,(a0)
    movem.l d0-d1,data+9
    movem.l data+9,d2-d3
    lea data+1,a1
    lea data+9,a2
    cmpm.w (a1)+,(a2)+
    move.w odd(pc),d4
    move.l a7,a6
    lea $8001,a7
    move.l d1,-(a7)
    bsr sub
back:
    move.l (a7)+,d5
    move.l a6,a7
    simhalt
sub:
    move.l (a7),d6
    rts
data:
    ds.b 16
    dc.b 0
odd:
    dc.b $AB,$CD";
        let mut interpreter = m68020(body);
        run_to_the_halt(&mut interpreter);
        let data = label(&interpreter, "data");
        assert_eq!(d(&interpreter, 0), 0x1122);
        assert_eq!(d(&interpreter, 1), 0x1122_3344);
        assert_eq!(bytes(&interpreter, data + 1, 4), [0x11, 0x22, 0x55, 0x67]);
        assert_eq!(
            (d(&interpreter, 2), d(&interpreter, 3)),
            (0x1122, 0x1122_3344)
        );
        assert_eq!(
            bytes(&interpreter, data + 9, 8),
            [0, 0, 0x11, 0x22, 0x11, 0x22, 0x33, 0x44]
        );
        assert_eq!(a(&interpreter, 1), data + 3);
        assert_eq!(d(&interpreter, 4), 0xABCD);
        assert_eq!(
            d(&interpreter, 5),
            0x1122_3344,
            "pushed and popped at an odd A7"
        );
        assert_eq!(
            d(&interpreter, 6),
            label(&interpreter, "back"),
            "bsr's return"
        );
        undo_restores_every_step(m68020(body));
        //the 68000 and the 68010 take the address error
        for processor in ["68000", "68010"] {
            let mut interpreter = on(processor, body);
            assert!(matches!(
                interpreter.run(),
                Err(RuntimeError::AddressError { .. })
            ));
        }
    }

    /// An odd access goes round the end of the 24-bit bus: a word at
    /// `$FFFFFF` and a long at `$FFFFFD`.
    #[test]
    fn an_odd_access_wraps_round_the_bus() {
        let mut interpreter = m68020(
            "    move.l #$AABBCCDD,$FFFFFD
    move.w #$1122,$FFFFFF
    move.l $FFFFFD,d0
    move.w $FFFFFF,d1
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(bytes(&interpreter, 0xFFFFFD, 4), [0xAA, 0xBB, 0x11, 0x22]);
        assert_eq!(d(&interpreter, 0), 0xAABB_1122);
        assert_eq!(d(&interpreter, 1), 0x1122);
    }

    /// The memory map checks every byte of an odd write and the first of an
    /// odd read, as it does of an even one.
    #[test]
    fn the_memory_map_checks_an_odd_access() {
        let mut interpreter = on(
            "68020\n    memory read $2000,$2FFF\n    memory rom $3000,$3FFF",
            "    move.l $1FFF,d0
    move.l #0,$3FFF
    move.l d0,$1FFF
    simhalt",
        );
        for _ in 0..2 {
            assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        }
        assert_eq!(
            bytes(&interpreter, 0x3FFF, 4),
            [0xFF; 4],
            "ROM ignores it whole"
        );
        assert!(matches!(
            interpreter.step(),
            Err(RuntimeError::BusError {
                address: 0x1FFF,
                ..
            })
        ));
    }

    /// An odd write into code clears the instructions it covers, and the new
    /// bytes run; a watched range records an odd store; a trap task reads
    /// into an odd buffer.
    #[test]
    fn code_watched_memory_and_trap_buffers() {
        let mut interpreter = m68020(
            "    move.l #$AA70054E,pad+1
    bra.s next
pad:
    dc.w 0
next:
    moveq #1,d0
    nop
    simhalt",
        );
        interpreter.step().unwrap();
        let next = label(&interpreter, "next") as usize;
        assert_eq!(
            interpreter.get_code_write(),
            Some(CodeWrite {
                address: next,
                instruction_index: Some(2),
            })
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 5);

        let mut interpreter = m68020("    move.l #$11223344,$5001\n    simhalt");
        interpreter.watch_memory(0x5000, 0x5004).unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::WatchedWrite));
        assert_eq!(
            interpreter.take_watched_writes(),
            [WatchedWrite {
                address: 0x5001,
                size: 4,
                value: 0x1122_3344
            }]
        );

        let mut interpreter = m68020(
            "    move.l #53,d0\n    moveq #1,d1\n    lea $5001,a1\n    moveq #3,d2\n    \
             trap #15\n    simhalt",
        );
        for _ in 0..4 {
            interpreter.step().unwrap();
        }
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Interrupt));
        interpreter
            .answer_interrupt(InterruptResult::ReadFile(Some(Bytes(vec![7, 8, 9]))))
            .unwrap();
        assert_eq!(bytes(&interpreter, 0x5001, 3), [7, 8, 9]);
    }
}
