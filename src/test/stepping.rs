//! What a debugger's Step over and Step out ask of the Interpreter (the
//! asm-editor's `docs/design/stepping-plan.md`): a call stack whose one rule
//! is that a frame is gone once the stack pointer of its stack is above it,
//! the call depth, runs that stop at a depth (`RunOptions::until_call_depth`)
//! and say why they stopped, and undo across all of it.

use crate::assembler::program::Program;
use crate::assembler::source::DEFAULT_ENTRY_PATH;
use crate::debugger::{FrameChange, FrameKind, MutationOperation, CALL_STACK_LIMIT};
use crate::instructions::{InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    Breakpoint, Interpreter, InterpreterOptions, InterpreterStatus, RunOptions, RuntimeError,
    StackPointer, StopReason, INITIAL_SUPERVISOR_STACK_POINTER, INITIAL_USER_STACK_POINTER,
};

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

fn without_history(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    )
}

fn label(interpreter: &Interpreter, name: &str) -> usize {
    interpreter
        .get_program()
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no label `{name}`"))
        .value as usize
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

/// A breakpoint on the line of `code` that holds `text`.
fn breakpoint_on(code: &str, text: &str) -> Breakpoint {
    let line = code
        .lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no line holds `{text}`"));
    Breakpoint::new(DEFAULT_ENTRY_PATH, line)
}

/// The routine each frame is in, oldest first.
fn names(interpreter: &Interpreter) -> Vec<String> {
    interpreter
        .get_pretty_call_stack()
        .iter()
        .map(|frame| frame.label_name.clone())
        .collect()
}

fn step_to(interpreter: &mut Interpreter, name: &str) {
    let address = label(interpreter, name);
    for _ in 0..10_000 {
        if interpreter.get_pc() == address {
            return;
        }
        interpreter.step().unwrap();
    }
    panic!("never reached `{name}`");
}

/// Everything undo has to put back of the call stack: the depth, and each
/// frame kept, oldest first.
#[derive(Debug, PartialEq)]
struct Stack {
    depth: usize,
    frames: Vec<(usize, usize, u32, StackPointer, FrameKind, Option<u8>)>,
}

fn stack(interpreter: &Interpreter) -> Stack {
    Stack {
        depth: interpreter.call_depth(),
        frames: interpreter
            .get_pretty_call_stack()
            .iter()
            .map(|frame| {
                (
                    frame.address,
                    frame.source_address,
                    frame.sp,
                    frame.stack,
                    frame.kind,
                    frame.vector,
                )
            })
            .collect(),
    }
}

/// Steps until the program stops running, and answers the depth before each
/// step and after the last.
fn walk(interpreter: &mut Interpreter) -> (Vec<usize>, Vec<Stack>) {
    let mut depths = vec![interpreter.call_depth()];
    let mut seen = vec![];
    while interpreter.get_status() == &InterpreterStatus::Running {
        seen.push(stack(interpreter));
        interpreter.step().unwrap();
        depths.push(interpreter.call_depth());
    }
    (depths, seen)
}

/// Undoes the steps [`walk`] took, newest first, checking that each puts the
/// call stack back as it was before it.
fn undo_all(interpreter: &mut Interpreter, seen: Vec<Stack>) {
    for (age, expected) in seen.into_iter().rev().enumerate() {
        interpreter.undo().unwrap();
        assert_eq!(stack(interpreter), expected, "undo {age}");
    }
}

mod undoing_many_steps {
    use super::*;

    #[test]
    fn takes_back_as_many_as_asked_and_no_more_than_the_history_holds() {
        let mut interpreter = machine(
            "    org $1000
start:
    moveq #1, d0
    bsr twice
    move.l d0, d1
    simhalt
twice:
    add.l d0, d0
    rts
",
        );
        for _ in 0..4 {
            interpreter.step().unwrap();
        }
        assert_eq!(d(&interpreter, 0), 2);
        assert_eq!(interpreter.get_undo_depth(), 4);

        //the call, the routine and its return, as one undo of a Step over would take them
        assert_eq!(interpreter.undo_many(3).unwrap(), 3);
        assert_eq!(d(&interpreter, 0), 1);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "start") + 2);
        assert_eq!(interpreter.call_depth(), 0);
        assert_eq!(interpreter.get_undo_depth(), 1);

        //and it stops at the oldest step the history holds
        assert_eq!(interpreter.undo_many(10).unwrap(), 1);
        assert_eq!(interpreter.undo_many(10).unwrap(), 0);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "start"));
    }
}

mod the_frame_rule {
    use super::*;

    #[test]
    fn nested_calls_count_one_frame_each() {
        let mut interpreter = machine(
            "    org $1000
start:
    bsr a
    simhalt
a:
    bsr b
    rts
b:
    bsr c
    rts
c:
    rts
",
        );
        step_to(&mut interpreter, "c");
        assert_eq!(names(&interpreter), ["a", "b", "c"]);
        let frames = interpreter.get_pretty_call_stack();
        //each return address pushed below the one before, on the supervisor stack
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame.sp, INITIAL_SUPERVISOR_STACK_POINTER - 4 * (index as u32 + 1));
            assert_eq!(frame.stack, StackPointer::Interrupt);
            assert_eq!(frame.registers[15], frame.sp, "A7 just after the push");
        }
        let json = serde_json::to_value(&frames[0]).unwrap();
        assert_eq!(json["stack"], "ssp");
        assert_eq!(json["sp"], INITIAL_SUPERVISOR_STACK_POINTER - 4);

        let mut interpreter = machine(
            "    org $1000
start:
    bsr a
    simhalt
a:
    bsr b
    rts
b:
    bsr c
    rts
c:
    rts
",
        );
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths, [0, 1, 2, 3, 2, 1, 0, 0]);
        undo_all(&mut interpreter, seen);
    }

    #[test]
    fn recursion_is_a_frame_per_level() {
        let mut interpreter = machine(
            "    org $1000
start:
    moveq #3,d0
    bsr fact
    simhalt
fact:
    subq.l #1,d0
    beq base
    bsr fact
base:
    rts
",
        );
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths.iter().max(), Some(&3));
        assert_eq!(depths.last(), Some(&0));
        undo_all(&mut interpreter, seen);
        assert_eq!(interpreter.call_depth(), 0);
    }

    /// The usual way to read parameters written after the call: the routine
    /// pops its return address and jumps past them. Its frame is gone at the
    /// pop, and the frames after it are right, where the pops by instruction
    /// left a stale frame for every later `rts` to take.
    #[test]
    fn a_routine_that_pops_its_return_address_is_gone_at_the_pop() {
        let mut interpreter = machine(
            "    org $1000
start:
    bsr outer
    simhalt
outer:
    bsr param
    dc.w 42
    move.w d0,d1
    bsr leaf
    rts
param:
    move.l (sp)+,a0
    move.w (a0)+,d0
    jmp (a0)
leaf:
    rts
",
        );
        step_to(&mut interpreter, "param");
        assert_eq!(names(&interpreter), ["outer", "param"]);
        interpreter.step().unwrap();
        assert_eq!(names(&interpreter), ["outer"], "gone at the pop");
        step_to(&mut interpreter, "leaf");
        assert_eq!(names(&interpreter), ["outer", "leaf"]);
        assert_eq!(d(&interpreter, 1), 42);
        interpreter.step().unwrap();
        assert_eq!(names(&interpreter), ["outer"]);
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 0);

        let mut interpreter = machine(
            "    org $1000
start:
    bsr outer
    simhalt
outer:
    bsr param
    dc.w 42
    move.w d0,d1
    bsr leaf
    rts
param:
    move.l (sp)+,a0
    move.w (a0)+,d0
    jmp (a0)
leaf:
    rts
",
        );
        let (_, seen) = walk(&mut interpreter);
        undo_all(&mut interpreter, seen);
    }

    /// A routine that moves its return address past the data after the call
    /// returns there with `rts`, and its frame is gone.
    #[test]
    fn a_changed_return_address_returns_where_it_says() {
        let mut interpreter = machine(
            "    org $1000
start:
    bsr skip2
    dc.w $4AFC
    moveq #1,d1
    simhalt
skip2:
    addq.l #2,(sp)
    rts
",
        );
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths, [0, 1, 1, 0, 0, 0]);
        assert_eq!(d(&interpreter, 1), 1, "the illegal word was skipped");
        undo_all(&mut interpreter, seen);
    }

    #[test]
    fn rtd_and_rtr_return_by_the_stack_pointer() {
        let mut interpreter = machine(
            "    opt p=68010
    org $1000
start:
    move.l #7,-(sp)
    bsr callee
    bsr flags
    simhalt
callee:
    rtd #4
flags:
    move.w #$001F,-(sp)
    rtr
",
        );
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths, [0, 0, 1, 0, 1, 1, 0, 0]);
        assert_eq!(interpreter.get_sp(), INITIAL_SUPERVISOR_STACK_POINTER as usize);
        assert_eq!(interpreter.get_sr() & 0xFF, 0x1F);
        undo_all(&mut interpreter, seen);
    }

    /// A routine that ends by jumping to another: the second one's `rts`
    /// returns to the first one's caller, and the call is over.
    #[test]
    fn a_tail_call_ends_when_the_second_routine_returns() {
        let mut interpreter = machine(
            "    org $1000
start:
    bsr first
    simhalt
first:
    jmp second
second:
    rts
",
        );
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths, [0, 1, 1, 0, 0]);
        undo_all(&mut interpreter, seen);
    }

    #[test]
    fn a_trap_from_supervisor_state_is_a_frame_until_its_rte() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$80
    trap #0
    simhalt
handler:
    nop
    rte
",
        );
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        let frames = interpreter.get_pretty_call_stack();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, FrameKind::Exception);
        assert_eq!(frames[0].stack, StackPointer::Interrupt);
        assert_eq!(frames[0].sp, INITIAL_SUPERVISOR_STACK_POINTER - 6);
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 1);
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 0, "gone after the `rte`");

        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$80
    trap #0
    simhalt
handler:
    nop
    rte
",
        );
        let (_, seen) = walk(&mut interpreter);
        undo_all(&mut interpreter, seen);
    }

    /// From user state the frame is on the supervisor stack, under a call
    /// frame on the user stack, whose stack pointer the trap does not move.
    #[test]
    fn a_trap_from_user_state_is_a_frame_on_the_supervisor_stack() {
        let code = "    org $1000
start:
    move.l #handler,$80
    move.w #$0000,sr
    bsr routine
    simhalt
routine:
    trap #0
    rts
handler:
    rte
";
        let mut interpreter = machine(code);
        step_to(&mut interpreter, "routine");
        let usp = interpreter.get_system().usp;
        assert_eq!(usp, INITIAL_USER_STACK_POINTER - 4);
        interpreter.step().unwrap();
        let frames = interpreter.get_pretty_call_stack();
        assert_eq!(
            frames
                .iter()
                .map(|frame| (frame.kind, frame.stack))
                .collect::<Vec<_>>(),
            [
                (FrameKind::Call, StackPointer::User),
                (FrameKind::Exception, StackPointer::Interrupt)
            ]
        );
        assert_eq!(interpreter.get_system().usp, usp, "the trap left the user stack");
        interpreter.step().unwrap();
        assert_eq!(names(&interpreter), ["routine"], "the `rte` ended the trap's frame");
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 0);

        let mut interpreter = machine(code);
        let (_, seen) = walk(&mut interpreter);
        undo_all(&mut interpreter, seen);
    }

    /// A handler that returns with `rtr`, which pops the 68000's status word
    /// and program counter as `rte` would but for the mode, or that drops the
    /// status word and returns with `rts`: its frame is gone all the same, and
    /// the step records the return from the exception.
    #[test]
    fn a_handler_left_with_rtr_or_rts_is_gone() {
        for (exit, depths) in [
            ("    rtr", vec![0, 0, 1, 0, 0, 0]),
            ("    addq.l #2,sp\n    rts", vec![0, 0, 1, 0, 0, 0, 0]),
        ] {
            let mut interpreter = machine(&format!(
                "    org $1000
start:
    move.l #handler,$80
    trap #0
    moveq #1,d1
    simhalt
handler:
{exit}
"
            ));
            let (seen_depths, seen) = walk(&mut interpreter);
            assert_eq!(seen_depths, depths, "{exit}");
            assert_eq!(d(&interpreter, 1), 1);
            let returned = interpreter
                .get_last_steps(10)
                .iter()
                .flat_map(|step| step.get_mutations())
                .filter(|mutation| matches!(mutation, MutationOperation::PopException { .. }))
                .count();
            assert_eq!(returned, 1, "{exit}");
            undo_all(&mut interpreter, seen);
        }
    }

    /// Several frames gone in one step, an unwind past two calls, are taken
    /// off together, and one undo puts both back.
    #[test]
    fn an_unwind_takes_several_frames_off_in_one_step() {
        let code = "    org $1000
start:
    bsr a
back:
    moveq #1,d1
    simhalt
a:
    bsr b
    rts
b:
    lea 8(sp),sp
    jmp back
";
        let mut interpreter = machine(code);
        step_to(&mut interpreter, "b");
        assert_eq!(interpreter.call_depth(), 2);
        let before = stack(&interpreter);
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 0);
        let step = interpreter.get_last_steps(1)[0].clone();
        assert!(matches!(
            step.get_frame_changes(),
            [FrameChange::Removed(_), FrameChange::Removed(_)]
        ));
        let pops = step
            .get_mutations()
            .iter()
            .filter(|mutation| matches!(mutation, MutationOperation::PopCall { .. }))
            .count();
        assert_eq!(pops, 2);
        interpreter.undo().unwrap();
        assert_eq!(stack(&interpreter), before);

        let mut interpreter = machine(code);
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths, [0, 1, 2, 0, 0, 0, 0]);
        undo_all(&mut interpreter, seen);
    }

    /// An IRQ taken during a routine is a frame of its own on top of the
    /// routine's, gone after its handler's `rte`, and its step is undone like
    /// any other.
    #[test]
    fn an_irq_taken_during_a_routine_is_a_frame_on_top() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #irq2,$68
    bsr routine
    simhalt
routine:
    nop
    nop
    rts
irq2:
    rte
",
        );
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        let mut seen = vec![stack(&interpreter)];
        interpreter.raise_irq(2).unwrap();
        interpreter.step().unwrap();
        assert_eq!(names(&interpreter), ["routine", "irq2"]);
        assert_eq!(interpreter.get_pretty_call_stack()[1].vector, Some(26));
        seen.push(stack(&interpreter));
        interpreter.step().unwrap();
        assert_eq!(names(&interpreter), ["routine"]);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "routine"));
        let (depths, rest) = walk(&mut interpreter);
        assert_eq!(depths, [1, 1, 1, 0, 0]);
        undo_all(&mut interpreter, rest);
        //the rte, then the IRQ
        interpreter.undo().unwrap();
        assert_eq!(stack(&interpreter), seen.pop().unwrap());
        interpreter.undo().unwrap();
        assert_eq!(stack(&interpreter), seen.pop().unwrap());
    }

    /// An instruction that starts with T set and raises a group 2 exception
    /// pushes two frames in one step: its own exception's and the trace's on
    /// top. One undo takes both off.
    #[test]
    fn a_trap_traced_pushes_two_frames_in_one_step() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #trap_handler,$80
    move.l #trace_handler,$24
    move.w #$A000,sr
    trap #0
    moveq #1,d1
    simhalt
trace_handler:
    rte
trap_handler:
    andi.w #$7FFF,(sp)
    rte
",
        );
        for _ in 0..3 {
            interpreter.step().unwrap();
        }
        let before = stack(&interpreter);
        interpreter.step().unwrap();
        assert_eq!(names(&interpreter), ["trap_handler", "trace_handler"]);
        assert_eq!(
            interpreter.get_last_steps(1)[0].get_frame_changes().len(),
            2,
            "both in one step"
        );
        interpreter.undo().unwrap();
        assert_eq!(stack(&interpreter), before);
        let (depths, seen) = walk(&mut interpreter);
        assert_eq!(depths, [0, 2, 1, 1, 0, 0, 0]);
        assert_eq!(d(&interpreter, 1), 1);
        undo_all(&mut interpreter, seen);
    }

    /// A 68020's interrupt in the master state pushes its frame on the master
    /// stack and a throwaway frame on the interrupt stack, where the handler
    /// runs: one frame for both, on the master stack, gone when the `rte` has
    /// popped them both. A call the handler makes is on the interrupt stack.
    #[test]
    fn an_interrupt_in_the_master_state_is_one_frame() {
        let mut interpreter = machine(
            "    opt p=68020
    org $1000
start:
    move.l #irq,$68
    move.l #$8000,d0
    movec d0,msp
    move.w #$3000,sr
spin:
    nop
    simhalt
irq:
    bsr sub
    rte
sub:
    rts
",
        );
        for _ in 0..4 {
            interpreter.step().unwrap();
        }
        let mut seen = vec![];
        interpreter.raise_irq(2).unwrap();
        seen.push(stack(&interpreter));
        interpreter.step().unwrap();
        let frames = interpreter.get_pretty_call_stack();
        assert_eq!(frames.len(), 1, "one exception, one frame");
        assert_eq!(frames[0].stack, StackPointer::Master);
        assert_eq!(frames[0].sp, 0x7FF8, "the format $0 frame on the master stack");
        seen.push(stack(&interpreter));
        interpreter.step().unwrap();
        assert_eq!(
            interpreter
                .get_pretty_call_stack()
                .iter()
                .map(|frame| frame.stack)
                .collect::<Vec<_>>(),
            [StackPointer::Master, StackPointer::Interrupt]
        );
        seen.push(stack(&interpreter));
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 1, "back in the handler");
        seen.push(stack(&interpreter));
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 0, "the `rte` popped both frames");
        assert_eq!(interpreter.get_pc(), label(&interpreter, "spin"));
        for expected in seen.into_iter().rev() {
            interpreter.undo().unwrap();
            assert_eq!(stack(&interpreter), expected);
        }
    }

    /// A return with nothing on the call stack, from a frame the program built
    /// itself, finds nothing to take off.
    #[test]
    fn a_return_from_no_call_takes_nothing() {
        let mut interpreter = machine(
            "    org $1000
start:
    pea there
    rts
there:
    moveq #1,d1
    simhalt
",
        );
        let (depths, seen) = walk(&mut interpreter);
        assert!(depths.iter().all(|depth| *depth == 0));
        assert_eq!(d(&interpreter, 1), 1);
        undo_all(&mut interpreter, seen);
    }

    /// The call stack is kept whether or not the history records.
    #[test]
    fn the_call_stack_is_kept_without_a_history() {
        let mut interpreter = without_history(
            "    org $1000
start:
    bsr a
    simhalt
a:
    bsr b
    rts
b:
    move.l (sp)+,a0
    jmp (a0)
",
        );
        let (depths, _) = walk(&mut interpreter);
        assert_eq!(depths, [0, 1, 2, 1, 1, 0, 0]);
        assert_eq!(interpreter.get_recorded_steps(), 0);
        assert!(!interpreter.can_undo());
    }

    #[test]
    fn the_newest_frames_are_asked_for_without_the_rest() {
        let mut interpreter = machine(
            "    org $1000
start:
    bsr a
    simhalt
a:
    bsr b
    rts
b:
    bsr c
    rts
c:
    rts
",
        );
        step_to(&mut interpreter, "c");
        let newest: Vec<String> = interpreter
            .get_newest_call_stack(2)
            .iter()
            .map(|frame| frame.label_name.clone())
            .collect();
        assert_eq!(newest, ["b", "c"], "oldest of them first");
        assert_eq!(interpreter.get_newest_call_stack(10).len(), 3);
        assert!(interpreter.get_newest_call_stack(0).is_empty());
    }
}

mod the_frame_limit {
    use super::*;

    /// Past the limit the oldest frames are forgotten and the depth goes on
    /// counting; undo puts a forgotten frame back exactly.
    #[test]
    fn the_newest_frames_are_kept_and_undo_brings_back_the_forgotten() {
        let mut interpreter = Interpreter::new(
            assemble(
                "    org $1000
start:
    bsr start
",
            ),
            Some(InterpreterOptions {
                keep_history: true,
                history_size: 10,
            }),
        );
        let calls = CALL_STACK_LIMIT + 5;
        assert_eq!(
            interpreter.run_with_limit(calls),
            Err(RuntimeError::ExecutionLimit(calls))
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Limit);
        assert_eq!(interpreter.call_depth(), calls);
        let oldest_sp = |interpreter: &Interpreter| interpreter.get_pretty_call_stack()[0].sp;
        assert_eq!(interpreter.get_pretty_call_stack().len(), CALL_STACK_LIMIT);
        //the first five forgotten, the sixth the oldest kept
        assert_eq!(oldest_sp(&interpreter), INITIAL_SUPERVISOR_STACK_POINTER - 4 * 6);
        interpreter.undo().unwrap();
        assert_eq!(interpreter.call_depth(), calls - 1);
        assert_eq!(interpreter.get_pretty_call_stack().len(), CALL_STACK_LIMIT);
        assert_eq!(oldest_sp(&interpreter), INITIAL_SUPERVISOR_STACK_POINTER - 4 * 5);
        interpreter.undo().unwrap();
        assert_eq!(oldest_sp(&interpreter), INITIAL_SUPERVISOR_STACK_POINTER - 4 * 4);
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), calls - 1);
        assert_eq!(oldest_sp(&interpreter), INITIAL_SUPERVISOR_STACK_POINTER - 4 * 5);
    }

    /// A program deeper than the limit that returns all the way: nothing can
    /// tell when the forgotten frames are gone, so the depth stays at their
    /// number once every frame kept is gone.
    #[test]
    fn the_forgotten_frames_stay_counted() {
        let mut interpreter = without_history(&format!(
            "    org $1000
start:
    move.l #{},d0
    bsr rec
    simhalt
rec:
    subq.l #1,d0
    beq base
    bsr rec
base:
    rts
",
            CALL_STACK_LIMIT + 5
        ));
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(interpreter.get_sp(), INITIAL_SUPERVISOR_STACK_POINTER as usize);
        assert!(interpreter.get_pretty_call_stack().is_empty());
        assert_eq!(interpreter.call_depth(), 5);
    }
}

mod until_call_depth {
    use super::*;

    const ROUTINE: &str = "    org $1000
start:
    bsr routine
after:
    moveq #9,d7
    simhalt
routine:
    moveq #1,d1
    moveq #2,d2
    rts
";

    fn entered() -> Interpreter {
        let mut interpreter = machine(ROUTINE);
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 1);
        interpreter
    }

    fn until(depth: usize) -> RunOptions {
        RunOptions::CONTINUE.until_call_depth(depth)
    }

    #[test]
    fn a_run_stops_when_the_routine_has_returned() {
        let mut interpreter = entered();
        assert_eq!(
            interpreter.run_with_options(None, until(0)),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "after"));
        assert_eq!(d(&interpreter, 7), 0, "the instruction it stopped at has not run");
    }

    /// Reached by the last instruction the limit allows, the depth is the
    /// stop, not the limit; the same run asked for no depth answers the limit.
    #[test]
    fn the_depth_reached_on_the_last_instruction_of_the_limit_is_the_stop() {
        let mut interpreter = entered();
        assert_eq!(
            interpreter.run_with_options(Some(3), until(0)),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "after"));

        let mut interpreter = entered();
        assert_eq!(
            interpreter.run_with_options(Some(3), RunOptions::CONTINUE),
            Err(RuntimeError::ExecutionLimit(3))
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Limit);
        let mut interpreter = entered();
        assert_eq!(
            interpreter.run_with_options(Some(2), until(0)),
            Err(RuntimeError::ExecutionLimit(2))
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Limit);
    }

    #[test]
    fn a_breakpoint_where_the_routine_returns_reports_the_depth() {
        let mut interpreter = entered();
        let breakpoints = [breakpoint_on(ROUTINE, "moveq #9,d7")];
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, until(0)),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "after"));
    }

    #[test]
    fn a_breakpoint_inside_the_routine_stops_the_run_there() {
        let mut interpreter = entered();
        let breakpoints = [breakpoint_on(ROUTINE, "moveq #2,d2")];
        assert_eq!(
            interpreter.run_with_breakpoints(&breakpoints, None, until(0)),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Breakpoint);
        assert_eq!(interpreter.call_depth(), 1);
        assert_eq!(d(&interpreter, 1), 1);
        assert_eq!(d(&interpreter, 2), 0);
        //on the first instruction too, when the run does not skip it
        let mut interpreter = entered();
        let breakpoints = [breakpoint_on(ROUTINE, "moveq #1,d1")];
        interpreter
            .run_with_breakpoints(&breakpoints, None, RunOptions::RESUME.until_call_depth(0))
            .unwrap();
        assert_eq!(interpreter.get_stop_reason(), StopReason::Breakpoint);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "routine"));
    }

    /// A run started at the depth, or above it, runs nothing, whatever is at
    /// the program counter: an IRQ ready is not taken either.
    #[test]
    fn a_run_started_at_the_depth_runs_nothing() {
        let mut interpreter = machine(ROUTINE);
        for depth in [0, 3] {
            let ran = interpreter.get_executed_instructions();
            assert_eq!(
                interpreter.run_with_options(None, until(depth)),
                Ok(InterpreterStatus::Running)
            );
            assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
            assert_eq!(interpreter.get_executed_instructions(), ran);
            assert_eq!(interpreter.get_pc(), 0x1000);
            assert!(!interpreter.can_undo());
        }
        //with an IRQ ready, which would be the next step
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #irq2,$68
    nop
    simhalt
irq2:
    rte
",
        );
        interpreter.step().unwrap();
        interpreter.raise_irq(2).unwrap();
        interpreter.run_with_options(None, until(0)).unwrap();
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_machine_state().pending_levels(), [2]);
        assert_eq!(interpreter.call_depth(), 0);
    }

    /// The host answers a trap inside the routine and runs on: the run after
    /// the answer stops when the routine returns.
    #[test]
    fn a_run_resumed_after_an_answer_stops_at_the_depth() {
        let code = "    org $1000
start:
    bsr routine
after:
    simhalt
routine:
    moveq #6,d0
    moveq #65,d1
    trap #15
    rts
";
        let mut interpreter = machine(code);
        interpreter.step().unwrap();
        assert_eq!(
            interpreter.run_with_options(None, until(0)),
            Ok(InterpreterStatus::Interrupt)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Status);
        interpreter
            .answer_interrupt(InterruptResult::DisplayChar)
            .unwrap();
        assert_eq!(
            interpreter.run_with_options(None, RunOptions::RESUME.until_call_depth(0)),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "after"));
    }

    /// The instruction that reached the depth stopped the run itself, with a
    /// watched write: that stop is answered first, and the run the host makes
    /// after it stops at once, having run nothing.
    #[test]
    fn a_stop_of_the_step_that_reached_the_depth_comes_first() {
        let mut interpreter = machine(
            "    org $1000
start:
    lea slot,a1
    bsr routine
after:
    simhalt
routine:
    move.l (sp)+,(a1)
    move.l (a1),a0
    jmp (a0)
slot:
    ds.l 1
",
        );
        let slot = label(&interpreter, "slot");
        interpreter.watch_memory(slot, slot + 4).unwrap();
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        assert_eq!(
            interpreter.run_with_options(None, until(0)),
            Ok(InterpreterStatus::WatchedWrite)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Status);
        assert_eq!(interpreter.call_depth(), 0, "gone at the pop");
        interpreter.take_watched_writes();
        let pc = interpreter.get_pc();
        let ran = interpreter.get_executed_instructions();
        assert_eq!(
            interpreter.run_with_options(None, RunOptions::RESUME.until_call_depth(0)),
            Ok(InterpreterStatus::Running)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_pc(), pc);
        assert_eq!(interpreter.get_executed_instructions(), ran);
    }

    /// A call that is the program's last instruction returns to its end: the
    /// end wins over the depth, and the program has ended.
    #[test]
    fn the_end_of_the_program_wins_over_the_depth() {
        let mut interpreter = machine(
            "    org $1000
routine:
    rts
start:
    bsr routine
    end start
",
        );
        interpreter.step().unwrap();
        assert_eq!(interpreter.call_depth(), 1);
        assert_eq!(
            interpreter.run_with_options(None, until(0)),
            Ok(InterpreterStatus::Terminated)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Status);
        assert!(interpreter.run_with_options(None, until(0)).is_err());
    }

    /// An IRQ raised before the run is taken and its handler runs to its
    /// return before the routine goes on to its own.
    #[test]
    fn an_irq_during_the_run_is_run_through() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #irq2,$68
    bsr routine
after:
    simhalt
routine:
    nop
    rts
irq2:
    moveq #2,d2
    rte
",
        );
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        interpreter.raise_irq(2).unwrap();
        interpreter.run_with_options(None, until(0)).unwrap();
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "after"));
        assert_eq!(d(&interpreter, 2), 2);
    }

    /// Recursion: the run stops when the depth is back, not at the first
    /// return to the same address, which a deeper level makes first.
    #[test]
    fn recursion_stops_when_the_depth_is_back() {
        let mut interpreter = machine(
            "    org $1000
start:
    moveq #4,d0
    bsr fact
    simhalt
fact:
    subq.l #1,d0
    beq base
    bsr fact
base:
    rts
",
        );
        while interpreter.call_depth() < 2 {
            interpreter.step().unwrap();
        }
        interpreter.run_with_options(None, until(1)).unwrap();
        assert_eq!(interpreter.get_stop_reason(), StopReason::CallDepth);
        assert_eq!(interpreter.call_depth(), 1);
        assert_eq!(interpreter.get_pc(), label(&interpreter, "base"));
        assert_eq!(names(&interpreter), ["fact"]);
    }

    /// A step, and a run that stops for its status, answer the status as their
    /// reason; a run with no depth asked for never stops for one.
    #[test]
    fn the_reason_of_a_step_and_of_a_plain_run() {
        let mut interpreter = entered();
        interpreter.run_with_options(None, until(0)).unwrap();
        interpreter.step().unwrap();
        assert_eq!(interpreter.get_stop_reason(), StopReason::Status);
        let mut interpreter = entered();
        assert_eq!(
            interpreter.run_with_options(None, RunOptions::CONTINUE),
            Ok(InterpreterStatus::Paused)
        );
        assert_eq!(interpreter.get_stop_reason(), StopReason::Status);
    }
}

mod the_history_ledger {
    use super::*;

    /// How many steps undo can take back, and how many the history has
    /// recorded, an IRQ's included, which undo never takes back.
    #[test]
    fn the_undo_depth_and_the_recorded_steps() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #irq2,$68
    nop
    nop
    simhalt
irq2:
    rte
",
        );
        assert_eq!(interpreter.get_undo_depth(), 0);
        assert_eq!(interpreter.get_recorded_steps(), 0);
        interpreter.step().unwrap();
        interpreter.raise_irq(2).unwrap();
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        assert_eq!(interpreter.get_recorded_steps(), 3, "the IRQ is a step");
        assert_eq!(interpreter.get_undo_depth(), 3);
        assert_eq!(interpreter.get_executed_instructions(), 2);
        interpreter.undo().unwrap();
        interpreter.undo().unwrap();
        assert_eq!(interpreter.get_undo_depth(), 1);
        assert_eq!(interpreter.get_recorded_steps(), 3, "undo takes none back");
        assert_eq!(interpreter.get_last_step_id(), 1);
        interpreter.step().unwrap();
        assert_eq!(interpreter.get_recorded_steps(), 4);
        assert_eq!(interpreter.get_last_step_id(), 4, "an id is never given twice");
        let skipped: Vec<u64> = interpreter
            .get_steps_back(1, 5)
            .iter()
            .map(|step| step.get_id())
            .collect();
        assert_eq!(skipped, [1]);
    }

    /// A full history holds its size, and undo takes back no more.
    #[test]
    fn the_undo_depth_stops_at_the_history_size() {
        let mut interpreter = Interpreter::new(
            assemble("    org $1000\nloop:\n    bra loop\n"),
            Some(InterpreterOptions {
                keep_history: true,
                history_size: 4,
            }),
        );
        for _ in 0..10 {
            interpreter.step().unwrap();
        }
        assert_eq!(interpreter.get_undo_depth(), 4);
        assert_eq!(interpreter.get_recorded_steps(), 10);
    }
}
