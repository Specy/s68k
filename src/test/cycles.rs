//! The cycle counter (the asm-editor's `docs/design/m68k-machine.md`,
//! decision 23): what each step costs, what the counter adds up, tasks 30 and
//! 31, and undo. The timing of each instruction form is checked against the
//! conformance suite's lengths (`src/conformance`); these are the cases the
//! suite does not reach, or the counter's own behaviour.

use crate::assembler::program::Program;
use crate::debugger::ExecutionStepKind;
use crate::instructions::{RegisterOperand, Size};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus};

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
                .map(|diagnostic| diagnostic.message())
                .collect::<Vec<_>>()
        ),
    }
}

fn machine(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(&format!("    org $1000\nstart:\n{code}\n")),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 10_000,
        }),
    )
}

/// The cycles of each of the next `count` steps, from the history, checking
/// that the counter adds them up.
fn costs(interpreter: &mut Interpreter, count: usize) -> Vec<u32> {
    let mut costs = Vec::new();
    for _ in 0..count {
        let before = interpreter.get_cycles();
        interpreter.step().unwrap();
        let step = interpreter.get_last_steps(1)[0];
        assert_eq!(
            step.get_old_cycles(),
            before,
            "the step journals the counter before it"
        );
        assert_eq!(
            interpreter.get_cycles(),
            before + step.get_cycles() as u64,
            "the counter adds the step's cycles"
        );
        costs.push(step.get_cycles());
    }
    costs
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

#[test]
fn each_step_records_its_cycles_and_the_counter_adds_them_up() {
    let mut interpreter = machine(
        "    moveq #7,d0
    move.w d0,d1
    add.l d0,d1
    exg d0,d1
    swap d0
    nop
    movea.l #$2000,a0
    move.w d0,(a0)
    move.l (a0)+,d2",
    );
    assert_eq!(interpreter.get_cycles(), 0, "nothing has run");
    assert_eq!(costs(&mut interpreter, 9), [4, 4, 8, 6, 4, 4, 12, 8, 12]);
    assert_eq!(interpreter.get_cycles(), 62);
}

#[test]
fn a_branch_costs_by_whether_it_is_taken() {
    let mut interpreter = machine(
        "    moveq #0,d0
    beq.s s_taken
    nop
s_taken:
    bne.s s_taken
    beq.w w_taken
    nop
w_taken:
    bne.w w_taken
    moveq #1,d1
loop:
    dbra d1,loop
    moveq #0,d2
    dbeq d1,loop
    bra.s next
    nop
next:
    bsr.w sub
    nop
sub:
    rts",
    );
    let branches = costs(&mut interpreter, 13);
    assert_eq!(
        branches,
        //moveq, beq.s and bne.s taken and not, beq.w and bne.w taken and not, moveq, dbra back
        //and run out, moveq, dbeq with its condition true, bra, bsr, rts
        [4, 10, 8, 10, 12, 4, 10, 14, 4, 12, 10, 18, 16]
    );
}

#[test]
fn a_shift_takes_two_cycles_a_bit_of_its_count() {
    let mut interpreter = machine(
        "    moveq #65,d1
    lsl.l #8,d0
    asr.b #1,d0
    lsl.w d1,d0
    rol.l d1,d0
    movea.l #$2000,a0
    lsl.w (a0)",
    );
    //a register count is taken mod 64, so 65 shifts by one
    assert_eq!(
        costs(&mut interpreter, 7),
        [4, 8 + 16, 6 + 2, 6 + 2, 8 + 2, 12, 8 + 4]
    );
}

#[test]
fn the_data_of_a_multiply_a_divide_a_bit_and_an_scc_counts() {
    let mut interpreter = machine(
        "    move.l #$FFFF,d1
    mulu d1,d0
    muls d1,d0
    move.l #$10000,d0
    divu #1,d0
    move.l #$52C8434E,d0
    divu #$64B2,d0
    moveq #20,d2
    bset d2,d3
    bclr #3,d3
    seq d4
    sne d4",
    );
    let found = costs(&mut interpreter, 12);
    assert_eq!(
        found,
        [
            12,
            38 + 32,
            38 + 2,
            12,
            10 + 4,
            12,
            120 + 4,
            4,
            6 + 2,
            12,
            //bit 3 of d3 was clear, so Z is set: seq is true and sne false
            6,
            4
        ]
    );
}

#[test]
fn an_exception_costs_what_the_68000_takes() {
    let mut interpreter = machine(
        "    move.l #handler,$80
    move.l #skip,$10
    move.l #handler,$14
    move.l #handler,$18
    move.l #handler,$24
    move.l #group0,$0C
    trap #0
    illegal
    divu #0,d0
    moveq #-1,d0
    chk #5,d0
    moveq #9,d0
    chk #5,d0
    movea.l #$2001,a0
    tst.w (a0)
    ori #$8000,sr
    nop
    simhalt
handler:
    rte
skip:
    addq.l #2,2(a7)
    rte
group0:
    addq.l #8,a7
    rte",
    );
    costs(&mut interpreter, 6);
    let rte = 20;
    let found = costs(&mut interpreter, 20);
    assert_eq!(
        found,
        [
            //trap #0, and illegal with the handler that steps over it
            34,
            rte,
            34,
            12 + 12,
            rte,
            //a division by zero, and chk below zero and above the bound
            38 + 4,
            rte,
            4,
            40 + 4,
            rte,
            4,
            38 + 4,
            rte,
            //an address error at the first access, and the handler that drops its frame
            12,
            58,
            8,
            rte,
            //traced: the nop, then the trace exception
            20,
            4 + 34,
            rte
        ]
    );
}

#[test]
fn a_privilege_violation_costs_34_and_stop_and_reset_their_own() {
    let mut interpreter = machine(
        "    move.l #handler,$20
    reset
    andi #$DFFF,sr
    reset
    simhalt
handler:
    stop #$2000",
    );
    costs(&mut interpreter, 1);
    assert_eq!(costs(&mut interpreter, 3), [132, 20, 34]);
    assert_eq!(costs(&mut interpreter, 1), [4], "stop, which waits");
    assert_eq!(interpreter.get_status(), &InterpreterStatus::Stopped);
}

#[test]
fn trap_15_tasks_cost_nothing() {
    let mut interpreter = machine(
        "    moveq #12,d0
    moveq #1,d1
    trap #15
    moveq #31,d0
    trap #15",
    );
    assert_eq!(costs(&mut interpreter, 5), [4, 4, 0, 4, 0]);
}

#[test]
fn task_30_clears_the_counter_and_task_31_reads_it() {
    let mut interpreter = machine(
        "    moveq #31,d0
    trap #15
    move.l d1,d2
    moveq #30,d0
    trap #15
    moveq #31,d0
    trap #15",
    );
    assert_eq!(costs(&mut interpreter, 2), [4, 0]);
    assert_eq!(d(&interpreter, 1), 4, "the moveq before it");
    assert_eq!(costs(&mut interpreter, 2), [4, 4]);
    interpreter.step().unwrap();
    let cleared = interpreter.get_last_steps(1)[0];
    assert_eq!(cleared.get_old_cycles(), 12);
    assert_eq!(
        cleared.get_cycles(),
        0,
        "the step that clears costs nothing"
    );
    assert_eq!(interpreter.get_cycles(), 0);
    costs(&mut interpreter, 2);
    assert_eq!(d(&interpreter, 1), 4);
    interpreter.undo().unwrap();
    interpreter.undo().unwrap();
    interpreter.undo().unwrap();
    assert_eq!(
        interpreter.get_cycles(),
        12,
        "undo puts back what task 30 cleared"
    );
}

#[test]
fn a_step_that_clears_the_counter_costs_what_it_took_after_the_clear() {
    let mut interpreter = machine(
        "    move.l #handler,$24
    moveq #30,d0
    ori #$8000,sr
    trap #15
    simhalt
handler:
    rte",
    );
    costs(&mut interpreter, 3);
    interpreter.step().unwrap();
    let step = interpreter.get_last_steps(1)[0];
    assert_eq!(step.get_cycles(), 34, "the trace exception after the clear");
    assert_eq!(interpreter.get_cycles(), 34);
}

#[test]
fn task_31_reads_0_once_the_counter_no_longer_fits_in_32_bits() {
    let mut interpreter = machine(
        "    moveq #31,d0
    trap #15
    moveq #31,d0
    trap #15",
    );
    interpreter.preset_cycles(0xFFFF_FFF8);
    costs(&mut interpreter, 2);
    assert_eq!(d(&interpreter, 1), 0xFFFF_FFFC, "4 cycles below the top");
    costs(&mut interpreter, 2);
    assert_eq!(interpreter.get_cycles(), 0x1_0000_0000);
    assert_eq!(d(&interpreter, 1), 0, "above 32 bits");
}

#[test]
fn undo_takes_a_steps_cycles_back() {
    let mut interpreter = machine(
        "    moveq #1,d0
    mulu d0,d0
    nop",
    );
    costs(&mut interpreter, 3);
    assert_eq!(interpreter.get_cycles(), 4 + 40 + 4);
    interpreter.undo().unwrap();
    assert_eq!(interpreter.get_cycles(), 44);
    interpreter.undo().unwrap();
    assert_eq!(interpreter.get_cycles(), 4);
    interpreter.step().unwrap();
    assert_eq!(
        interpreter.get_cycles(),
        44,
        "and running it again counts it again"
    );
}

#[test]
fn a_poke_costs_nothing() {
    let mut interpreter = machine("    nop\n    nop");
    costs(&mut interpreter, 1);
    interpreter.begin_poke().unwrap();
    interpreter.set_register_value(RegisterOperand::Data(0), 5, Size::Long);
    assert!(interpreter.end_poke().unwrap());
    let poke = interpreter.get_last_steps(1)[0];
    assert_eq!(poke.get_kind(), ExecutionStepKind::Poke);
    assert_eq!(poke.get_cycles(), 0);
    assert_eq!(poke.get_old_cycles(), 4);
    interpreter.undo().unwrap();
    assert_eq!(interpreter.get_cycles(), 4);
}

#[test]
fn an_irq_step_takes_44() {
    let mut interpreter = machine(
        "    move.l #handler,$68
    andi #$F8FF,sr
    nop
    simhalt
handler:
    rte",
    );
    costs(&mut interpreter, 2);
    interpreter.raise_irq(2).unwrap();
    interpreter.step().unwrap();
    let irq = interpreter.get_last_steps(1)[0];
    assert_eq!(irq.get_kind(), ExecutionStepKind::Irq);
    assert_eq!(irq.get_cycles(), 44);
    let before = irq.get_old_cycles();
    assert_eq!(interpreter.get_cycles(), before + 44);
    interpreter.undo().unwrap();
    assert_eq!(
        interpreter.get_cycles(),
        before,
        "undo takes the IRQ's cycles back"
    );
}
