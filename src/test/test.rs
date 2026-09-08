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
        fn the_run_ends_after_the_last_instruction_and_not_on_it() {
            let mut interpreter = prepare("    org $1000\n    nop\n");
            assert!(!interpreter.has_reached_bottom());
            interpreter.step().expect("one instruction to run");
            assert_eq!(interpreter.get_pc(), 0x1004, "the nop is 4 bytes wide");
            assert!(interpreter.has_reached_bottom());
            assert!(interpreter.has_terminated());
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
