//! Interrupt requests, `stop` and `reset` (the asm-editor's
//! `docs/design/m68k-machine.md`, decisions 13 and 16 to 22).
//!
//! Every program here installs a handler on each of the seven autovectors
//! that writes its level to a log and returns, and most of them pause with
//! `simhalt` where the test raises its requests, as the host raises them
//! between two calls. The log is what the tests read, in the order the
//! handlers ran. Undo is checked across IRQ steps and `stop` against a
//! snapshot of the machine, its interrupt state included.

use crate::assembler::program::Program;
use crate::assembler::source::DEFAULT_ENTRY_PATH;
use crate::debugger::{ExecutionStepKind, FrameKind, MutationOperation};
use crate::instructions::{InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    Breakpoint, Interpreter, InterpreterOptions, InterpreterStatus, RunOptions, RuntimeError,
    Termination,
};
use crate::machine::interrupts::{AutoIrq, DeviceEvent, MachineState};

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

/// A program whose `body` runs after the seven autovectors are installed and
/// A0 points at the log, with the handlers and the log after it. Each
/// handler writes its level to the log and returns; `handlers` replaces the
/// one of the level it names.
fn program(body: &str, handlers: &[(u8, &str)]) -> String {
    let mut code = String::from("    org $1000\nstart:\n");
    for level in 1..=7u8 {
        code += &format!("    move.l #irq{level},${:X}\n", 0x60 + 4 * level as u32);
    }
    code += "    lea log,a0\n";
    code += body;
    code += "\n";
    for level in 1..=7u8 {
        match handlers.iter().find(|(handled, _)| *handled == level) {
            Some((_, handler)) => code += &format!("irq{level}:\n{handler}\n"),
            None => code += &format!("irq{level}:\n    move.b #{level},(a0)+\n    rte\n"),
        }
    }
    code += "log:\n    ds.b 32\n";
    code
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn label(interpreter: &Interpreter, name: &str) -> usize {
    interpreter
        .get_program()
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no label `{name}`"))
        .value as usize
}

/// What the handlers wrote, in the order they ran.
fn log(interpreter: &Interpreter) -> Vec<u8> {
    let start = label(interpreter, "log");
    let end = interpreter.get_cpu().get_register_values()[8] as usize;
    interpreter
        .get_memory()
        .read_bytes(start, end - start)
        .unwrap()
        .to_vec()
}

fn pending(interpreter: &Interpreter) -> Vec<u8> {
    interpreter.get_machine_state().pending_levels()
}

/// Runs to the next `simhalt`.
fn run_to_the_pause(interpreter: &mut Interpreter) {
    assert_eq!(
        interpreter.run(),
        Ok(InterpreterStatus::Paused),
        "{:?}",
        interpreter.get_termination()
    );
}

fn last_step_kind(interpreter: &Interpreter) -> ExecutionStepKind {
    interpreter.get_last_steps(1)[0].get_kind()
}

/// The 0-based line of `code` that holds `text`, for a breakpoint.
fn line_of(code: &str, text: &str) -> usize {
    code.lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no line holds `{text}`"))
}

/// Everything the programs here change, their interrupt state included.
#[derive(Debug, PartialEq)]
struct Snapshot {
    registers: Vec<u32>,
    usp: u32,
    ssp: u32,
    sr: u16,
    pc: usize,
    status: InterpreterStatus,
    termination: Option<Termination>,
    machine: MachineState,
    call_stack: Vec<(usize, usize, FrameKind, Option<u8>)>,
    memory: Vec<u8>,
}

fn snapshot(interpreter: &Interpreter) -> Snapshot {
    let memory = interpreter.get_memory();
    let mut bytes = Vec::new();
    for (start, end) in [(0x0, 0x400), (0x1000, 0x1400), (0xFFFF00, 0x1000000)] {
        bytes.extend_from_slice(memory.read_bytes(start, end - start).unwrap());
    }
    Snapshot {
        registers: interpreter.get_cpu().get_register_values(),
        usp: interpreter.get_usp(),
        ssp: interpreter.get_ssp(),
        sr: interpreter.get_sr(),
        pc: interpreter.get_pc(),
        status: *interpreter.get_status(),
        termination: interpreter.get_termination().cloned(),
        machine: interpreter.get_machine_state(),
        call_stack: interpreter
            .get_pretty_call_stack()
            .iter()
            .map(|frame| (frame.address, frame.source_address, frame.kind, frame.vector))
            .collect(),
        memory: bytes,
    }
}

/// Steps `interpreter` until it stops running, a snapshot before every step,
/// then undoes every step and checks that each undo puts back the snapshot
/// taken before the step it undid. Answers the kinds of the steps it took.
fn undo_restores_every_step(mut interpreter: Interpreter) -> Vec<ExecutionStepKind> {
    let mut snapshots = vec![snapshot(&interpreter)];
    let mut kinds = Vec::new();
    loop {
        let id = interpreter.get_last_step_id();
        let stepped = interpreter.step();
        if interpreter.get_last_step_id() == id {
            //stopped, with nothing to wake it
            break;
        }
        snapshots.push(snapshot(&interpreter));
        kinds.push(last_step_kind(&interpreter));
        match stepped {
            Ok(InterpreterStatus::Running) | Ok(InterpreterStatus::Stopped) => continue,
            _ => break,
        }
    }
    snapshots.pop();
    while let Some(before) = snapshots.pop() {
        interpreter.undo().expect("a step to undo");
        assert_eq!(snapshot(&interpreter), before, "undoing step {}", snapshots.len());
    }
    assert!(!interpreter.can_undo());
    kinds
}

mod taking_a_request {
    use super::*;

    #[test]
    fn the_highest_pending_level_is_taken_first_as_a_step_of_its_own() {
        let mut interpreter = machine(&program("    simhalt\nresume:\n    nop\n    simhalt", &[]));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(2).unwrap();
        interpreter.raise_irq(5).unwrap();
        assert_eq!(pending(&interpreter), [2, 5]);
        let executed = interpreter.get_executed_instructions();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "irq5"));
        assert_eq!(interpreter.get_sr(), 0x2500, "the mask rises to the level");
        assert_eq!(pending(&interpreter), [2]);
        assert_eq!(
            interpreter.get_executed_instructions(),
            executed,
            "an IRQ runs no instruction"
        );
        //the frame: the status register from before, and the instruction it came before
        let sp = interpreter.get_ssp() as usize;
        let memory = interpreter.get_memory();
        assert_eq!(memory.read_word(sp).unwrap(), 0x2000);
        assert_eq!(memory.read_long(sp + 2).unwrap() as usize, label(&interpreter, "resume"));
        let stack = interpreter.get_pretty_call_stack();
        assert_eq!(stack.len(), 1);
        assert_eq!((stack[0].kind, stack[0].vector), (FrameKind::Exception, Some(29)));
        assert_eq!(stack[0].label_name, "irq5");
        let step = interpreter.get_last_steps(1)[0];
        assert_eq!(step.get_pc(), label(&interpreter, "resume"));
        assert!(step.get_location().is_none(), "an IRQ is on no line");
        assert!(step.get_mutations().iter().any(|mutation| matches!(
            mutation,
            MutationOperation::PushException { vector: 29, from, .. } if *from == label(&interpreter, "resume")
        )));
        assert!(step.get_mutations().iter().any(|mutation| matches!(
            mutation,
            MutationOperation::SetMachineState { old, new }
                if old.pending_levels() == [2, 5] && new.pending_levels() == [2]
        )));
        //the handler's `rte` lowers the mask again, and 2 is taken before the `nop`
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [5, 2]);
        assert!(pending(&interpreter).is_empty());
        assert_eq!(interpreter.get_sr(), 0x2000);
        assert!(interpreter.get_pretty_call_stack().is_empty());
    }

    #[test]
    fn a_request_waits_until_the_mask_is_lowered() {
        let mut interpreter = machine(&program(
            "    move.w #$2300,sr
    simhalt
    nop
    move.w #$2000,sr
lowered:
    nop
    simhalt",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(2).unwrap();
        interpreter.raise_irq(3).unwrap();
        interpreter.raise_irq(4).unwrap();
        //4 is above the mask of 3; 3 is not, and neither is 2
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [4, 3, 2]);
        //3 and 2 were taken once `move #$2000,sr` lowered the mask, before the `nop` after it
        let irqs: Vec<usize> = interpreter
            .get_last_steps(100)
            .iter()
            .filter(|step| step.get_kind() == ExecutionStepKind::Irq)
            .map(|step| step.get_pc())
            .collect();
        let lowered = label(&interpreter, "lowered");
        assert_eq!(irqs[..2], [lowered, lowered]);
    }

    #[test]
    fn level_7_is_taken_whatever_the_mask_and_the_rest_wait() {
        let mut interpreter = machine(&program(
            "    move.w #$2700,sr\n    simhalt\n    nop\n    simhalt",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(6).unwrap();
        interpreter.raise_irq(7).unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [7]);
        assert_eq!(pending(&interpreter), [6]);
        assert_eq!(interpreter.get_sr(), 0x2700);
    }

    #[test]
    fn a_higher_level_interrupts_a_handler_and_a_lower_one_waits_for_its_rte() {
        let mut interpreter = machine(&program(
            "    simhalt\n    nop\n    simhalt",
            &[(2, "    move.b #2,(a0)+\n    simhalt\n    move.b #$12,(a0)+\n    rte")],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(2).unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [2], "paused inside the handler");
        interpreter.raise_irq(1).unwrap();
        interpreter.raise_irq(5).unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        let vectors: Vec<Option<u8>> = interpreter
            .get_pretty_call_stack()
            .iter()
            .map(|frame| frame.vector)
            .collect();
        assert_eq!(vectors, [Some(26), Some(29)], "5 inside 2");
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [2, 5, 0x12, 1]);
    }

    #[test]
    fn a_request_latched_twice_is_taken_once() {
        let mut interpreter = machine(&program("    simhalt\n    nop\n    simhalt", &[]));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(3).unwrap();
        interpreter.raise_irq(3).unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [3]);
    }

    #[test]
    fn an_autovector_with_no_handler_ends_the_program_with_the_request() {
        let code = "    org $1000\nstart:\n    simhalt\n    nop\n    simhalt\n";
        let mut interpreter = machine(code);
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(4).unwrap();
        let before = snapshot(&interpreter);
        assert_eq!(interpreter.step(), Err(RuntimeError::Irq { level: 4 }));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::Irq { level: 4 },
                vector: Some(28),
            })
        );
        assert_eq!(pending(&interpreter), [4], "it was not taken");
        assert_eq!(interpreter.get_ssp(), 0x0100_0000, "and pushed nothing");
        interpreter.undo().unwrap();
        assert_eq!(snapshot(&interpreter), before);
    }

    #[test]
    fn a_request_is_one_level_from_1_to_7_and_none_is_taken_after_the_end() {
        let mut interpreter = machine("    org $1000\nstart:\n    nop\n");
        for level in [0, 8, 255] {
            assert!(matches!(
                interpreter.raise_irq(level),
                Err(RuntimeError::InvalidArgument(_))
            ));
        }
        assert!(pending(&interpreter).is_empty());
        interpreter.run().unwrap();
        assert!(interpreter.has_terminated());
        interpreter.raise_irq(3).unwrap();
        assert!(pending(&interpreter).is_empty());
    }

    /// The limit counts instructions, as `get_executed_instructions` does: an
    /// IRQ is a step of the history and no instruction, so a run limited to
    /// three takes it and runs three.
    #[test]
    fn a_limited_run_counts_instructions_and_not_the_irqs_taken() {
        let mut interpreter = machine(&program("    simhalt\n    nop\n    nop\n    simhalt", &[]));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(1).unwrap();
        let executed = interpreter.get_executed_instructions();
        let steps = interpreter.get_last_step_id();
        //the IRQ, then the handler's `move.b` and `rte`, and one `nop`
        assert_eq!(
            interpreter.run_with_limit(3),
            Err(RuntimeError::ExecutionLimit(3))
        );
        assert_eq!(interpreter.get_executed_instructions(), executed + 3);
        assert_eq!(interpreter.get_last_step_id(), steps + 4, "four steps");
        assert_eq!(log(&interpreter), [1]);
        //the same with breakpoints that stop nothing
        let mut interpreter = machine(&program("    simhalt\n    nop\n    nop\n    simhalt", &[]));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(1).unwrap();
        let executed = interpreter.get_executed_instructions();
        assert_eq!(
            interpreter.run_with_breakpoints(&[], Some(3), RunOptions::default()),
            Err(RuntimeError::ExecutionLimit(3))
        );
        assert_eq!(interpreter.get_executed_instructions(), executed + 3);
    }

    /// A host that runs in slices of a budget and measures each by
    /// `get_executed_instructions`, as the editor does, runs exactly its
    /// budget whatever the IRQs: before, each IRQ took one instruction off the
    /// slice it came in.
    #[test]
    fn a_host_running_in_slices_runs_its_budget_whatever_the_irqs() {
        let code = program("loop:\n    addq.l #1,d0\n    bra loop", &[]);
        let mut interpreter = machine(&code);
        let budget = 1_000u64;
        let first = interpreter.get_executed_instructions();
        let mut slices = 0;
        while interpreter.get_executed_instructions() - first < budget {
            //a timer firing between two slices, once the first has installed the handlers
            if slices > 0 {
                interpreter.raise_irq(3).unwrap();
            }
            let remaining = budget - (interpreter.get_executed_instructions() - first);
            let slice = remaining.min(97) as usize;
            assert_eq!(
                interpreter.run_with_limit(slice),
                Err(RuntimeError::ExecutionLimit(slice))
            );
            slices += 1;
        }
        assert_eq!(interpreter.get_executed_instructions() - first, budget);
        assert_eq!(slices, 11, "ten slices of 97 and one of 30");
        assert_eq!(log(&interpreter), [3; 10]);
    }

    /// The program ends where it runs into memory it never placed, but an IRQ
    /// ready at that point is taken first, and its handler returns there.
    #[test]
    fn a_request_ready_at_the_end_of_the_program_is_taken_before_it_ends() {
        //the last instruction placed lowers the mask, and nothing is placed after it
        let code = program("    move.w #$2700,sr\n    simhalt\n    jmp last", &[]).replace(
            "log:\n",
            "log:\n    ds.b 32\nlast:\n    move.w #$2000,sr\nafter:\n",
        );
        let mut interpreter = machine(&code);
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(6).unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        //the last instruction, which leaves the IRQ ready before the end
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(interpreter.get_pc(), label(&interpreter, "after"));
        assert!(!interpreter.has_reached_bottom(), "the IRQ comes first");
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Terminated));
        assert!(matches!(
            interpreter.get_termination(),
            Some(Termination::EndOfProgram { address }) if *address == label(&interpreter, "after")
        ));
        assert_eq!(log(&interpreter), [6]);
    }
}

mod priorities {
    use super::*;

    /// A request that comes while a `trap #15` task waits on the host is
    /// taken after the answer, before the next instruction: EASy68K loses one
    /// taken in the same step as a trap.
    #[test]
    fn a_request_during_a_task_is_taken_after_its_answer() {
        let mut interpreter = machine(&program(
            "    move.b #5,d0\n    trap #15\nread:\n    nop\n    simhalt",
            &[],
        ));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
        interpreter.raise_irq(2).unwrap();
        interpreter.answer_interrupt(InterruptResult::ReadChar('a')).unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
        assert_eq!(interpreter.get_last_steps(1)[0].get_pc(), label(&interpreter, "read"));
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [2]);
        assert_eq!(d(&interpreter, 1) as u8, b'a');
    }

    /// An illegal instruction at the program counter has not been fetched
    /// when the request is taken: it raises its exception after the `rte`,
    /// where EASy68K runs it again.
    #[test]
    fn a_request_comes_before_an_illegal_instruction() {
        let mut interpreter = machine(&program(
            "    move.l #illegal_handler,$10\n    simhalt\n    illegal\n    simhalt
illegal_handler:
    move.b #$44,(a0)+
    addq.l #2,2(a7)
    rte",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(3).unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [3, 0x44]);
    }

    /// An instruction that starts with T set is traced, and an IRQ its write
    /// of the status register made ready is taken next: its frame goes on
    /// top of the trace exception's, and its handler runs first, before the
    /// trace handler's first instruction, as a 68000 orders them.
    #[test]
    fn a_trace_frame_is_pushed_first_and_the_irq_handler_runs_first() {
        let mut interpreter = machine(&program(
            "    move.l #trace,$24
    move.w #$2700,sr
    simhalt
    move.w #$A700,sr
traced:
    move.w #$2000,sr
after:
    nop
    simhalt
trace:
    move.b #9,(a0)+
    rte",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(3).unwrap();
        //`move.w #$A700,sr` sets T, which traces the instruction after it
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(interpreter.get_pc(), label(&interpreter, "traced"));
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(interpreter.get_pc(), label(&interpreter, "trace"), "traced");
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
        let sp = interpreter.get_ssp() as usize;
        assert_eq!(
            interpreter.get_memory().read_long(sp + 2).unwrap() as usize,
            label(&interpreter, "trace"),
            "the IRQ came before the trace handler's first instruction"
        );
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [3, 9]);
    }

    /// T set when the request is taken stays in the stacked status register:
    /// the handler runs untraced, and the instruction after its `rte` is
    /// traced.
    #[test]
    fn the_handler_runs_untraced_and_the_rte_traces_the_next_instruction() {
        let mut interpreter = machine(&program(
            "    move.l #trace,$24
    move.w #$2700,sr
    simhalt
    move.w #$A000,sr
    nop
    move.w #$2000,sr
    simhalt
trace:
    move.b #9,(a0)+
    rte",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(3).unwrap();
        run_to_the_pause(&mut interpreter);
        //the `nop` and the `move.w #$2000,sr` after it are both traced
        assert_eq!(log(&interpreter), [3, 9, 9]);
    }
}

mod stop {
    use super::*;

    #[test]
    fn stop_loads_the_status_register_and_waits_for_a_request() {
        let mut interpreter = machine(&program(
            "    stop #$2000\nwoken:\n    move.b #$AA,(a0)+\n    simhalt",
            &[],
        ));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        assert!(interpreter.get_machine_state().is_stopped());
        assert_eq!(interpreter.get_pc(), label(&interpreter, "woken"));
        assert!(!interpreter.has_terminated());
        //nothing runs while nothing is pending
        let id = interpreter.get_last_step_id();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Stopped));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        assert_eq!(interpreter.run_with_limit(10), Ok(InterpreterStatus::Stopped));
        assert_eq!(
            interpreter.run_with_breakpoints(&[], Some(10), RunOptions::RESUME),
            Ok(InterpreterStatus::Stopped)
        );
        assert_eq!(interpreter.get_last_step_id(), id);
        //a request wakes it: it no longer waits, and the next step takes the request
        interpreter.raise_irq(3).unwrap();
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
        assert!(interpreter.get_machine_state().is_stopped(), "until the IRQ is taken");
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
        assert!(!interpreter.get_machine_state().is_stopped());
        assert_eq!(interpreter.get_pc(), label(&interpreter, "irq3"));
        let sp = interpreter.get_ssp() as usize;
        assert_eq!(
            interpreter.get_memory().read_long(sp + 2).unwrap() as usize,
            label(&interpreter, "woken"),
            "the frame returns past the `stop`"
        );
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [3, 0xAA], "nothing ran before the handler");
    }

    #[test]
    fn a_run_wakes_on_a_request_and_goes_on() {
        let mut interpreter = machine(&program("    stop #$2000\n    move.b #$AA,(a0)+\n    simhalt", &[]));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        interpreter.raise_irq(6).unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [6, 0xAA]);
    }

    #[test]
    fn a_mask_too_high_keeps_the_processor_stopped() {
        let mut interpreter = machine(&program(
            "    stop #$2500\n    move.w #$2000,sr\n    simhalt",
            &[],
        ));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        interpreter.raise_irq(3).unwrap();
        interpreter.raise_irq(5).unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Stopped));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        assert_eq!(pending(&interpreter), [3, 5]);
        interpreter.raise_irq(6).unwrap();
        run_to_the_pause(&mut interpreter);
        //6 woke it; its `rte` put back the mask of 5, and lowering it took 5 and 3
        assert_eq!(log(&interpreter), [6, 5, 3]);
    }

    #[test]
    fn a_request_already_above_the_new_mask_is_taken_at_once() {
        let mut interpreter = machine(&program(
            "    move.w #$2700,sr\n    simhalt\n    stop #$2000\n    move.b #$AA,(a0)+\n    simhalt",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(4).unwrap();
        //a run goes through the `stop`
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [4, 0xAA]);
        //a step stops the processor, which does not wait, and the next takes the request
        let mut stepped = machine(&program(
            "    move.w #$2700,sr\n    simhalt\n    stop #$2000\n    simhalt",
            &[],
        ));
        run_to_the_pause(&mut stepped);
        stepped.raise_irq(4).unwrap();
        assert_eq!(stepped.step(), Ok(InterpreterStatus::Running));
        assert!(stepped.get_machine_state().is_stopped());
        assert_eq!(stepped.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&stepped), ExecutionStepKind::Irq);
        assert!(!stepped.get_machine_state().is_stopped(), "the request woke it");
    }

    /// `stop` loads S as well: with S clear the processor stops in user mode,
    /// where EASy68K raises a privilege violation, and the IRQ that wakes it
    /// runs its handler in supervisor mode on the supervisor stack.
    #[test]
    fn stop_can_stop_in_user_mode() {
        let mut interpreter = machine(&program(
            "    move.l #$8000,a1\n    move.l a1,usp\n    stop #$0000\nback:\n    move.l a7,d2\n    simhalt",
            &[(1, "    move.l a7,d1\n    move.w (a7),d3\n    rte")],
        ));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        assert_eq!(interpreter.get_sr(), 0x0000);
        assert_eq!(interpreter.get_cpu().get_register_values()[15], 0x8000, "A7 is the USP");
        interpreter.raise_irq(1).unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 0x00FF_FFFA, "the frame is on the SSP");
        assert_eq!(d(&interpreter, 3), 0x0000, "and stacks the user mode");
        assert_eq!(d(&interpreter, 2), 0x8000, "back on the user stack");
        assert_eq!(interpreter.get_sr() & 0x2000, 0);
    }

    #[test]
    fn stop_is_privileged() {
        let mut interpreter =
            machine("    org $1000\nstart:\n    move.w #$0000,sr\n    stop #$2000\n    simhalt\n");
        assert_eq!(interpreter.run(), Err(RuntimeError::PrivilegeViolation));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::PrivilegeViolation,
                vector: Some(8),
            })
        );
        assert!(!interpreter.get_machine_state().is_stopped());
    }

    /// A `stop` that starts with T set does not stop: the trace exception
    /// follows it, stacking the status register it loaded and the next
    /// instruction, and the program goes on after the handler's `rte`.
    #[test]
    fn a_traced_stop_takes_the_trace_exception_and_does_not_stop() {
        let mut interpreter = machine(&program(
            "    move.l #trace,$24
    move.w #$A000,sr
    stop #$2300
after:
    move.b #$AA,(a0)+
    simhalt
trace:
    move.w (a7),d1
    move.l 2(a7),d2
    move.b #9,(a0)+
    rte",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [9, 0xAA]);
        assert_eq!(d(&interpreter, 1), 0x2300, "the status register `stop` loaded");
        assert_eq!(d(&interpreter, 2) as usize, label(&interpreter, "after"));
        assert!(!interpreter.get_machine_state().is_stopped());
    }

    #[test]
    fn breakpoints_find_a_stopped_processor_stopped() {
        let code = program("    stop #$2000\n    nop\n    simhalt", &[]);
        let mut interpreter = machine(&code);
        let breakpoints = [Breakpoint::new(DEFAULT_ENTRY_PATH, line_of(&code, "    nop"))];
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, Some(1000), RunOptions::CONTINUE),
            Ok(InterpreterStatus::Stopped)
        );
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, Some(1000), RunOptions::RESUME),
            Ok(InterpreterStatus::Stopped)
        );
        interpreter.raise_irq(2).unwrap();
        //the IRQ, its handler, and the breakpoint after the `stop`
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, Some(1000), RunOptions::RESUME),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(log(&interpreter), [2]);
        assert_eq!(
            interpreter.get_current_location().map(|location| location.line),
            Some(line_of(&code, "    nop"))
        );
    }
}

mod reset {
    use super::*;

    #[test]
    fn reset_changes_nothing_but_the_program_counter() {
        let mut interpreter = machine(
            "    org $1000\nstart:\n    move.l #5,d0\n    move.w #$2704,sr\n    simhalt\n    reset\n    simhalt\n",
        );
        run_to_the_pause(&mut interpreter);
        let before = snapshot(&interpreter);
        run_to_the_pause(&mut interpreter);
        let after = snapshot(&interpreter);
        assert_eq!(after.pc, before.pc + 6, "`reset` and the `simhalt` after it");
        assert_eq!(
            Snapshot { pc: before.pc, ..after },
            before,
            "nothing else moved"
        );
    }

    #[test]
    fn reset_is_privileged() {
        let mut interpreter =
            machine("    org $1000\nstart:\n    move.w #$0000,sr\n    reset\n    simhalt\n");
        assert_eq!(interpreter.run(), Err(RuntimeError::PrivilegeViolation));
    }
}

mod undo {
    use super::*;

    /// Every step of a program that stops and is woken, with requests latched
    /// before it starts, under a mask of 7 until the program has installed
    /// its handlers: undo puts back the latch, the stopped flag and the status
    /// with everything else.
    #[test]
    fn undo_restores_every_step_across_irqs_and_stop() {
        let code = program(
            "    stop #$2300
    move.w #$2000,sr
    nop
    stop #$0000
    simhalt",
            &[(2, "    move.b #2,(a0)+\n    move.w #$2700,sr\n    rte")],
        );
        let mut interpreter = machine(&code);
        interpreter.set_sr(0x2700);
        for level in [2, 5] {
            interpreter.raise_irq(level).unwrap();
        }
        let kinds = undo_restores_every_step(interpreter);
        assert_eq!(
            kinds.iter().filter(|kind| **kind == ExecutionStepKind::Irq).count(),
            2,
            "{kinds:?}"
        );
    }

    #[test]
    fn undoing_an_irq_step_makes_the_request_pending_again() {
        let mut interpreter = machine(&program("    stop #$2000\n    simhalt", &[]));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        let stopped = snapshot(&interpreter);
        interpreter.raise_irq(3).unwrap();
        let raised = snapshot(&interpreter);
        run_to_the_pause(&mut interpreter);
        //the `simhalt`, the `rte`, the `move.b` and the IRQ
        for _ in 0..4 {
            interpreter.undo().unwrap();
        }
        assert_eq!(snapshot(&interpreter), raised);
        assert_eq!(pending(&interpreter), [3]);
        assert!(interpreter.get_machine_state().is_stopped());
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running, "woken, not waiting");
        //undoing the `stop` keeps the request: the program never saw it
        interpreter.undo().unwrap();
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
        assert!(!interpreter.get_machine_state().is_stopped());
        assert_eq!(pending(&interpreter), [3]);
        assert_eq!(
            Snapshot { status: stopped.status, machine: stopped.machine, pc: stopped.pc, ..snapshot(&interpreter) },
            stopped,
            "the rest is as before the `stop`, but the status register it loaded"
        );
        //and running on from there takes it before the `stop`
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert_eq!(last_step_kind(&interpreter), ExecutionStepKind::Irq);
    }

    /// A Poke of the status register that lowers the mask under a pending
    /// request wakes a stopped processor, and undoing it leaves it waiting
    /// again; one that raises the mask over it makes it wait.
    #[test]
    fn a_poke_of_the_mask_wakes_a_stopped_processor_or_leaves_it_waiting() {
        let mut interpreter = machine(&program("    stop #$2700\n    simhalt", &[]));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Stopped));
        interpreter.raise_irq(3).unwrap();
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Stopped, "masked");
        interpreter.begin_poke().unwrap();
        interpreter.set_sr(0x2000);
        assert!(interpreter.end_poke().unwrap());
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
        interpreter.undo().unwrap();
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Stopped);
        assert_eq!(interpreter.get_sr(), 0x2700);
        interpreter.begin_poke().unwrap();
        interpreter.set_sr(0x2000);
        interpreter.end_poke().unwrap();
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [3]);
    }

    #[test]
    fn a_request_that_came_after_a_step_survives_its_undo() {
        let mut interpreter = machine(&program("    move.w #$2700,sr\n    nop\n    simhalt", &[]));
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        interpreter.raise_irq(4).unwrap();
        interpreter.undo().unwrap();
        interpreter.undo().unwrap();
        assert_eq!(pending(&interpreter), [4]);
    }
}

mod tasks {
    use super::*;

    fn run_task(setup: &str) -> Interpreter {
        let mut interpreter =
            machine(&format!("    org $1000\nstart:\n{setup}\n    trap #15\n    simhalt\n"));
        run_to_the_pause(&mut interpreter);
        interpreter
    }

    #[test]
    fn task_32_answers_easy68ks_version() {
        let interpreter = run_task("    move.b #32,d0\n    move.l #4,d1");
        assert_eq!(d(&interpreter, 1), 0x0005_1001);
    }

    #[test]
    fn task_32_sets_the_timers() {
        let interpreter = run_task(
            "    move.b #32,d0\n    move.b #6,d1\n    move.b #$82,d2\n    move.l #500,d3\n    trap #15
    move.b #$85,d2\n    move.l #20,d3\n    trap #15
    move.b #$87,d2\n    move.l #1000,d3\n    trap #15
    move.b #7,d2\n    trap #15
    move.b #$90,d2\n    trap #15",
        );
        assert_eq!(
            interpreter.get_machine_state().auto_irqs(),
            [
                AutoIrq { level: 2, interval_ms: 500 },
                AutoIrq { level: 5, interval_ms: 20 }
            ]
        );
        let interpreter = run_task(
            "    move.b #32,d0\n    move.b #6,d1\n    move.b #$83,d2\n    move.l #9,d3\n    trap #15
    move.b #0,d2\n    trap #15",
        );
        assert!(interpreter.get_machine_state().auto_irqs().is_empty(), "0 turns them all off");
    }

    #[test]
    fn task_32_refuses_the_hardware_window_and_ignores_what_easy68k_ignores() {
        for part in [0, 1, 2, 3, 7] {
            let mut interpreter = machine(&format!(
                "    org $1000\nstart:\n    move.b #32,d0\n    move.b #{part},d1\n    trap #15\n    simhalt\n"
            ));
            assert_eq!(
                interpreter.run(),
                Err(RuntimeError::UnsupportedTrapTask { task: 32 }),
                "D1.B of {part}"
            );
        }
        for part in [8, 0x7F, 0xFF] {
            let interpreter = run_task(&format!("    move.b #32,d0\n    move.b #{part},d1"));
            assert_eq!(d(&interpreter, 1), part, "D1.B of {part} does nothing");
        }
    }

    #[test]
    fn task_60_sets_the_mouse_levels_per_event() {
        let interpreter = run_task(
            "    move.b #60,d0\n    move.w #$0101,d1\n    trap #15
    move.w #$0202,d1\n    trap #15
    move.w #$0304,d1\n    trap #15
    move.w #$0807,d1",
        );
        let mouse = interpreter.get_machine_state().mouse_levels();
        assert_eq!((mouse.down, mouse.up, mouse.moved), (1, 2, 3), "above 7 is ignored");
        let interpreter = run_task(
            "    move.b #60,d0\n    move.w #$0507,d1\n    trap #15\n    move.w #$0005,d1",
        );
        let mouse = interpreter.get_machine_state().mouse_levels();
        assert_eq!((mouse.down, mouse.up, mouse.moved), (0, 5, 0), "level 0 turns them off");
    }

    #[test]
    fn task_62_sets_the_keyboard_levels_per_event() {
        let interpreter = run_task(
            "    move.b #62,d0\n    move.w #$0501,d1\n    trap #15\n    move.w #$0602,d1",
        );
        let keyboard = interpreter.get_machine_state().keyboard_levels();
        assert_eq!((keyboard.key_down, keyboard.key_up), (5, 6));
        let interpreter = run_task("    move.b #62,d0\n    move.w #$0703,d1\n    trap #15\n    move.w #$0901,d1");
        let keyboard = interpreter.get_machine_state().keyboard_levels();
        assert_eq!((keyboard.key_down, keyboard.key_up), (7, 7));
    }

    #[test]
    fn the_settings_are_journaled_and_undone() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.b #62,d0
    move.w #$0201,d1
    trap #15
    move.b #32,d0
    move.b #6,d1
    move.b #$83,d2
    move.l #250,d3
    trap #15
    simhalt
",
        );
        run_to_the_pause(&mut interpreter);
        assert_eq!(interpreter.get_machine_state().auto_irqs().len(), 1);
        let journaled = interpreter
            .get_last_steps(100)
            .iter()
            .flat_map(|step| step.get_mutations())
            .filter(|mutation| matches!(mutation, MutationOperation::SetMachineState { .. }))
            .count();
        assert_eq!(journaled, 2);
        interpreter.undo().unwrap();
        interpreter.undo().unwrap();
        assert!(interpreter.get_machine_state().auto_irqs().is_empty());
        assert_eq!(interpreter.get_machine_state().keyboard_levels().key_down, 2);
        while interpreter.can_undo() {
            interpreter.undo().unwrap();
        }
        assert_eq!(interpreter.get_machine_state(), MachineState::default());
    }
}

mod device_events {
    use super::*;

    #[test]
    fn an_event_raises_the_level_the_program_asked_for() {
        let mut interpreter = machine(&program(
            "    move.b #62,d0\n    move.w #$0501,d1\n    trap #15
    move.b #60,d0\n    move.w #$0304,d1\n    trap #15
    simhalt\n    nop\n    simhalt",
            &[],
        ));
        run_to_the_pause(&mut interpreter);
        assert_eq!(interpreter.device_event(DeviceEvent::KeyUp), None);
        assert_eq!(interpreter.device_event(DeviceEvent::MouseDown), None);
        assert!(pending(&interpreter).is_empty(), "nothing asked for those");
        assert_eq!(interpreter.device_event(DeviceEvent::KeyDown), Some(5));
        assert_eq!(interpreter.device_event(DeviceEvent::MouseMove), Some(3));
        assert_eq!(pending(&interpreter), [3, 5]);
        run_to_the_pause(&mut interpreter);
        assert_eq!(log(&interpreter), [5, 3]);
    }

    #[test]
    fn an_event_with_no_level_set_changes_nothing() {
        let mut interpreter = machine(&program("    simhalt", &[]));
        run_to_the_pause(&mut interpreter);
        for event in [
            DeviceEvent::KeyDown,
            DeviceEvent::KeyUp,
            DeviceEvent::MouseDown,
            DeviceEvent::MouseUp,
            DeviceEvent::MouseMove,
        ] {
            assert_eq!(interpreter.device_event(event), None);
        }
        assert_eq!(interpreter.get_machine_state(), MachineState::default());
    }
}

mod breakpoints {
    use super::*;

    /// A run takes the request before it looks at the breakpoint on the
    /// program counter: the next instruction to run is the handler's, and a
    /// breakpoint there stops the run first.
    #[test]
    fn a_pending_request_is_taken_before_the_breakpoint_at_the_pc() {
        let code = program(
            "    simhalt\nresume:\n    nop\n    simhalt",
            &[(2, "    move.b #2,(a0)+ ; first\n    rte")],
        );
        let at_resume = Breakpoint::new(DEFAULT_ENTRY_PATH, line_of(&code, "    nop"));
        let in_handler = Breakpoint::new(DEFAULT_ENTRY_PATH, line_of(&code, "; first"));
        let breakpoints = [at_resume.clone(), in_handler];
        let mut interpreter = machine(&code);
        run_to_the_pause(&mut interpreter);
        interpreter.raise_irq(2).unwrap();
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, RunOptions::RESUME),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_pc(), label(&interpreter, "irq2"), "in the handler");
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, RunOptions::CONTINUE),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_pc(), label(&interpreter, "resume"), "back at the `nop`");
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, RunOptions::CONTINUE),
            Ok(InterpreterStatus::Paused)
        );
        assert_eq!(log(&interpreter), [2]);
    }

    /// Continuing from a breakpoint with a request pending takes the request
    /// first, so the run's first step is the IRQ: the handler runs, and the
    /// breakpoint stops the run again when it returns, its instruction being
    /// about to run. The next continue runs it.
    #[test]
    fn continuing_from_a_breakpoint_with_a_request_pending_runs_the_handler_first() {
        let code = program("    simhalt\nresume:\n    nop\n    simhalt", &[]);
        let breakpoints = [Breakpoint::new(DEFAULT_ENTRY_PATH, line_of(&code, "    nop"))];
        let mut interpreter = machine(&code);
        run_to_the_pause(&mut interpreter);
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, RunOptions::RESUME),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_pc(), label(&interpreter, "resume"));
        interpreter.raise_irq(4).unwrap();
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, RunOptions::CONTINUE),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_pc(), label(&interpreter, "resume"));
        assert_eq!(log(&interpreter), [4]);
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, RunOptions::CONTINUE),
            Ok(InterpreterStatus::Paused)
        );
        //resuming mid-program stops at it when the handler returns too
        let mut resumed = machine(&code);
        run_to_the_pause(&mut resumed);
        resumed
            .run_with_breakpoints(&breakpoints, None, RunOptions::RESUME)
            .unwrap();
        resumed.raise_irq(4).unwrap();
        assert_eq!(
            resumed.run_with_breakpoints(&breakpoints, None, RunOptions::RESUME),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(resumed.get_pc(), label(&resumed, "resume"));
        assert_eq!(log(&resumed), [4]);
    }
}
