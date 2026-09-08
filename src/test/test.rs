//! The Interpreter's own tests: small programs assembled and run, and the trap
//! tasks answered one by one.
//!
//! Every one of them goes through [`assemble`], which is
//! [`assemble_source`](crate::assembler::assemble_source), so a program that
//! stops being accepted by the Assembler fails here with its Diagnostics rather
//! than with a panic about a missing Program.

use console::Term;

use crate::assembler::program::Program;
use crate::instructions::{Interrupt, InterruptResult};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError};

//TODO add better tests for all cases and if i find bugs etc
#[cfg(test)]
mod tests {
    use crate::instructions::{RegisterOperand, Size};
    use crate::test::test::assemble_and_run;

    /// `equ` names a value, and that value reaches the instruction that uses
    /// it.
    ///
    /// This case read `ten equ #10` and `register_1 equ d1` until the rewrite:
    /// 1.4.2's `equ` was a text substitution and could alias anything, the `#`
    /// of an immediate and a register name included. A Constant is a value now
    /// (ADR 0001, and CONTEXT.md, "Constant"), so the `#` belongs to the
    /// instruction and a register cannot be aliased at all.
    #[test]
    fn equ_substitution() {
        let interpreter = assemble_and_run(
            "ten equ 10
	move.l #ten, d1
",
        );
        assert_eq!(
            interpreter.get_register_value(RegisterOperand::Data(1), Size::Long),
            10
        );
    }

    #[test]
    fn correctly_apply_pre_decrement() {
        let interpreter = assemble_and_run(
            "move.l #$818081a8, d0
move.l #$1000, a0
move.l d0, (a0)+
move.l d0, (a0)
move.l #$0F0F, d0
and.w d0, (a0)
and.w d0, -(a0)",
        );
        let expected: u32 = 0x81800108;
        let expected2: u32 = 0x010081A8;
        let mem = interpreter.get_memory();
        assert_eq!(mem.read_long(0x1000).unwrap(), expected);
        assert_eq!(mem.read_long(0x1004).unwrap(), expected2);
    }

    #[test]
    fn test_addressing_modes() {
        assemble_and_run(
            "
    move.l #10, d0
    move.l #$10, (a0)
    move.l #10*2, (a0)+
    move.l #'hi', -(a0)
    move.l #10, 10(a0)
    move.l #10, 10(a0,d0)
    move.l #10, 10(a0,d0.w)
    move.l #10, 10(a0,d0.l)
    move.l d0, 1000
    move.l d0, $1000
    movem.l d0-d1/a0-a5/a7, (a0)
    movem.l D0-D1/A0-A5/A7, (a0)


        ",
        );
    }

    #[test]
    fn test_case_insensitive_registers_in_indirect_displacement() {
        assemble_and_run(
            "
    move.l #$1000, A0
    move.b #$42, $0(A0)
    move.b $0(A0), D0
    move.b $0(a0), D0
    move.b $0(A0), d0
        ",
        );
    }

    #[test]
    fn test_complex_code() {
        assemble_and_run(
            "ORG    $1000
    length: dc.w 20
    arr: dc.w 11, 71, 26, 44, 45, 65, 86, 10, 36, 26, 87, 86, 99, 48, 70, 89, 68, 92, 47, 80
START:
    * sort the array
    MOVE.l #arr, -(sp)
    MOVE.w length,-(sp)
    bsr sort_array
    add.l #6, sp
    * print the sorted array
    MOVE.l #arr, -(sp)
    MOVE.w length,-(sp)
    bsr print_array
    bra end
sa_off_length equ 44
sa_off_array_pointer equ 46
sort_array:
    MOVE.w d0, -(sp)
    MOVE.l d1, -(sp)
    MOVE.l d2, -(sp)
    MOVE.l d3, -(sp)
    MOVE.w d4, -(sp)
    MOVE.l d5, -(sp)
    MOVE.l a0, -(sp)
    MOVE.l a1, -(sp)
    MOVE.l a2, -(sp)
    MOVE.l d7, -(sp)
    MOVE.l a6, -(sp)
    MOVE.w sa_off_length(sp), d7
    muls #2, d7
    MOVE.l sa_off_array_pointer(sp), a6
    * uses:
* d0 (w) = i
* d1 (l) = end
* d2 (l) = j
* d3 (l) = diff
* d4 (w) = tmp
* d5 (l) = swaps
* a0 (l) = toSort
* a1 (l) = beforeElement
* a2 (l) = currentElement
* d7 (w) = parameter length
* a6 (l) = parameter array pointer
* total offset = 34
    MOVE.w #0, d0 *i = 0
    MOVE.w #0, d5 *swaps = 0
for_i_start:
    cmp.w d7, d0
    bge for_i_end *if(i >= length) goto for_i_end
    MOVE.w #2, d2 * j = 1
    MOVE.w d7, d1 * end = length
    sub.w d0, d1  * end -= i
    for_j_start:
        cmp d1, d2
        bge for_j_end   * if(j >= end) goto_for_j_end
        MOVE.l a6, a1   * beforeElement = array pointer
        add.l d2, a1    * beforeElement += j
        sub.l #2, a1    * beforeElement -= 1
        MOVE.l a6, a2   * currentElement = array pointer
        add.l d2, a2    * currentElement += j
        MOVE.w (a1), d3 * diff = *beforeElement
        sub.w (a2), d3  * diff -= *currentElement
        tst d3
        blt if_smaller  * if(diff < 0)
            MOVE.w (a1), d4 * tmp = *beforeElement
            add.l #1, d5 * swaps++
            MOVE.w (a2), (a1) * *beforeElement = *currentElement
            MOVE.w d4, (a2) * *currentElement = tmp
        if_smaller:
        add.l #2, d2 * j++
        bra for_j_start
    for_j_end:
    add.l #2, d0 * i++
    bra for_i_start
for_i_end:
    MOVE.l (sp)+, a6
    MOVE.l (sp)+, d7
    MOVE.l (sp)+, a2
    MOVE.l (sp)+, a1
    MOVE.l (sp)+, a0
    MOVE.l (sp)+, d5
    MOVE.w (sp)+, d4
    MOVE.l (sp)+, d3
    MOVE.l (sp)+, d2
    MOVE.l (sp)+, d1
    MOVE.w (sp)+, d0
    rts
* uses:  d0(l) d1(w) d7(w) a2(l) register offset = 12
* total offset = register offset + return = 16
pa_off_length equ 16
pa_off_array_pointer equ 18
print_array:
    MOVE.l d0, -(sp)
    MOVE.w d1, -(sp)
    MOVE.w d7, -(sp)
    MOVE.l a2, -(sp)
    MOVE.w pa_off_length(sp), d7
    MOVE.l pa_off_array_pointer(sp), a2
for_start:
    MOVE.l #3, d0
    MOVE.w (a2), d1
    add.l #2, a2
    trap #15
    MOVE.l #6, d0
    MOVE.l #',', d1
    trap #15
    tst d7
    sub.l #1, d7
    bgt for_start
for_end:
    MOVE.l (sp)+,a2
    MOVE.w (sp)+,d7
    MOVE.w (sp)+,d1
    MOVE.l (sp)+,d0
    rts
end:",
        );
    }

    /// What the Interpreter reads out of the Program: the Entry point, the
    /// initial memory, the Location of every instruction and the breakpoints
    /// and the call stack built on it.
    mod running_a_program {
        use crate::assembler::source::DEFAULT_ENTRY_PATH;
        use crate::interpreter::{Breakpoint, Interpreter, InterpreterOptions, InterpreterStatus};
        use crate::test::test::{assemble, prepare};

        /// An Interpreter over `code` that keeps a history, which is what the
        /// call stack and undo need.
        fn with_history(code: &str) -> Interpreter {
            Interpreter::new(
                assemble(code),
                Some(InterpreterOptions {
                    keep_history: true,
                    history_size: 100,
                }),
            )
        }

        #[test]
        fn the_run_starts_at_the_entry_point() {
            //`end` names it, and it is read even though it is written last
            let interpreter = prepare(
                "    org $1000
first:
    nop
second:
    nop
    end second
",
            );
            assert_eq!(interpreter.get_pc(), 0x1004);
        }

        #[test]
        fn a_program_with_no_instruction_has_already_terminated() {
            let interpreter = prepare("* nothing but a comment\n");
            assert!(interpreter.has_terminated());
        }

        #[test]
        fn paused_is_appended_to_the_public_status_values() {
            assert_eq!(InterpreterStatus::Running as u32, 0);
            assert_eq!(InterpreterStatus::Interrupt as u32, 1);
            assert_eq!(InterpreterStatus::Terminated as u32, 2);
            assert_eq!(InterpreterStatus::TerminatedWithException as u32, 3);
            assert_eq!(InterpreterStatus::Paused as u32, 4);
        }

        #[test]
        fn the_run_ends_after_the_last_instruction_and_not_on_it() {
            let mut interpreter = prepare("    org $1000\n    nop\n");
            assert!(!interpreter.has_reached_bottom());
            interpreter.step().expect("one instruction to run");
            assert_eq!(interpreter.get_pc(), 0x1004, "the nop is 4 bytes wide");
            assert!(interpreter.has_reached_bottom());
            assert!(interpreter.has_terminated());
        }

        #[test]
        fn simhalt_pauses_after_itself_and_step_resumes() {
            //`Directives/simhalt.htm`: the simulator halts and "No registers are
            //modified". The next step continues at the instruction after it.
            let mut interpreter = prepare(
                "    org $1000
    move.l #7,d0
    simhalt
    move.l #9,d0
    nop
",
            );
            interpreter.step().expect("the move");
            let status = interpreter.step().expect("the simhalt");
            assert_eq!(status, InterpreterStatus::Paused);
            assert!(!interpreter.has_terminated());
            assert_eq!(
                interpreter.get_cpu().get_register_values()[0],
                7,
                "simhalt modifies no register"
            );
            assert_eq!(
                interpreter.get_pc(),
                0x1008,
                "the program counter stops one past the simhalt"
            );
            assert_eq!(
                interpreter.step().expect("the step after the pause"),
                InterpreterStatus::Running
            );
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 9);
        }

        #[test]
        fn run_resumes_after_each_simhalt() {
            let mut interpreter = prepare(
                "    org $1000
    move.l #1,d0
    simhalt
    move.l #2,d0
    simhalt
    move.l #3,d0
",
            );
            assert_eq!(
                interpreter.run().expect("the first pause"),
                InterpreterStatus::Paused
            );
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 1);
            assert_eq!(
                interpreter.run().expect("the second pause"),
                InterpreterStatus::Paused
            );
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 2);
            assert_eq!(
                interpreter.run().expect("the end after the second resume"),
                InterpreterStatus::Terminated
            );
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 3);
        }

        #[test]
        fn simhalt_at_the_bottom_pauses_until_resumed() {
            let mut interpreter = prepare("    simhalt\n");
            assert_eq!(
                interpreter.step().expect("the simhalt"),
                InterpreterStatus::Paused
            );
            assert!(interpreter.has_reached_bottom());
            assert!(!interpreter.has_terminated());
            assert_eq!(
                interpreter
                    .step()
                    .expect("resume past the last instruction"),
                InterpreterStatus::Terminated
            );
        }

        #[test]
        fn undo_restores_both_sides_of_a_pause() {
            let mut interpreter = with_history(
                "    org $1000
    move.l #7,d0
    simhalt
    move.l #9,d0
    nop
",
            );
            interpreter.step().expect("the first move");
            interpreter.step().expect("the simhalt");
            interpreter.step().expect("the move after the pause");

            interpreter.undo().expect("the resumed move");
            assert_eq!(*interpreter.get_status(), InterpreterStatus::Paused);
            assert_eq!(interpreter.get_pc(), 0x1008);
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 7);

            interpreter.undo().expect("the simhalt");
            assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
            assert_eq!(interpreter.get_pc(), 0x1004);
        }

        #[test]
        fn dc_writes_its_bytes_and_ds_only_reserves_room() {
            //`ds` stores nothing (`Directives/ds.htm`), so its block keeps the fill memory
            //starts with; 1.4.2 wrote zeros over an eighth of it
            let interpreter = prepare(
                "    org $1000
    nop
    org $2000
data: dc.b 1,2
room: ds.b 4
",
            );
            let memory = interpreter.get_memory();
            assert_eq!(memory.read_byte(0x2000).unwrap(), 1);
            assert_eq!(memory.read_byte(0x2001).unwrap(), 2);
            assert_eq!(
                (0x2002..0x2006)
                    .map(|address| memory.read_byte(address).unwrap())
                    .collect::<Vec<u8>>(),
                vec![255, 255, 255, 255]
            );
        }

        #[test]
        fn the_current_location_is_the_line_the_instruction_was_written_on() {
            let mut interpreter = prepare(
                "    org $1000
    nop
    nop
",
            );
            let location = interpreter
                .get_current_location()
                .expect("the pc is on an instruction")
                .clone();
            assert_eq!(location.file, DEFAULT_ENTRY_PATH);
            assert_eq!(location.line, 1);
            interpreter.step().expect("one instruction to run");
            assert_eq!(interpreter.get_current_location().map(|l| l.line), Some(2));
            interpreter.step().expect("one instruction to run");
            assert_eq!(
                interpreter.get_current_location(),
                None,
                "past the last instruction there is no line to point at"
            );
        }

        const THREE_MOVES: &str = "    org $1000
start:
    move.l #1,d0
    move.l #2,d0
    move.l #3,d0
";

        #[test]
        fn a_breakpoint_stops_the_run_on_the_line_it_names() {
            let mut interpreter = prepare(THREE_MOVES);
            let breakpoints = [Breakpoint::new(DEFAULT_ENTRY_PATH, 3)];
            assert_eq!(
                interpreter.get_breakpoint_addresses(&breakpoints),
                [0x1004].into_iter().collect()
            );
            interpreter
                .run_with_breakpoints(&breakpoints, None)
                .expect("to stop at the breakpoint");
            assert_eq!(interpreter.get_pc(), 0x1004);
            assert_eq!(
                interpreter.get_register_value(
                    crate::instructions::RegisterOperand::Data(0),
                    crate::instructions::Size::Long
                ),
                1,
                "the line the breakpoint is on has not run yet"
            );
            //a breakpoint on the line the pc is already on does not stop it again
            let status = interpreter
                .run_with_breakpoints(&breakpoints, None)
                .expect("to run on");
            assert_eq!(status, InterpreterStatus::Terminated);
        }

        #[test]
        fn a_breakpoint_after_simhalt_stops_before_the_resumed_instruction() {
            let mut interpreter = prepare(
                "    org $1000
    move.l #1,d0
    simhalt
    move.l #2,d0
    nop
",
            );
            let breakpoints = [Breakpoint::new(DEFAULT_ENTRY_PATH, 3)];

            assert_eq!(
                interpreter
                    .run_with_breakpoints(&breakpoints, None)
                    .expect("the simhalt"),
                InterpreterStatus::Paused
            );
            assert_eq!(interpreter.get_pc(), 0x1008);

            assert_eq!(
                interpreter
                    .run_with_breakpoints(&breakpoints, None)
                    .expect("the breakpoint after the pause"),
                InterpreterStatus::Running
            );
            assert_eq!(interpreter.get_pc(), 0x1008);
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 1);

            assert_eq!(
                interpreter
                    .run_with_breakpoints(&breakpoints, None)
                    .expect("continue from the breakpoint"),
                InterpreterStatus::Terminated
            );
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 2);
        }

        #[test]
        fn run_with_limit_resumes_after_simhalt() {
            let mut interpreter = prepare(
                "    simhalt
    move.l #9,d0
    nop
",
            );
            assert_eq!(
                interpreter.run_with_limit(3).expect("the pause"),
                InterpreterStatus::Paused
            );
            assert_eq!(
                interpreter.run_with_limit(3).expect("the resumed run"),
                InterpreterStatus::Terminated
            );
            assert_eq!(interpreter.get_cpu().get_register_values()[0], 9);
        }

        #[test]
        fn a_breakpoint_of_another_file_stops_nothing() {
            let mut interpreter = prepare(THREE_MOVES);
            let breakpoints = [Breakpoint::new("other.m68k", 3)];
            assert!(interpreter
                .get_breakpoint_addresses(&breakpoints)
                .is_empty());
            let status = interpreter
                .run_with_breakpoints(&breakpoints, None)
                .expect("to run to the end");
            assert_eq!(status, InterpreterStatus::Terminated);
        }

        #[test]
        fn a_breakpoint_on_a_line_that_assembles_to_nothing_stops_nothing() {
            let interpreter = prepare(THREE_MOVES);
            //line 0 is the `org` and line 1 is the Label on its own
            for line in [0, 1] {
                assert!(interpreter
                    .get_breakpoint_addresses(&[Breakpoint::new(DEFAULT_ENTRY_PATH, line)])
                    .is_empty());
            }
        }

        const A_CALL: &str = "    org $1000
start:
    bsr routine
    move.b #9,d0
    trap #15
routine:
    rts
";

        #[test]
        fn the_call_stack_names_the_label_of_the_routine_and_where_it_was_written() {
            let mut interpreter = with_history(A_CALL);
            interpreter.step().expect("the bsr to run");
            let stack = interpreter.get_pretty_call_stack();
            assert_eq!(stack.len(), 1);
            assert_eq!(stack[0].label_name, "routine");
            assert_eq!(stack[0].label_address, 0x100c);
            let location = stack[0]
                .label_location
                .as_ref()
                .expect("the Label of a routine has a Location");
            assert_eq!(location.file, DEFAULT_ENTRY_PATH);
            assert_eq!(location.line, 5);
            assert_eq!(
                stack[0].source_address, 0x1004,
                "the frame remembers where the call returns to, one past the `bsr`"
            );
        }

        #[test]
        fn undo_puts_the_call_stack_back() {
            let mut interpreter = with_history(A_CALL);
            interpreter.step().expect("the bsr to run");
            assert_eq!(interpreter.get_pc(), 0x100c);
            interpreter.step().expect("the rts to run");
            assert_eq!(interpreter.get_pc(), 0x1004);
            assert!(interpreter.get_pretty_call_stack().is_empty());
            //the return address is one past the instruction that called, and finding that
            //instruction again is what the stored size is for
            interpreter.undo().expect("the rts to be undone");
            let stack = interpreter.get_pretty_call_stack();
            assert_eq!(stack.len(), 1);
            assert_eq!(stack[0].label_name, "routine");
        }

        #[test]
        fn an_undone_step_carries_the_location_of_the_line_that_ran() {
            let mut interpreter = with_history("    org $1000\n    nop\n    nop\n");
            interpreter.step().expect("one instruction to run");
            let step = interpreter.undo().expect("a step to undo");
            let location = step.get_location().expect("a step ran an instruction");
            assert_eq!(location.file, DEFAULT_ENTRY_PATH);
            assert_eq!(location.line, 1);
            assert_eq!(interpreter.get_pc(), 0x1000, "undo goes back to the line");
        }
    }

    /// The status register: `$2700` to start with, its low byte the condition
    /// codes, its high byte stored and of no effect (the design record,
    /// "Instructions").
    mod the_status_register {
        use crate::instructions::{RegisterOperand, Size};
        use crate::interpreter::{Flags, Interpreter, InterpreterOptions, INITIAL_STATUS_REGISTER};
        use crate::test::test::{assemble, prepare};

        fn with_history(code: &str) -> Interpreter {
            Interpreter::new(
                assemble(code),
                Some(InterpreterOptions {
                    keep_history: true,
                    history_size: 100,
                }),
            )
        }

        /// The flags of the fixture's own order: X, N, Z, V, C.
        fn flags(interpreter: &Interpreter) -> String {
            let flags = interpreter.get_flags_as_array();
            format!(
                "X:{} N:{} Z:{} V:{} C:{}",
                flags[4], flags[3], flags[2], flags[1], flags[0]
            )
        }

        #[test]
        fn a_program_starts_in_supervisor_mode_with_the_interrupt_mask_at_seven() {
            let interpreter = prepare(
                "    nop
",
            );
            assert_eq!(interpreter.get_sr(), 0x2700);
            assert_eq!(INITIAL_STATUS_REGISTER, 0x2700);
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");
        }

        #[test]
        fn move_to_ccr_reads_a_word_and_keeps_its_low_byte() {
            // `Reference/68ks4d.htm`: "the lower byte of a word is copied to
            // the flag register", and the flags are the byte moved rather than
            // the result of moving it.
            let mut interpreter = prepare(
                "    move.w #$ff1f,d0
    move.w d0,ccr
",
            );
            interpreter.run().expect("the two moves");
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:1 V:1 C:1");
            assert_eq!(
                interpreter.get_sr(),
                0x271f,
                "the system byte is where it was"
            );
        }

        #[test]
        fn move_to_ccr_of_zero_leaves_the_zero_flag_clear() {
            // The help's own warning: "if you clear the flag register the Z
            // flag won't be set".
            let mut interpreter = prepare(
                "    move.w #$04,d0
    move.w d0,ccr
    move.w #0,ccr
",
            );
            interpreter.run().expect("the three moves");
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");
        }

        #[test]
        fn move_to_sr_writes_the_whole_register_and_move_from_sr_reads_it_back() {
            let mut interpreter = prepare(
                "    move.w #$2705,sr
    move.w sr,d1
    move.w sr,$2000
",
            );
            interpreter.run().expect("the three moves");
            assert_eq!(interpreter.get_sr(), 0x2705);
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:1 V:0 C:1");
            assert_eq!(
                interpreter.get_register_value(RegisterOperand::Data(1), Size::Word),
                0x2705
            );
            assert_eq!(
                interpreter.get_memory().read_word(0x2000).unwrap(),
                0x2705,
                "`move sr,<ea>` writes a word"
            );
        }

        #[test]
        fn move_from_ccr_writes_the_low_byte_as_a_word() {
            let mut interpreter = prepare(
                "    move.w #$1f,ccr
    move.w ccr,d2
",
            );
            interpreter.run().expect("the two moves");
            assert_eq!(
                interpreter.get_register_value(RegisterOperand::Data(2), Size::Word),
                0x1f,
                "the condition codes, zero extended to a word"
            );
        }

        #[test]
        fn andi_ori_and_eori_reach_both_halves_of_the_register() {
            let mut interpreter = prepare(
                "    ori.b #$1f,ccr
    andi.b #$1a,ccr
    eori.b #$02,ccr
    ori.w #$0700,sr
    andi.w #$f0ff,sr
    eori.w #$2000,sr
",
            );
            interpreter.run().expect("the six instructions");
            // $1f & $1a = $1a, ^ $02 = $18: extend and negative.
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:0");
            // The system byte started at $27, `ori` left it there, `andi
            // #$f0ff` cleared the interrupt mask and `eori #$2000` flipped the
            // supervisor bit off.
            assert_eq!(interpreter.get_sr(), 0x0018);
        }

        #[test]
        fn undo_puts_the_whole_status_register_back() {
            let mut interpreter = with_history(
                "    move.w #$2705,sr
    move.w #$0000,sr
",
            );
            interpreter.step().expect("the first move");
            assert_eq!(interpreter.get_sr(), 0x2705);
            interpreter.step().expect("the second move");
            assert_eq!(interpreter.get_sr(), 0x0000);
            interpreter.undo().expect("a step to undo");
            assert_eq!(
                interpreter.get_sr(),
                0x2705,
                "undo restores the system byte and not only the condition codes"
            );
            interpreter.undo().expect("another step to undo");
            assert_eq!(interpreter.get_sr(), 0x2700, "and back to where it started");
        }

        #[test]
        fn the_flag_getters_still_answer_what_they_always_did() {
            let mut interpreter = prepare(
                "    move.w #$04,d0
    move.w d0,ccr
",
            );
            interpreter.run().expect("the two moves");
            assert!(interpreter.get_flag(Flags::Zero));
            assert!(!interpreter.get_flag(Flags::Carry));
            assert_eq!(interpreter.get_flags_as_array(), vec![0, 0, 1, 0, 0]);
        }
    }

    /// `movep`, `tas`, `rtr` and the three instructions that end a run with an
    /// exception.
    mod movep_tas_rtr_and_the_exceptions {
        use crate::instructions::{RegisterOperand, Size};
        use crate::interpreter::{Interpreter, InterpreterStatus, RuntimeError};
        use crate::test::test::prepare;

        /// Runs a program that is expected to stop with an exception, and
        /// answers the Interpreter and the error.
        fn run_expecting_an_exception(code: &str) -> (Interpreter, RuntimeError) {
            let mut interpreter = prepare(code);
            match interpreter.run() {
                Err(error) => {
                    assert_eq!(
                        *interpreter.get_status(),
                        InterpreterStatus::TerminatedWithException,
                        "an exception ends the run"
                    );
                    (interpreter, error)
                }
                Ok(status) => panic!("expected an exception, the run answered {:?}", status),
            }
        }

        #[test]
        fn movep_writes_every_second_byte_most_significant_first() {
            // `Reference/68ks4g.htm`: "The MSB in the data register transfers
            // to or from the address x(An), the next byte to or from x+2(An)
            // and so on."
            let mut interpreter = prepare(
                "    move.l #$12345678,d0
    move.l #$2000,a0
    movep.l d0,0(a0)
    movep.w d0,8(a0)
",
            );
            interpreter.run().expect("the four instructions");
            let memory = interpreter.get_memory();
            assert_eq!(
                (0x2000..0x2008)
                    .map(|address| memory.read_byte(address).unwrap())
                    .collect::<Vec<u8>>(),
                vec![0x12, 0xff, 0x34, 0xff, 0x56, 0xff, 0x78, 0xff],
                "the bytes land at 0, 2, 4 and 6 and nothing between them is touched"
            );
            assert_eq!(
                (0x2008..0x200b)
                    .map(|address| memory.read_byte(address).unwrap())
                    .collect::<Vec<u8>>(),
                vec![0x56, 0xff, 0x78],
                "a word is the low two bytes of the register"
            );
        }

        #[test]
        fn movep_reads_every_second_byte_back() {
            let mut interpreter = prepare(
                "    move.l #$2000,a1
    move.l #$12345678,d0
    movep.l d0,0(a1)
    movep.l 0(a1),d1
    movep.w 4(a1),d2
",
            );
            interpreter.run().expect("the five instructions");
            assert_eq!(
                interpreter.get_register_value(RegisterOperand::Data(1), Size::Long),
                0x12345678
            );
            assert_eq!(
                interpreter.get_register_value(RegisterOperand::Data(2), Size::Word),
                0x5678,
                "a word form writes the low word and leaves the rest of the register"
            );
        }

        #[test]
        fn movep_touches_no_flag() {
            let mut interpreter = prepare(
                "    move.l #$2000,a0
    move.l #$12345678,d0
    move.w #$1f,ccr
    movep.w d0,0(a0)
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(
                interpreter.get_sr() & 0xff,
                0x1f,
                "`movep` leaves the flags alone (`Reference/68ks4g.htm`)"
            );
        }

        #[test]
        fn tas_sets_bit_seven_and_reads_the_flags_from_the_value_before() {
            // `Reference/68ks5w.htm`: N and Z are the byte before the
            // operation, V and C are cleared, and the top bit is set after.
            let mut interpreter = prepare(
                "    move.l #$2000,a0
    move.b #0,(a0)
    tas (a0)
    tas (a0)
",
            );
            interpreter.step().expect("the movea");
            interpreter.step().expect("the move");
            interpreter.step().expect("the first tas");
            assert_eq!(
                interpreter.get_memory().read_byte(0x2000).unwrap(),
                0x80,
                "the top bit is set"
            );
            assert_eq!(
                interpreter.get_flags_as_array(),
                vec![0, 0, 1, 0, 0],
                "the byte was zero, so Z is set and N is not"
            );
            interpreter.step().expect("the second tas");
            assert_eq!(interpreter.get_memory().read_byte(0x2000).unwrap(), 0x80);
            assert_eq!(
                interpreter.get_flags_as_array(),
                vec![0, 0, 0, 1, 0],
                "the byte was $80 this time, so N is set and Z is not"
            );
        }

        #[test]
        fn rtr_pops_the_condition_codes_and_then_the_return_address() {
            // The subroutine keeps its caller's flags on the stack and puts
            // them back on the way out (`Reference/68ks9f.htm`).
            let mut interpreter = prepare(
                "    org $1000
    move.l #7,d1
    move.w #$1f,ccr
    bsr routine
    simhalt
routine:
    move.w sr,-(sp)
    move.w #0,ccr
    rtr
",
            );
            interpreter
                .run()
                .expect("the program to end on the simhalt");
            assert_eq!(
                interpreter.get_flags_as_array(),
                vec![1, 1, 1, 1, 1],
                "`rtr` put the caller's flags back"
            );
            assert_eq!(
                interpreter.get_pc(),
                0x1010,
                "and returned to the `simhalt` after the `bsr`, which is one past it now"
            );
            assert_eq!(
                interpreter.get_register_value(RegisterOperand::Data(1), Size::Long),
                7
            );
            assert_eq!(
                interpreter.get_sp(),
                0x01000000,
                "the stack is back where it started"
            );
        }

        #[test]
        fn chk_says_nothing_when_the_register_is_within_bounds() {
            let mut interpreter = prepare(
                "    move.w #5,d0
    chk #10,d0
",
            );
            interpreter.run().expect("the two instructions");
            assert_eq!(*interpreter.get_status(), InterpreterStatus::Terminated);
        }

        #[test]
        fn chk_ends_the_run_when_the_register_is_above_the_bound() {
            let (interpreter, error) = run_expecting_an_exception(
                "    move.w #11,d0
    chk #10,d0
",
            );
            match error {
                RuntimeError::ChkOutOfBounds { value, bound } => {
                    assert_eq!((value, bound), (11, 10))
                }
                other => panic!("expected a `chk` exception, got {:?}", other),
            }
            assert!(
                !interpreter.get_flag(crate::interpreter::Flags::Negative),
                "N is cleared when the register is above the bound"
            );
        }

        #[test]
        fn chk_ends_the_run_when_the_register_is_negative() {
            let (interpreter, error) = run_expecting_an_exception(
                "    move.w #-1,d0
    chk #10,d0
",
            );
            match error {
                RuntimeError::ChkOutOfBounds { value, bound } => {
                    assert_eq!((value, bound), (-1, 10))
                }
                other => panic!("expected a `chk` exception, got {:?}", other),
            }
            assert!(
                interpreter.get_flag(crate::interpreter::Flags::Negative),
                "N is set when the register is below zero"
            );
        }

        #[test]
        fn trapv_ends_the_run_only_when_the_overflow_flag_is_set() {
            let mut interpreter = prepare(
                "    move.w #1,d0
    trapv
",
            );
            interpreter.run().expect("no overflow, no exception");
            assert_eq!(*interpreter.get_status(), InterpreterStatus::Terminated);

            let (_, error) = run_expecting_an_exception(
                "    move.w #$7fff,d0
    add.w #1,d0
    trapv
",
            );
            assert!(
                matches!(error, RuntimeError::OverflowException),
                "got {error:?}"
            );
        }

        #[test]
        fn illegal_always_ends_the_run() {
            let (interpreter, error) = run_expecting_an_exception(
                "    illegal
    nop
",
            );
            assert!(
                matches!(error, RuntimeError::IllegalInstruction),
                "got {error:?}"
            );
            assert_eq!(
                interpreter.get_pc(),
                interpreter.get_program().entry() + 4,
                "the run stops one past the `illegal`"
            );
        }
    }

    /// The extend-flag and binary-coded-decimal group: `addx`, `subx`, `negx`,
    /// `roxl`, `roxr`, `abcd`, `sbcd` and `nbcd`, with the flag rules of their
    /// reference pages.
    ///
    /// The rule every one of them turns on is the Z flag's: **cleared when the
    /// result is not zero, and left exactly as it was when it is**, so that a
    /// program can test a number of any width by setting Z, working up from the
    /// least significant piece, and reading Z at the end
    /// (`Reference/68ks5e.htm`).
    mod the_extend_flag_and_binary_coded_decimal {
        use crate::instructions::{RegisterOperand, Size};
        use crate::interpreter::Interpreter;
        use crate::test::test::prepare;

        /// The flags in the fixture's own order: X, N, Z, V, C.
        fn flags(interpreter: &Interpreter) -> String {
            let flags = interpreter.get_flags_as_array();
            format!(
                "X:{} N:{} Z:{} V:{} C:{}",
                flags[4], flags[3], flags[2], flags[1], flags[0]
            )
        }

        fn data(interpreter: &Interpreter, register: u8, size: Size) -> u32 {
            interpreter.get_register_value(RegisterOperand::Data(register), size)
        }

        fn address(interpreter: &Interpreter, register: u8) -> u32 {
            interpreter.get_register_value(RegisterOperand::Address(register), Size::Long)
        }

        #[test]
        fn addx_adds_the_extend_flag_as_well() {
            // The help's own example: "ADDX D0,D1 — adds D0 and D1 + X bit".
            let mut interpreter = prepare(
                "    move.w #$10,ccr
    move.l #$00000002,d0
    move.l #$00000003,d1
    addx.l d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Long), 6, "2 + 3 + the X bit");
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");
        }

        #[test]
        fn addx_sets_the_carry_the_negative_and_the_overflow_from_the_result() {
            // A `move` sets N and Z from what it moves, so a program that
            // wants a flag in a particular state before an `addx` writes the
            // condition codes **last**, which is what these three lines do.
            let mut interpreter = prepare(
                "    move.b #$ff,d0
    move.b #$01,d1
    move.w #$00,ccr
    addx.b d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0);
            assert_eq!(
                flags(&interpreter),
                "X:1 N:0 Z:0 V:0 C:1",
                "the byte carried out, and Z is not set although the byte is zero"
            );

            // The extend flag alone can overflow a byte: 127 + 0 + 1 is -128.
            let mut interpreter = prepare(
                "    move.b #$00,d0
    move.b #$7f,d1
    move.w #$10,ccr
    addx.b d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x80);
            assert_eq!(flags(&interpreter), "X:0 N:1 Z:0 V:1 C:0");

            // And it can take an overflow away again: -128 + -1 + 1 is -128,
            // which a byte holds, so V stays clear where `add` would set it.
            let mut interpreter = prepare(
                "    move.b #$80,d0
    move.b #$ff,d1
    move.w #$10,ccr
    addx.b d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x80);
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:1");
        }

        /// The rule an implementation of `addx` usually gets wrong, pinned in
        /// both directions: a 64-bit addition in two longs whose high half
        /// comes out zero must **not** set Z, because the number as a whole is
        /// not zero.
        #[test]
        fn the_zero_flag_is_cleared_by_a_result_that_is_not_zero_and_never_set() {
            // $0000000000000001 + $0000000000000001 = $0000000000000002. The
            // low longs give 2 and clear Z; the high longs give 0 and must
            // leave it clear, because 2 is not zero.
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.l #$00000001,d0
    move.l #$00000000,d1
    move.l #$00000001,d2
    move.l #$00000000,d3
    add.l d2,d0
    addx.l d3,d1
",
            );
            interpreter.run().expect("the seven instructions");
            assert_eq!(data(&interpreter, 0, Size::Long), 2);
            assert_eq!(data(&interpreter, 1, Size::Long), 0);
            assert_eq!(
                flags(&interpreter),
                "X:0 N:0 Z:0 V:0 C:0",
                "the high long came out zero and Z stayed cleared: the 64-bit sum is 2"
            );

            // The other direction: $0000000100000001 + $00000000ffffffff =
            // $0000000200000000. The low longs give zero and set Z; the high
            // longs give 2 and clear it again.
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.l #$00000001,d0
    move.l #$00000001,d1
    move.l #$ffffffff,d2
    move.l #$00000000,d3
    add.l d2,d0
    addx.l d3,d1
",
            );
            interpreter.run().expect("the seven instructions");
            assert_eq!(data(&interpreter, 0, Size::Long), 0);
            assert_eq!(data(&interpreter, 1, Size::Long), 2);
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");

            // And the whole point of the rule: a number that really is zero
            // leaves the Z the program set before it started.
            let mut interpreter = prepare(
                "    move.l #$00000000,d0
    move.l #$00000000,d1
    move.l #$00000000,d2
    move.l #$00000000,d3
    move.w #$04,ccr
    add.l d2,d0
    addx.l d3,d1
",
            );
            interpreter.run().expect("the seven instructions");
            assert_eq!(
                flags(&interpreter),
                "X:0 N:0 Z:1 V:0 C:0",
                "every piece was zero, so Z is still set"
            );
        }

        #[test]
        fn addx_through_memory_walks_both_registers_down() {
            // The multi-precision idiom of the help: set Z, clear X, and work
            // up from the least significant long. The two numbers are
            // $0000000100000001 at $2000 and $00000000ffffffff at $3000, and
            // the sum is $0000000200000000.
            let mut interpreter = prepare(
                "    move.l #$00000001,$2000
    move.l #$00000001,$2004
    move.l #$00000000,$3000
    move.l #$ffffffff,$3004
    move.l #$2008,a0
    move.l #$3008,a1
    move.w #$04,ccr
    addx.l -(a1),-(a0)
    addx.l -(a1),-(a0)
",
            );
            interpreter.run().expect("the nine instructions");
            let memory = interpreter.get_memory();
            assert_eq!(memory.read_long(0x2004).unwrap(), 0x0000_0000);
            assert_eq!(memory.read_long(0x2000).unwrap(), 0x0000_0002);
            assert_eq!(
                address(&interpreter, 0),
                0x2000,
                "`-(a0)` walked down twice"
            );
            assert_eq!(address(&interpreter, 1), 0x3000);
            assert_eq!(
                flags(&interpreter),
                "X:0 N:0 Z:0 V:0 C:0",
                "the high long is 2, so the 64-bit sum is not zero"
            );
        }

        #[test]
        fn subx_takes_the_extend_flag_away_as_well() {
            // The help's own example: "SUBX.B D0,D1 — D1 = D1 - D0 - X".
            let mut interpreter = prepare(
                "    move.w #$10,ccr
    move.b #$03,d0
    move.b #$09,d1
    subx.b d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 5, "9 - 3 - 1");
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");
        }

        #[test]
        fn subx_borrows_into_the_extend_and_the_carry_alike() {
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.w #$0002,d0
    move.w #$0001,d1
    subx.w d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Word), 0xffff, "1 - 2");
            assert_eq!(
                flags(&interpreter),
                "X:1 N:1 Z:0 V:0 C:1",
                "the borrow is in X and in C, and Z was cleared by a result that is not zero"
            );
        }

        #[test]
        fn negx_is_zero_less_the_operand_less_the_extend_flag() {
            // The help's own example: "if D0 contained 2, X = 1, D0 would
            // become 0000FFFD" (`Reference/68ks5q.htm`).
            let mut interpreter = prepare(
                "    move.l #$12340002,d0
    move.w #$10,ccr
    negx.w d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(
                data(&interpreter, 0, Size::Long),
                0x1234_fffd,
                "a word `negx` leaves the rest of the register alone"
            );
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:1");
        }

        #[test]
        fn negx_of_zero_with_no_extend_flag_borrows_nothing() {
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.l #0,d0
    negx.l d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 0, Size::Long), 0);
            assert_eq!(
                flags(&interpreter),
                "X:0 N:0 Z:1 V:0 C:0",
                "nothing was borrowed, and the Z the program set is still there"
            );
        }

        #[test]
        fn roxl_rotates_through_the_extend_flag() {
            // The help's own example: "ROXL.B #1,D0 — if D0.B contained
            // 11110000, X = 1 it would now be 11100001; if X = 0 then
            // 11100000" (`Reference/68ks7g.htm`).
            let mut interpreter = prepare(
                "    move.b #%11110000,d0
    move.w #$10,ccr
    roxl.b #1,d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0b1110_0001);
            assert_eq!(
                flags(&interpreter),
                "X:1 N:1 Z:0 V:0 C:1",
                "the bit that left the byte is in X and in C alike"
            );

            let mut interpreter = prepare(
                "    move.b #%11110000,d0
    move.w #$00,ccr
    roxl.b #1,d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0b1110_0000);
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:1");
        }

        #[test]
        fn roxr_rotates_the_other_way() {
            // "ROXR.B #1,D0 — if D0.B contained 00001111, X = 1 it would now
            // be 10000111; if X = 0 then 00000111" (`Reference/68ks7h.htm`).
            let mut interpreter = prepare(
                "    move.b #%00001111,d0
    move.w #$10,ccr
    roxr.b #1,d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0b1000_0111);
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:1");

            let mut interpreter = prepare(
                "    move.b #%00001111,d0
    move.w #$00,ccr
    roxr.b #1,d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0b0000_0111);
            assert_eq!(flags(&interpreter), "X:1 N:0 Z:0 V:0 C:1");
        }

        #[test]
        fn a_rotation_of_zero_places_leaves_the_extend_flag_and_answers_it_in_the_carry() {
            // "X — the last bit that was rotated from the operand. Unaffected
            // if rotation step was zero", and "C — same as X", which is the one
            // place a rotate's carry is not a bit it moved.
            let mut interpreter = prepare(
                "    move.b #%10000001,d0
    move.b #0,d1
    move.w #$10,ccr
    roxl.b d1,d0
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(
                data(&interpreter, 0, Size::Byte),
                0b1000_0001,
                "nothing moved"
            );
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:1");
        }

        #[test]
        fn a_rotation_by_a_register_goes_round_the_extend_flag_as_well() {
            // Nine places of a byte rotation is the whole nine-bit register: a
            // byte plus the extend flag comes back to where it started.
            let mut interpreter = prepare(
                "    move.b #%10110010,d0
    move.b #9,d1
    move.w #$10,ccr
    roxl.b d1,d0
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0b1011_0010);
            assert_eq!(flags(&interpreter), "X:1 N:1 Z:0 V:0 C:1");
        }

        #[test]
        fn the_memory_form_of_a_rotate_moves_one_word_by_one_place() {
            let mut interpreter = prepare(
                "    move.w #$8001,$2000
    move.l #$2000,a0
    move.w #$00,ccr
    roxl (a0)
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(
                interpreter.get_memory().read_word(0x2000).unwrap(),
                0x0002,
                "the top bit went to X and a zero came in at the bottom"
            );
            assert_eq!(flags(&interpreter), "X:1 N:0 Z:0 V:0 C:1");
        }

        #[test]
        fn abcd_adds_two_decimal_bytes() {
            // "ABCD.B D0,D1 — adds the 2 BCD numbers in D0 and D1 and stores
            // the answer in D1" (`Reference/68ks8e.htm`). 25 + 17 = 42, and
            // not $3c.
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.b #$25,d0
    move.b #$17,d1
    abcd d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x42);
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");
        }

        #[test]
        fn abcd_carries_out_of_ninety_nine_into_the_extend_flag() {
            let mut interpreter = prepare(
                "    move.b #$99,d0
    move.b #$01,d1
    move.w #$04,ccr
    abcd d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x00, "100, in one byte");
            assert_eq!(
                flags(&interpreter),
                "X:1 N:0 Z:1 V:0 C:1",
                "the hundred is in X and C, and the Z the program set survives a zero result"
            );
        }

        #[test]
        fn abcd_through_memory_adds_a_number_of_any_length() {
            // 123456 + 000045 = 123501, three bytes at a time, starting at the
            // least significant end, which is what `-(An)` is for.
            let mut interpreter = prepare(
                "    move.b #$12,$2000
    move.b #$34,$2001
    move.b #$56,$2002
    move.b #$00,$3000
    move.b #$00,$3001
    move.b #$45,$3002
    move.l #$2003,a1
    move.l #$3003,a0
    move.w #$04,ccr
    abcd -(a0),-(a1)
    abcd -(a0),-(a1)
    abcd -(a0),-(a1)
",
            );
            interpreter.run().expect("the twelve instructions");
            let memory = interpreter.get_memory();
            assert_eq!(
                (0x2000..0x2003)
                    .map(|address| memory.read_byte(address).unwrap())
                    .collect::<Vec<u8>>(),
                vec![0x12, 0x35, 0x01],
                "123456 + 45 = 123501"
            );
            assert_eq!(address(&interpreter, 0), 0x3000);
            assert_eq!(address(&interpreter, 1), 0x2000);
            assert_eq!(
                flags(&interpreter),
                "X:0 N:0 Z:0 V:0 C:0",
                "the answer is not zero, so Z was cleared on the way"
            );
        }

        #[test]
        fn sbcd_subtracts_two_decimal_bytes() {
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.b #$17,d0
    move.b #$25,d1
    sbcd d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x08, "25 - 17 = 08");
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:0 V:0 C:0");

            // And a borrow: 17 - 25 is 92 with a loan of a hundred, which is
            // the X flag the next `sbcd` up takes away.
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.b #$25,d0
    move.b #$17,d1
    sbcd d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x92);
            assert_eq!(flags(&interpreter), "X:1 N:0 Z:0 V:0 C:1");
        }

        #[test]
        fn nbcd_is_the_tens_complement() {
            // "The tens complement to 01 is 99 (1+99=100), to 26 is 74"
            // (`Reference/68ks8f.htm`).
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.b #$01,d0
    move.b #$26,d1
    nbcd d0
    nbcd d1
",
            );
            interpreter.step().expect("the move to ccr");
            interpreter.step().expect("a move");
            interpreter.step().expect("a move");
            interpreter.step().expect("the first nbcd");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0x99);
            assert_eq!(
                flags(&interpreter),
                "X:1 N:0 Z:0 V:0 C:1",
                "a hundred was borrowed"
            );
            interpreter.step().expect("the second nbcd");
            assert_eq!(
                data(&interpreter, 1, Size::Byte),
                0x73,
                "the second `nbcd` takes the borrow of the first away as well: 74 - 1"
            );
        }

        #[test]
        fn nbcd_of_zero_borrows_nothing() {
            let mut interpreter = prepare(
                "    move.w #$04,ccr
    move.b #$00,d0
    nbcd d0
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 0, Size::Byte), 0x00);
            assert_eq!(
                flags(&interpreter),
                "X:0 N:0 Z:1 V:0 C:0",
                "nothing was borrowed, and the Z the program set is still there"
            );
        }

        #[test]
        fn undo_puts_a_memory_pair_back_the_way_it_was() {
            // Nothing here is special-cased for undo: the two address
            // registers and the byte in memory are written through the same
            // setters every other instruction uses, and the condition codes
            // come back with the step. The test is here because a predecrement
            // pair writes three things at once, which is as many as anything
            // in the crate does.
            let mut interpreter = Interpreter::new(
                crate::test::test::assemble(
                    "    move.b #$56,$2002
    move.b #$45,$3002
    move.l #$2003,a1
    move.l #$3003,a0
    move.w #$04,ccr
    abcd -(a0),-(a1)
",
                ),
                Some(crate::interpreter::InterpreterOptions {
                    keep_history: true,
                    history_size: 100,
                }),
            );
            for _ in 0..5 {
                interpreter.step().expect("the six instructions");
            }
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:1 V:0 C:0");
            interpreter.step().expect("the abcd");
            assert_eq!(interpreter.get_memory().read_byte(0x2002).unwrap(), 0x01);
            assert_eq!(flags(&interpreter), "X:1 N:0 Z:0 V:0 C:1");
            interpreter.undo().expect("a step to undo");
            assert_eq!(
                interpreter.get_memory().read_byte(0x2002).unwrap(),
                0x56,
                "the byte is back"
            );
            assert_eq!(address(&interpreter, 0), 0x3003, "and both registers");
            assert_eq!(address(&interpreter, 1), 0x2003);
            assert_eq!(flags(&interpreter), "X:0 N:0 Z:1 V:0 C:0");
        }

        #[test]
        fn the_decimal_instructions_leave_the_flags_the_help_calls_undefined() {
            // N and V are undefined for all three (`Reference/68ks8e.htm`,
            // `68ks8g.htm`, `68ks8f.htm`), and s68k leaves an undefined flag
            // exactly where it was rather than inventing a value for it.
            let mut interpreter = prepare(
                "    move.b #$25,d0
    move.b #$17,d1
    move.w #$0e,ccr
    abcd d0,d1
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(data(&interpreter, 1, Size::Byte), 0x42);
            assert_eq!(
                flags(&interpreter),
                "X:0 N:1 Z:0 V:1 C:0",
                "N and V are where the `move` to `ccr` put them"
            );
        }
    }

    /// The PC-relative modes, and the round trip that makes them work here.
    ///
    /// The source writes the address it wants, the Assembler stores the
    /// distance from this instruction's extension word to it, and the
    /// Interpreter adds the two back together: every test below is that pair
    /// agreeing, since a program that read the wrong place would fail on the
    /// value rather than on the arithmetic.
    mod pc_relative_addressing {
        use crate::instructions::{RegisterOperand, Size};
        use crate::test::test::prepare;

        fn data(interpreter: &crate::interpreter::Interpreter, register: u8) -> u32 {
            interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
        }

        fn address(interpreter: &crate::interpreter::Interpreter, register: u8) -> u32 {
            interpreter.get_register_value(RegisterOperand::Address(register), Size::Long)
        }

        #[test]
        fn a_pc_relative_read_reaches_the_address_the_source_wrote() {
            // Five instructions from `$1000`, so `value` is at `$1014`; the
            // first one is at `$1000` and its displacement is therefore
            // `$1014 - $1002`, which is 18.
            let mut interpreter = prepare(
                "    move.l value(pc),d0
    move.l (value,pc),d1
    lea value(pc),a0
    move.l (a0),d2
    nop
value: dc.l $12345678
",
            );
            interpreter.run().expect("the five instructions");
            assert_eq!(data(&interpreter, 0), 0x12345678);
            assert_eq!(
                data(&interpreter, 1),
                0x12345678,
                "`(value,pc)` is the same operand written the other way"
            );
            assert_eq!(address(&interpreter, 0), 0x1014, "`lea` gives the address");
            assert_eq!(data(&interpreter, 2), 0x12345678);
        }

        #[test]
        fn a_pc_relative_operand_reaches_backwards_as_well() {
            let mut interpreter = prepare(
                "value: dc.l $0badf00d
    move.l value(pc),d0
",
            );
            interpreter.run().expect("the one instruction");
            assert_eq!(data(&interpreter, 0), 0x0badf00d);
        }

        /// The base is the address of the instruction being executed, not the
        /// program counter, which has already stepped past it: two identical
        /// lines at two addresses read the same place.
        #[test]
        fn the_distance_is_measured_from_the_instruction_that_holds_it() {
            let mut interpreter = prepare(
                "    move.l value(pc),d0
    move.l value(pc),d1
    nop
    nop
    move.l value(pc),d2
value: dc.l $11223344
",
            );
            interpreter.run().expect("the five instructions");
            assert_eq!(data(&interpreter, 0), 0x11223344);
            assert_eq!(data(&interpreter, 1), 0x11223344);
            assert_eq!(data(&interpreter, 2), 0x11223344);
        }

        #[test]
        fn a_pc_relative_index_adds_the_register() {
            let mut interpreter = prepare(
                "    move.w #4,d1
    move.l table(pc,d1.w),d0
    nop
table: dc.l $aaaaaaaa,$bbbbbbbb
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(
                data(&interpreter, 0),
                0xbbbbbbbb,
                "`table` plus the four in `d1` is the second long"
            );
            // `(pc,d1.w)` writes no address at all, so its displacement is
            // zero and the operand is the extension word of its own line plus
            // the register: `$1004 + 2 + 6` is the `dc.l` at `$100c`.
            let mut interpreter = prepare(
                "    move.w #6,d1
    move.l (pc,d1.w),d2
    nop
table: dc.l $00c0ffee
",
            );
            interpreter.run().expect("the three instructions");
            assert_eq!(data(&interpreter, 2), 0x00c0ffee);
        }

        /// `movem` reads registers back through a PC-relative operand and
        /// never writes them out through one, which is the one place its two
        /// directions differ by more than the side the list is on.
        #[test]
        fn movem_reads_registers_back_through_a_pc_relative_operand() {
            let mut interpreter = prepare(
                "    movem.l table(pc),d0-d2
    nop
table: dc.l $11111111,$22222222,$33333333
",
            );
            interpreter.run().expect("the two instructions");
            assert_eq!(data(&interpreter, 0), 0x11111111);
            assert_eq!(data(&interpreter, 1), 0x22222222);
            assert_eq!(data(&interpreter, 2), 0x33333333);
        }

        #[test]
        fn jmp_and_jsr_take_a_pc_relative_operand() {
            let mut interpreter = prepare(
                "    jsr routine(pc)
    jmp done(pc)
    move.l #$dead,d0
routine:
    move.l #1,d1
    rts
done:
    move.l #2,d2
",
            );
            interpreter.run().expect("the program");
            assert_eq!(data(&interpreter, 1), 1, "the subroutine ran");
            assert_eq!(data(&interpreter, 2), 2, "and the jump landed");
            assert_eq!(data(&interpreter, 0), 0, "the line between them did not");
        }

        /// `pea` pushes the address a PC-relative operand names, which is what
        /// makes a position-independent call to a string work.
        #[test]
        fn pea_pushes_the_address_and_not_the_displacement() {
            let mut interpreter = prepare(
                "    pea greeting(pc)
    move.l (a7)+,a1
    move.b (a1),d0
    nop
greeting: dc.b 'hi',0
",
            );
            interpreter.run().expect("the four instructions");
            assert_eq!(address(&interpreter, 1), 0x1010);
            assert_eq!(data(&interpreter, 0) & 0xff, u32::from(b'h'));
        }
    }

    mod traps {
        use crate::instructions::{
            Interrupt, InterruptResult, KeyStateRequest, KeyStateResult, RegisterOperand, Size,
        };
        use crate::test::test::{run_answering, run_expecting_error};

        fn data_long(interpreter: &crate::interpreter::Interpreter, register: u8) -> u32 {
            interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
        }

        #[test]
        fn trap_7_answers_pending_keyboard_input_in_d1_byte() {
            let (interpreter, interrupts) = run_answering(
                "move.l #$FFFFFFFF, d1
    move.b #7, d0
    trap #15",
                |_| InterruptResult::CheckKeyboardInput(true),
            );
            assert!(matches!(interrupts[0], Interrupt::CheckKeyboardInput));
            //only the byte carries the answer
            assert_eq!(data_long(&interpreter, 1), 0xFFFFFF01);

            let (interpreter, _) = run_answering(
                "move.b #7, d0
    trap #15",
                |_| InterruptResult::CheckKeyboardInput(false),
            );
            assert_eq!(data_long(&interpreter, 1), 0);
        }

        #[test]
        fn trap_19_reads_four_key_codes_and_answers_with_ff_bytes() {
            let (interpreter, interrupts) = run_answering(
                "move.b #19, d0
    move.l #$01020304, d1
    trap #15",
                |_| InterruptResult::GetKeyState(KeyStateResult::Keys([true, false, true, false])),
            );
            match &interrupts[0] {
                Interrupt::GetKeyState(KeyStateRequest::Keys(keys)) => {
                    assert_eq!(keys, &[1, 2, 3, 4])
                }
                other => panic!("Expected a key request, got {:?}", other),
            }
            assert_eq!(data_long(&interpreter, 1), 0xFF00FF00);
        }

        #[test]
        fn trap_19_with_zero_asks_for_the_last_keys() {
            let (interpreter, interrupts) = run_answering(
                "move.b #19, d0
    trap #15",
                |_| {
                    InterruptResult::GetKeyState(KeyStateResult::LastKeys {
                        up: 0x1B,
                        down: 0x41,
                    })
                },
            );
            assert!(matches!(
                interrupts[0],
                Interrupt::GetKeyState(KeyStateRequest::LastKeys)
            ));
            //the released key sits in the upper word, the pressed key in the lower word
            assert_eq!(data_long(&interpreter, 1), 0x001B0041);
        }

        #[test]
        fn trap_61_reads_the_mouse_mode_and_answers_flags_and_position() {
            let (interpreter, interrupts) = run_answering(
                "move.b #61, d0
    move.b #2, d1
    trap #15",
                |_| InterruptResult::ReadMouse {
                    flags: 0x25,
                    x: 100,
                    y: 200,
                },
            );
            assert!(matches!(interrupts[0], Interrupt::ReadMouse(2)));
            assert_eq!(data_long(&interpreter, 0), 0x25);
            assert_eq!(data_long(&interpreter, 1), 0x00C80064);
        }

        #[test]
        fn trap_61_rejects_unknown_modes() {
            let error = run_expecting_error(
                "move.b #61, d0
    move.b #3, d1
    trap #15",
            );
            assert!(
                error.contains("Invalid mouse read mode: 3"),
                "Unexpected error: {}",
                error
            );
        }

        #[test]
        fn trap_92_accepts_only_the_supported_drawing_modes() {
            for mode in [2u8, 4, 16, 17] {
                let (_, interrupts) = run_answering(
                    &format!(
                        "move.b #92, d0
    move.b #{}, d1
    trap #15",
                        mode
                    ),
                    |_| InterruptResult::SetDrawingMode,
                );
                assert!(matches!(interrupts[0], Interrupt::SetDrawingMode(m) if m == mode));
            }
            let error = run_expecting_error(
                "move.b #92, d0
    move.b #14, d1
    trap #15",
            );
            assert!(
                error.contains("Unsupported drawing mode: 14"),
                "Unexpected error: {}",
                error
            );
        }

        #[test]
        fn trap_94_repaints() {
            let (_, interrupts) = run_answering(
                "move.b #94, d0
    trap #15",
                |_| InterruptResult::Repaint,
            );
            assert!(matches!(interrupts[0], Interrupt::Repaint));
        }

        #[test]
        fn trap_96_answers_the_pen_position_in_d1_and_d2_words() {
            let (interpreter, interrupts) = run_answering(
                "move.l #$FFFFFFFF, d1
    move.l #$FFFFFFFF, d2
    move.b #96, d0
    trap #15",
                |_| InterruptResult::GetPenPosition(10, 20),
            );
            assert!(matches!(interrupts[0], Interrupt::GetPenPosition));
            assert_eq!(data_long(&interpreter, 1), 0xFFFF000A);
            assert_eq!(data_long(&interpreter, 2), 0xFFFF0014);
        }

        #[test]
        fn drawing_tasks_read_their_coordinates_as_signed_words() {
            //EASy68K casts every drawing coordinate to a short before it draws
            //(`simIO->rectangle((short)D[1], ...)`), so a shape may start off the left or the top
            //of the screen and have the part that is on it clipped. Read unsigned, -20 would
            //arrive as 65516 and the shape would land on the far side of the screen instead.
            let (_, interrupts) = run_answering(
                "move.b #87, d0
    move.w #-20, d1
    move.w #-10, d2
    move.w #40, d3
    move.w #50, d4
    trap #15",
                |_| InterruptResult::DrawRectangle,
            );
            assert!(matches!(
                interrupts[0],
                Interrupt::DrawRectangle(-20, -10, 40, 50)
            ));
        }

        #[test]
        fn trap_96_answers_a_pen_position_off_the_screen() {
            let (interpreter, _) = run_answering(
                "move.l #$FFFFFFFF, d1
    move.l #$FFFFFFFF, d2
    move.b #96, d0
    trap #15",
                |_| InterruptResult::GetPenPosition(-20, 100),
            );
            //the low word of each, which is what a signed short leaves in D1.W and D2.W
            assert_eq!(data_long(&interpreter, 1), 0xFFFFFFEC);
            assert_eq!(data_long(&interpreter, 2), 0xFFFF0064);
        }

        #[test]
        fn trap_20_reads_the_number_and_the_field_width() {
            let (_, interrupts) = run_answering(
                "move.b #20, d0
    move.l #$FFFFFFFB, d1
    move.b #8, d2
    trap #15",
                |_| InterruptResult::DisplaySignedNumberInField,
            );
            match &interrupts[0] {
                Interrupt::DisplaySignedNumberInField { value, width } => {
                    assert_eq!(*value, -5);
                    assert_eq!(*width, 8);
                }
                other => panic!("Expected a number in a field, got {:?}", other),
            }
        }

        #[test]
        fn trap_17_composes_a_string_and_a_number() {
            let (_, interrupts) = run_answering(
                "ORG $1000
message: dc.b 'total ', 0, 0
start:
    move.l #message, a1
    move.l #42, d1
    move.b #17, d0
    trap #15",
                |_| InterruptResult::DisplayStringAndNumber,
            );
            match &interrupts[0] {
                Interrupt::DisplayStringAndNumber { string, number } => {
                    assert_eq!(string, "total ");
                    assert_eq!(*number, 42);
                }
                other => panic!("Expected a string and a number, got {:?}", other),
            }
        }

        #[test]
        fn trap_18_composes_a_string_and_a_number_read() {
            let (interpreter, interrupts) = run_answering(
                "ORG $1000
message: dc.b 'age? ', 0
start:
    move.l #message, a1
    move.b #18, d0
    trap #15",
                |_| InterruptResult::DisplayStringAndReadNumber(-3),
            );
            match &interrupts[0] {
                Interrupt::DisplayStringAndReadNumber(string) => assert_eq!(string, "age? "),
                other => panic!("Expected a string and a number read, got {:?}", other),
            }
            assert_eq!(data_long(&interpreter, 1), 0xFFFFFFFD);
        }

        #[test]
        fn trap_24_carries_the_shortcut_request() {
            let (_, interrupts) = run_answering(
                "move.b #24, d0
    move.l #1, d1
    trap #15",
                |_| InterruptResult::SetSimulatorShortcuts,
            );
            assert!(matches!(interrupts[0], Interrupt::SetSimulatorShortcuts(1)));
        }

        #[test]
        fn trap_33_sets_the_screen_size_from_the_packed_request() {
            let (_, interrupts) = run_answering(
                "move.b #33, d0
    move.l #$03200258, d1
    trap #15",
                |_| InterruptResult::SetScreenSize,
            );
            assert!(matches!(interrupts[0], Interrupt::SetScreenSize(800, 600)));
        }

        #[test]
        fn trap_33_with_zero_answers_the_current_size() {
            let (interpreter, interrupts) = run_answering(
                "move.b #33, d0
    trap #15",
                |_| InterruptResult::GetScreenSize(640, 480),
            );
            assert!(matches!(interrupts[0], Interrupt::GetScreenSize));
            assert_eq!(data_long(&interpreter, 1), 0x028001E0);
        }

        #[test]
        fn trap_33_accepts_the_windowed_and_full_screen_requests() {
            for mode in [1u8, 2] {
                let (_, interrupts) = run_answering(
                    &format!(
                        "move.b #33, d0
    move.l #{}, d1
    trap #15",
                        mode
                    ),
                    |_| InterruptResult::SetScreenMode,
                );
                assert!(matches!(interrupts[0], Interrupt::SetScreenMode(m) if m == mode));
            }
        }

        #[test]
        fn trap_11_clears_only_for_ff00() {
            let (_, interrupts) = run_answering(
                "move.b #11, d0
    move.w #$FF00, d1
    trap #15",
                |_| InterruptResult::ClearScreen,
            );
            assert!(matches!(interrupts[0], Interrupt::ClearScreen));
        }

        #[test]
        fn trap_11_sets_and_gets_the_text_cursor() {
            let (_, interrupts) = run_answering(
                "move.b #11, d0
    move.w #$0305, d1
    trap #15",
                |_| InterruptResult::SetTextCursorPosition,
            );
            //column in the high byte, row in the low byte
            assert!(matches!(
                interrupts[0],
                Interrupt::SetTextCursorPosition(3, 5)
            ));

            let (interpreter, interrupts) = run_answering(
                "move.b #11, d0
    move.w #$00FF, d1
    trap #15",
                |_| InterruptResult::GetTextCursorPosition(10, 20),
            );
            assert!(matches!(interrupts[0], Interrupt::GetTextCursorPosition));
            assert_eq!(data_long(&interpreter, 1), 0x0A14);
        }

        #[test]
        fn unknown_tasks_still_fail() {
            let error = run_expecting_error(
                "move.b #99, d0
    trap #15",
            );
            assert!(
                error.contains("Unknown interrupt: 99"),
                "Unexpected error: {}",
                error
            );
        }
    }
}

/// Assembles `code`, panicking with every error the Assembler found if it does
/// not build.
fn assemble(code: &str) -> Program {
    let assembly = crate::assembler::assemble_source(code);
    match assembly.program {
        Some(program) => program,
        None => {
            let errors: Vec<String> = assembly
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .map(|diagnostic| {
                    format!(
                        "line {}: {} [{}]",
                        diagnostic.location.line + 1,
                        diagnostic.message(),
                        diagnostic.code()
                    )
                })
                .collect();
            panic!("Code did not assemble:\n{}", errors.join("\n"))
        }
    }
}

/// Assembles `code` and runs it to the end, answering every interrupt from the
/// terminal.
fn assemble_and_run(code: &str) -> Interpreter {
    let mut interpreter = prepare(code);
    while !interpreter.has_terminated() {
        let status = interpreter.run().unwrap();
        match status {
            InterpreterStatus::Interrupt => {
                let interrupt = interpreter.get_current_interrupt().unwrap();
                handle_interrupt(&mut interpreter, &interrupt);
            }
            InterpreterStatus::TerminatedWithException => {
                panic!("Program Terminated with exception");
            }
            _ => {}
        }
    }
    interpreter
}

/// Assembles and runs a program, answering every interrupt with the given
/// closure, and returns the interrupts it raised.
fn run_answering(
    code: &str,
    mut answer: impl FnMut(&Interrupt) -> InterruptResult,
) -> (Interpreter, Vec<Interrupt>) {
    let mut interpreter = prepare(code);
    let mut interrupts = Vec::new();
    while !interpreter.has_terminated() {
        let status = interpreter.run().unwrap();
        if status == InterpreterStatus::Interrupt {
            let interrupt = interpreter.get_current_interrupt().unwrap();
            let result = answer(&interrupt);
            interrupts.push(interrupt);
            interpreter.answer_interrupt(result).unwrap();
        }
    }
    (interpreter, interrupts)
}

/// Assembles and runs a program that is expected to stop with a runtime error,
/// and returns its message.
fn run_expecting_error(code: &str) -> String {
    let mut interpreter = prepare(code);
    match interpreter.run() {
        Err(RuntimeError::Raw(message)) => message,
        other => panic!("Expected a runtime error, got: {:?}", other),
    }
}

/// An Interpreter ready to run `code`, with no history kept.
fn prepare(code: &str) -> Interpreter {
    let options = InterpreterOptions {
        keep_history: false,
        ..Default::default()
    };
    Interpreter::new(assemble(code), Some(options))
}

/// Answers one interrupt from the terminal, the way the command line does.
fn handle_interrupt(interpreter: &mut Interpreter, interrupt: &Interrupt) {
    match interrupt {
        Interrupt::DisplayNumber(number) => {
            print!("{}", number);
            interpreter
                .answer_interrupt(InterruptResult::DisplayNumber)
                .unwrap();
        }
        Interrupt::DisplayStringWithCRLF(string) => {
            println!("{}", string);
            interpreter
                .answer_interrupt(InterruptResult::DisplayStringWithCRLF)
                .unwrap();
        }
        Interrupt::DisplayStringWithoutCRLF(string) => {
            print!("{}", string);
            interpreter
                .answer_interrupt(InterruptResult::DisplayStringWithoutCRLF)
                .unwrap();
        }
        Interrupt::GetTime => {
            interpreter
                .answer_interrupt(InterruptResult::GetTime(0))
                .unwrap();
        }
        Interrupt::DisplayChar(char) => {
            print!("{}", char);
            interpreter
                .answer_interrupt(InterruptResult::DisplayChar)
                .unwrap();
        }
        Interrupt::ReadChar => {
            let char = Term::stdout().read_char().expect("Unable to read char");
            interpreter
                .answer_interrupt(InterruptResult::ReadChar(char))
                .unwrap();
        }
        Interrupt::ReadNumber => {
            let num = Term::stdout().read_line().expect("Unable to read line");
            let num = num.trim().parse::<i32>().expect("Unable to parse number");
            interpreter
                .answer_interrupt(InterruptResult::ReadNumber(num))
                .unwrap();
        }
        Interrupt::ReadKeyboardString => {
            let string = Term::stdout().read_line().expect("Unable to read line");
            interpreter
                .answer_interrupt(InterruptResult::ReadKeyboardString(string))
                .unwrap();
        }
        Interrupt::Terminate => {
            interpreter
                .answer_interrupt(InterruptResult::Terminate)
                .unwrap();
        }
        Interrupt::Delay(_) => {
            interpreter
                .answer_interrupt(InterruptResult::Delay)
                .unwrap();
        }
        _ => {
            println!("Unhandled interrupt: {:?}", interrupt);
            interpreter
                .answer_interrupt(InterruptResult::Terminate)
                .unwrap();
        }
    }
}
