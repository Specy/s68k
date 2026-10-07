//! The `trap #15` tasks that reach past the text and the screen: the input
//! settings of tasks 12 and 16, the files of tasks 50 to 59 and the sound of
//! tasks 70 to 77; and, for every task, how one fails, what an answer has to be
//! to be taken, and why a program ended.
//!
//! The rules are EASy68K 5.16.1's (`CODE9.CPP`, `SIMOPS2.CPP`, `simIOu.cpp`,
//! `STARTSIM.CPP`), and a test of a rule s68k departs from says so.

use std::collections::BTreeSet;

use crate::assembler::program::Program;
use crate::debugger::MutationOperation;
use crate::instructions::{
    Bytes, FileDialogMode, FileExistence, InputSettings, Interrupt, InterruptResult,
    KeyStateRequest, KeyStateResult, OpenedFile, RegisterOperand, Size,
};
use crate::interpreter::{
    Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, Termination,
};

fn assemble(code: &str) -> Program {
    let assembly = crate::assembler::assemble_source(code);
    match assembly.program {
        Some(program) => program,
        None => panic!("did not assemble: {:?}", assembly.diagnostics),
    }
}

/// An Interpreter for `code` that keeps no history.
fn prepare(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    )
}

/// An Interpreter for `code` that keeps a history, so that undo can be tested.
fn with_history(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 100,
        }),
    )
}

/// `interpreter`, run to the interrupt its program raises, and that interrupt.
fn run_to_interrupt(interpreter: &mut Interpreter) -> Interrupt {
    assert_eq!(
        interpreter.run().expect("the program runs to its trap"),
        InterpreterStatus::Interrupt
    );
    interpreter
        .get_current_interrupt()
        .expect("an interrupt waits")
}

/// The Interpreter of `code` waiting on its first interrupt, and that interrupt.
fn waiting(code: &str) -> (Interpreter, Interrupt) {
    let mut interpreter = prepare(code);
    let interrupt = run_to_interrupt(&mut interpreter);
    (interpreter, interrupt)
}

/// The Interpreter of `code` once its first interrupt has been given `answer`.
fn answered(code: &str, answer: InterruptResult) -> Interpreter {
    let (mut interpreter, _) = waiting(code);
    interpreter.answer_interrupt(answer).expect("the answer");
    interpreter
}

fn data(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn bytes_at(interpreter: &Interpreter, address: usize, length: usize) -> Vec<u8> {
    interpreter
        .get_memory()
        .read_bytes(address, length)
        .expect("memory to read back")
        .to_vec()
}

/// A program that puts `setup` in place and then runs task `task`, with a
/// marker in the high word of D0 so that a test sees D0.W is all a result
/// writes.
fn task(task: u8, setup: &str) -> String {
    format!(
        "    ORG $1000
start:
{setup}
    move.l #$ABCD0000,d0
    move.b #{task},d0
    trap #15
    nop
path:   dc.b 'data\\in.txt',0
other:  dc.b 'caf',$E9,'.txt',0
filter: dc.b '*.txt',0
title:  dc.b 'Load',0
buffer: dcb.b 300,$55
"
    )
}

/// D0.W, where every file task writes its result.
fn result(interpreter: &Interpreter) -> u32 {
    let d0 = data(interpreter, 0);
    assert_eq!(d0 >> 16, 0xABCD, "only D0.W is written");
    d0 & 0xFFFF
}

fn assert_ended_with(interpreter: &Interpreter, error: &RuntimeError) {
    assert_eq!(
        *interpreter.get_status(),
        InterpreterStatus::TerminatedWithException
    );
    assert!(interpreter.has_terminated());
    assert_eq!(
        interpreter.get_termination(),
        Some(&Termination::Exception(error.clone()))
    );
}

mod input_settings {
    use super::*;

    const ON: InputSettings = InputSettings {
        echo: true,
        prompt: true,
        line_feed: true,
    };

    #[test]
    fn every_run_starts_with_the_echo_the_prompt_and_the_line_feed_on() {
        //`initSim` sets all three, whatever the run before it did
        assert_eq!(prepare("    nop").get_input_settings(), ON);
        assert_eq!(InputSettings::default(), ON);
    }

    #[test]
    fn task_12_turns_the_echo_off_for_zero_and_on_for_anything_else() {
        let settings = |d1: &str| {
            let mut interpreter = prepare(&format!(
                "    move.l #{d1},d1\n    move.b #12,d0\n    trap #15\n    nop"
            ));
            interpreter.step().expect("the move");
            interpreter.step().expect("the move");
            interpreter.step().expect("the trap");
            interpreter.get_input_settings()
        };
        assert!(!settings("0").echo);
        //D1.B, so a zero byte under other bytes is still off
        assert!(!settings("$FF00").echo);
        assert!(settings("1").echo);
        assert!(settings("$80").echo);
    }

    #[test]
    fn task_16_turns_the_prompt_and_the_line_feed_off_and_on() {
        let after = |values: &[u8]| {
            let program: String = values
                .iter()
                .map(|value| format!("    move.b #{value},d1\n    move.b #16,d0\n    trap #15\n"))
                .collect();
            let mut interpreter = prepare(&(program + "    nop"));
            interpreter.run().expect("the settings raise no interrupt");
            interpreter.get_input_settings()
        };
        assert_eq!(
            after(&[0]),
            InputSettings {
                prompt: false,
                ..ON
            }
        );
        assert_eq!(after(&[0, 1]), ON);
        assert_eq!(
            after(&[2]),
            InputSettings {
                line_feed: false,
                ..ON
            }
        );
        assert_eq!(after(&[2, 3]), ON);
        //each value changes its own setting and nothing else
        assert_eq!(
            after(&[0, 2]),
            InputSettings {
                echo: true,
                prompt: false,
                line_feed: false
            }
        );
    }

    #[test]
    fn task_16_refuses_any_other_value() {
        //EASy68K does nothing for 4 and up; s68k stops and says why, as for task 15's base
        let mut interpreter = prepare("    move.b #4,d1\n    move.b #16,d0\n    trap #15\n    nop");
        let error = interpreter.run().expect_err("an error");
        match &error {
            RuntimeError::InvalidTrapArgument { task: 16, reason } => {
                assert!(reason.starts_with("D1.B is 4,"), "{}", reason)
            }
            other => panic!("unexpected {:?}", other),
        }
        assert_ended_with(&interpreter, &error);
        assert_eq!(interpreter.get_input_settings(), ON, "and changed nothing");
    }

    #[test]
    fn the_settings_raise_no_interrupt() {
        let mut interpreter = prepare(
            "    move.b #0,d1\n    move.b #12,d0\n    trap #15\n    move.l #7,d2\n    simhalt",
        );
        assert_eq!(
            interpreter.run().expect("the run"),
            InterpreterStatus::Paused,
            "the run goes past the trap to the halt"
        );
        assert_eq!(data(&interpreter, 2), 7);
        assert!(interpreter.get_current_interrupt().is_err());
    }

    #[test]
    fn undo_puts_the_settings_back() {
        let mut interpreter = with_history(
            "    move.b #0,d1\n    move.b #12,d0\n    trap #15\n    move.b #2,d1\n    move.b #16,d0\n    trap #15\n    nop",
        );
        for _ in 0..6 {
            interpreter.step().expect("a step");
        }
        assert_eq!(
            interpreter.get_input_settings(),
            InputSettings {
                echo: false,
                prompt: true,
                line_feed: false
            }
        );
        let step = interpreter.undo().expect("the second trap");
        match step.get_mutations().as_slice() {
            [MutationOperation::SetInputSettings { old, new }] => {
                assert!(old.line_feed && !new.line_feed);
            }
            other => panic!("the trap journals its setting, got {:?}", other),
        }
        assert!(interpreter.get_input_settings().line_feed);
        assert!(!interpreter.get_input_settings().echo);
        interpreter.undo().expect("a move");
        interpreter.undo().expect("a move");
        interpreter.undo().expect("the first trap");
        assert_eq!(interpreter.get_input_settings(), ON, "back to the start");
        //and stepping again sets them again
        interpreter.step().expect("the trap again");
        assert!(!interpreter.get_input_settings().echo);
    }

    #[test]
    fn a_setting_that_changes_nothing_journals_nothing() {
        let mut interpreter = with_history("    move.b #1,d1\n    move.b #12,d0\n    trap #15");
        for _ in 0..3 {
            interpreter.step().expect("a step");
        }
        let step = interpreter.undo().expect("the trap");
        assert!(
            step.get_mutations().is_empty(),
            "the echo was on already: {:?}",
            step.get_mutations()
        );
    }
}

mod files {
    use super::*;

    #[test]
    fn task_50_closes_every_file() {
        let code = task(50, "");
        let (_, interrupt) = waiting(&code);
        assert!(matches!(interrupt, Interrupt::CloseAllFiles));
        assert_eq!(
            result(&answered(&code, InterruptResult::CloseAllFiles(true))),
            0
        );
        assert_eq!(
            result(&answered(&code, InterruptResult::CloseAllFiles(false))),
            2
        );
    }

    #[test]
    fn task_51_carries_the_path_decoded() {
        //`\` is a separator to Windows, and sent as `/`; the name is Windows-1252
        let (_, interrupt) = waiting(&task(51, "    lea path,a1"));
        match interrupt {
            Interrupt::OpenFile(path) => assert_eq!(path, "data/in.txt"),
            other => panic!("unexpected {:?}", other),
        }
        let (_, interrupt) = waiting(&task(51, "    lea other,a1"));
        match interrupt {
            Interrupt::OpenFile(path) => assert_eq!(path, "café.txt"),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn a_path_stops_at_255_characters() {
        //`strncpy(buf, inStr, 255)`: a longer name is cut, not refused
        let code = "    ORG $1000
start:
    lea name,a1
    move.b #57,d0
    trap #15
name: dcb.b 300,'a'
    dc.b 0
";
        let (_, interrupt) = waiting(code);
        match interrupt {
            Interrupt::DeleteFile(path) => assert_eq!(path, "a".repeat(255)),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn task_51_answers_the_file_number_in_d1_and_the_result_in_d0_w() {
        let code = task(51, "    lea path,a1\n    move.l #$12345678,d1");
        let opened = answered(
            &code,
            InterruptResult::OpenFile(Some(OpenedFile {
                handle: 3,
                read_only: false,
            })),
        );
        assert_eq!(result(&opened), 0);
        assert_eq!(data(&opened, 1), 3, "the whole of D1.L");

        let read_only = answered(
            &code,
            InterruptResult::OpenFile(Some(OpenedFile {
                handle: 0,
                read_only: true,
            })),
        );
        assert_eq!(result(&read_only), 3, "a file opened for reading only");
        assert_eq!(data(&read_only, 1), 0);

        let failed = answered(&code, InterruptResult::OpenFile(None));
        assert_eq!(result(&failed), 2);
        assert_eq!(data(&failed, 1), 0xFFFF_FFFF, "-1, for no file");
    }

    #[test]
    fn task_52_creates_a_file() {
        let code = task(52, "    lea path,a1");
        match waiting(&code).1 {
            Interrupt::NewFile(path) => assert_eq!(path, "data/in.txt"),
            other => panic!("unexpected {:?}", other),
        }
        let created = answered(&code, InterruptResult::NewFile(Some(7)));
        assert_eq!((result(&created), data(&created, 1)), (0, 7));
        let failed = answered(&code, InterruptResult::NewFile(None));
        assert_eq!((result(&failed), data(&failed, 1)), (2, 0xFFFF_FFFF));
    }

    /// Task 53 reading file 2 into `buffer`, asking for ten bytes.
    fn reading() -> String {
        task(53, "    lea buffer,a1\n    move.l #2,d1\n    move.l #10,d2")
    }

    fn buffer_address(interpreter: &Interpreter) -> usize {
        interpreter.get_program().symbols()["buffer"].value as usize
    }

    #[test]
    fn task_53_reads_into_a1_and_counts_in_d2() {
        let (_, interrupt) = waiting(&reading());
        assert!(matches!(
            interrupt,
            Interrupt::ReadFile {
                handle: 2,
                count: 10
            }
        ));
        //fewer bytes than asked for is a success that says how many
        let read = answered(
            &reading(),
            InterruptResult::ReadFile(Some(Bytes(vec![1, 2, 3]))),
        );
        assert_eq!(result(&read), 0);
        assert_eq!(data(&read, 2), 3, "D2.L is the count read");
        let buffer = buffer_address(&read);
        assert_eq!(bytes_at(&read, buffer, 4), [1, 2, 3, 0x55], "and no more");
    }

    #[test]
    fn task_53_at_the_end_of_the_file_reports_1_and_leaves_d2() {
        let end = answered(&reading(), InterruptResult::ReadFile(Some(Bytes(vec![]))));
        assert_eq!(result(&end), 1);
        assert_eq!(data(&end, 2), 10, "as it was");
        let buffer = buffer_address(&end);
        assert_eq!(bytes_at(&end, buffer, 1), [0x55], "and nothing written");
    }

    #[test]
    fn task_53_reports_2_when_the_read_fails() {
        let failed = answered(&reading(), InterruptResult::ReadFile(None));
        assert_eq!((result(&failed), data(&failed, 2)), (2, 10));
    }

    #[test]
    fn task_53_refuses_more_bytes_than_it_asked_for() {
        let (mut interpreter, _) = waiting(&reading());
        let refused =
            interpreter.answer_interrupt(InterruptResult::ReadFile(Some(Bytes(vec![9; 11]))));
        match refused {
            Err(RuntimeError::InvalidAnswer { interrupt, reason }) => {
                assert_eq!(interrupt, "ReadFile");
                assert!(reason.contains("at most 10"), "{}", reason);
            }
            other => panic!("unexpected {:?}", other),
        }
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Interrupt);
        let buffer = buffer_address(&interpreter);
        assert_eq!(bytes_at(&interpreter, buffer, 1), [0x55]);
        interpreter
            .answer_interrupt(InterruptResult::ReadFile(Some(Bytes(vec![9; 10]))))
            .expect("ten bytes are taken");
        assert_eq!(data(&interpreter, 2), 10);
    }

    #[test]
    fn task_54_carries_the_bytes_at_a1() {
        let code = "    ORG $1000
start:
    lea bytes,a1
    move.l #4,d1
    move.l #3,d2
    move.l #$ABCD0000,d0
    move.b #54,d0
    trap #15
    nop
bytes: dc.b 'abcd'
";
        let (_, interrupt) = waiting(code);
        match interrupt {
            Interrupt::WriteFile { handle, bytes } => {
                assert_eq!(handle, 4);
                assert_eq!(bytes.as_slice(), b"abc", "D2.L of them");
            }
            other => panic!("unexpected {:?}", other),
        }
        let written = answered(code, InterruptResult::WriteFile(true));
        assert_eq!((result(&written), data(&written, 2)), (0, 3));
        //a file open for reading only is one a write fails on, and that is 2, as `fwrite`
        //fails in EASy68K
        assert_eq!(
            result(&answered(code, InterruptResult::WriteFile(false))),
            2
        );
    }

    #[test]
    fn task_55_moves_to_an_absolute_position() {
        let code = task(55, "    move.l #1,d1\n    move.l #1234,d2");
        assert!(matches!(
            waiting(&code).1,
            Interrupt::PositionFile {
                handle: 1,
                offset: 1234
            }
        ));
        assert_eq!(
            result(&answered(&code, InterruptResult::PositionFile(true))),
            0
        );
        assert_eq!(
            result(&answered(&code, InterruptResult::PositionFile(false))),
            2
        );
    }

    #[test]
    fn task_55_refuses_a_negative_position_without_asking() {
        //`fseek(fp, offset, SEEK_SET)` takes D2.L as an `int` and fails before the start
        let mut interpreter = prepare(&task(55, "    move.l #1,d1\n    move.l #-1,d2"));
        assert_eq!(
            interpreter.run().expect("the run"),
            InterpreterStatus::Terminated
        );
        assert_eq!(result(&interpreter), 2);
    }

    #[test]
    fn task_56_closes_a_file() {
        let code = task(56, "    move.l #6,d1");
        assert!(matches!(waiting(&code).1, Interrupt::CloseFile(6)));
        assert_eq!(
            result(&answered(&code, InterruptResult::CloseFile(true))),
            0
        );
        assert_eq!(
            result(&answered(&code, InterruptResult::CloseFile(false))),
            2
        );
    }

    #[test]
    fn task_57_deletes_a_file() {
        let code = task(57, "    lea path,a1");
        assert!(matches!(waiting(&code).1, Interrupt::DeleteFile(path) if path == "data/in.txt"));
        assert_eq!(
            result(&answered(&code, InterruptResult::DeleteFile(true))),
            0
        );
        assert_eq!(
            result(&answered(&code, InterruptResult::DeleteFile(false))),
            2
        );
    }

    #[test]
    fn task_58_carries_the_dialog() {
        let (_, interrupt) = waiting(&task(
            58,
            "    move.l #1,d1\n    lea title,a1\n    lea filter,a2\n    lea path,a3",
        ));
        match interrupt {
            Interrupt::FileDialog {
                mode,
                title,
                filter,
                path,
            } => {
                assert_eq!(mode, FileDialogMode::Save);
                assert_eq!(title, "Load");
                assert_eq!(filter, "*.txt");
                assert_eq!(path, "data/in.txt");
            }
            other => panic!("unexpected {:?}", other),
        }
        //a title or a filter whose register is 0 is none
        let (_, interrupt) = waiting(&task(
            58,
            "    move.l #0,d1\n    move.l #0,a1\n    move.l #0,a2\n    lea path,a3",
        ));
        match interrupt {
            Interrupt::FileDialog {
                mode,
                title,
                filter,
                ..
            } => {
                assert_eq!(mode, FileDialogMode::Open);
                assert_eq!((title.as_str(), filter.as_str()), ("", ""));
            }
            other => panic!("unexpected {:?}", other),
        }
    }

    /// Task 58 asking to open a file, its path buffer at `buffer`.
    fn choosing() -> String {
        task(
            58,
            "    move.l #0,d1\n    move.l #0,a1\n    move.l #0,a2\n    lea buffer,a3",
        )
    }

    #[test]
    fn task_58_writes_the_chosen_path_to_a3() {
        let chosen = answered(
            &choosing(),
            InterruptResult::FileDialog(Some("scores/é.txt".to_string())),
        );
        assert_eq!(result(&chosen), 0);
        assert_eq!(data(&chosen, 1), 1, "D1.L is 1 for a file chosen");
        let buffer = buffer_address(&chosen);
        let written = bytes_at(&chosen, buffer, 257);
        assert_eq!(&written[..12], b"scores/\xE9.txt", "in Windows-1252");
        //`strncpy` pads with NULs to 255 bytes, then the 256th is a NUL too
        assert!(written[12..256].iter().all(|&byte| byte == 0));
        assert_eq!(written[256], 0x55, "and nothing past the 256 bytes");
    }

    #[test]
    fn task_58_cuts_a_chosen_path_at_255_characters() {
        let chosen = answered(
            &choosing(),
            InterruptResult::FileDialog(Some("p".repeat(300))),
        );
        let buffer = buffer_address(&chosen);
        let written = bytes_at(&chosen, buffer, 257);
        assert!(written[..255].iter().all(|&byte| byte == b'p'));
        assert_eq!((written[255], written[256]), (0, 0x55));
    }

    #[test]
    fn task_58_cancelled_puts_0_in_d1() {
        let cancelled = answered(&choosing(), InterruptResult::FileDialog(None));
        assert_eq!((result(&cancelled), data(&cancelled, 1)), (0, 0));
        let buffer = buffer_address(&cancelled);
        assert_eq!(bytes_at(&cancelled, buffer, 1), [0x55], "nothing written");
    }

    #[test]
    fn task_58_reports_2_when_the_path_does_not_fit() {
        let code = task(
            58,
            "    move.l #0,d1\n    move.l #0,a1\n    move.l #0,a2\n    lea buffer,a3\n    move.l #$FFFF80,a3",
        );
        //the path the dialog starts on is read from (A3) as well, and memory starts as $FF,
        //so it reads 255 characters of it
        let after = answered(
            &code,
            InterruptResult::FileDialog(Some("a.txt".to_string())),
        );
        assert_eq!(result(&after), 2);
        assert_eq!(data(&after, 1), 0, "D1.L is left as it was");
    }

    #[test]
    fn task_58_refuses_a_mode_other_than_0_and_1() {
        //EASy68K shows no dialog and reports success; s68k stops and says why
        let mut interpreter = prepare(&task(58, "    move.l #2,d1\n    lea path,a3"));
        let error = interpreter.run().expect_err("an error");
        assert!(
            matches!(&error, RuntimeError::InvalidTrapArgument { task: 58, reason } if reason.starts_with("D1.L is 2,")),
            "{:?}",
            error
        );
        assert_ended_with(&interpreter, &error);
    }

    #[test]
    fn task_59_reports_what_is_at_the_path_whatever_d1_l_holds() {
        //`fileOp` never reads its mode, so neither does this
        let code = task(59, "    lea path,a1\n    move.l #5,d1");
        assert!(matches!(waiting(&code).1, Interrupt::FileExists(path) if path == "data/in.txt"));
        for (existence, expected) in [
            (FileExistence::Writable, 0),
            (FileExistence::ReadOnly, 3),
            (FileExistence::Missing, 2),
        ] {
            let after = answered(&code, InterruptResult::FileExists(existence));
            assert_eq!(result(&after), expected, "{:?}", existence);
            assert_eq!(data(&after, 1), 5, "D1 is not written");
        }
    }

    #[test]
    fn a_file_number_outside_0_to_7_is_2_without_asking() {
        for (number, task_number) in [("8", 53), ("-1", 53), ("8", 54), ("$100", 55), ("8", 56)] {
            let mut interpreter = prepare(&task(
                task_number,
                &format!("    lea buffer,a1\n    move.l #{number},d1\n    move.l #1,d2"),
            ));
            assert_eq!(
                interpreter.run().expect("the run"),
                InterpreterStatus::Terminated,
                "task {task_number} with {number} raises no interrupt"
            );
            assert_eq!(result(&interpreter), 2, "task {task_number} with {number}");
        }
    }

    #[test]
    fn a_read_or_a_write_of_no_bytes_is_2_without_asking() {
        //`fwrite(buf, 0, 1, fp)` writes no item and fails. `fread` of nothing fails too,
        //unless an earlier read reached the end of the file, which EASy68K then reports as
        //1: s68k keeps no such indicator and reports 2, a documented deviation
        for task_number in [53, 54] {
            let mut interpreter = prepare(&task(
                task_number,
                "    lea buffer,a1\n    move.l #0,d1\n    move.l #0,d2",
            ));
            assert_eq!(
                interpreter.run().expect("the run"),
                InterpreterStatus::Terminated
            );
            assert_eq!(result(&interpreter), 2, "task {task_number}");
        }
    }

    #[test]
    fn a_buffer_past_the_end_of_memory_is_2_without_asking() {
        //EASy68K checks (A1) + D2.L against the 16 MB before it reads or writes; its 32 bit
        //sum wraps for a count near 4 GB and goes on, where s68k reports 2 as well
        for (address, count) in [("$FFFFF0", "$11"), ("$1000", "$FFFFF001")] {
            for task_number in [53, 54] {
                let mut interpreter = prepare(&task(
                    task_number,
                    &format!("    move.l #{address},a1\n    move.l #0,d1\n    move.l #{count},d2"),
                ));
                assert_eq!(
                    interpreter.run().expect("the run"),
                    InterpreterStatus::Terminated,
                    "task {task_number}, {address} + {count}"
                );
                assert_eq!(result(&interpreter), 2);
            }
        }
        //the last byte of memory is still one
        let (_, interrupt) = waiting(&task(
            53,
            "    move.l #$FFFFF0,a1\n    move.l #0,d1\n    move.l #$10,d2",
        ));
        assert!(matches!(interrupt, Interrupt::ReadFile { count: 16, .. }));
    }

    #[test]
    fn undo_puts_back_what_a_read_wrote() {
        let mut interpreter = with_history(&reading());
        let interrupt = run_to_interrupt(&mut interpreter);
        assert!(matches!(interrupt, Interrupt::ReadFile { .. }));
        interpreter
            .answer_interrupt(InterruptResult::ReadFile(Some(Bytes(vec![1, 2, 3]))))
            .expect("the bytes");
        let buffer = buffer_address(&interpreter);
        assert_eq!(bytes_at(&interpreter, buffer, 3), [1, 2, 3]);
        let step = interpreter.undo().expect("the trap");
        assert!(
            step.get_mutations()
                .iter()
                .any(|mutation| matches!(mutation, MutationOperation::WriteMemoryBytes { new, .. } if new == &[1, 2, 3])),
            "the bytes are journaled with the step: {:?}",
            step.get_mutations()
        );
        assert_eq!(bytes_at(&interpreter, buffer, 3), [0x55; 3]);
        assert_eq!(data(&interpreter, 2), 10);
        assert_eq!(data(&interpreter, 0), 0xABCD_0035, "D0 is the task again");
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
    }

    #[test]
    fn undo_puts_back_a_result_the_interpreter_wrote_itself() {
        let mut interpreter = with_history(&task(56, "    move.l #9,d1"));
        interpreter.run().expect("the run");
        assert_eq!(result(&interpreter), 2);
        interpreter.undo().expect("the nop");
        interpreter.undo().expect("the trap");
        assert_eq!(data(&interpreter, 0), 0xABCD_0038);
    }
}

mod sound {
    use super::*;

    #[test]
    fn the_sound_tasks_carry_their_arguments() {
        let setup = "    lea path,a1\n    move.l #$1203,d1\n    move.l #2,d2";
        let cases: [(u8, Interrupt); 8] = [
            (70, Interrupt::PlaySound("data/in.txt".to_string())),
            (
                71,
                Interrupt::LoadSound {
                    path: "data/in.txt".to_string(),
                    index: 3,
                },
            ),
            (72, Interrupt::PlayLoadedSound(3)),
            (73, Interrupt::PlaySoundDirectX("data/in.txt".to_string())),
            (
                74,
                Interrupt::LoadSoundDirectX {
                    path: "data/in.txt".to_string(),
                    index: 3,
                },
            ),
            (75, Interrupt::PlayLoadedSoundDirectX(3)),
            (
                76,
                Interrupt::ControlSound {
                    index: 3,
                    control: 2,
                },
            ),
            (
                77,
                Interrupt::ControlSoundDirectX {
                    index: 3,
                    control: 2,
                },
            ),
        ];
        for (number, expected) in cases {
            let (_, interrupt) = waiting(&task(number, setup));
            assert_eq!(
                format!("{:?}", interrupt),
                format!("{:?}", expected),
                "task {number}, D1.B the index"
            );
        }
    }

    #[test]
    fn a_sound_answer_is_1_or_0_in_d0_w_and_task_71_writes_nothing() {
        let played = answered(
            &task(70, "    lea path,a1"),
            InterruptResult::PlaySound(true),
        );
        assert_eq!(result(&played), 1);
        let busy = answered(
            &task(76, "    move.l #0,d1"),
            InterruptResult::ControlSound(false),
        );
        assert_eq!(result(&busy), 0);
        let loaded = answered(&task(71, "    lea path,a1"), InterruptResult::LoadSound);
        assert_eq!(data(&loaded, 0), 0xABCD_0047, "D0 is still the task");
    }
}

mod failures {
    use super::*;

    #[test]
    fn an_unsupported_task_ends_the_program_with_an_exception() {
        //the printer, the cycle counter, the serial ports, an interrupt request, the network,
        //and numbers that are no task at all
        for number in [10u8, 21, 30, 31, 32, 40, 60, 62, 100, 107, 26, 99, 200] {
            let mut interpreter =
                prepare(&format!("    move.b #{number},d0\n    trap #15\n    nop"));
            let error = interpreter.run().expect_err("an error");
            assert_eq!(error, RuntimeError::UnsupportedTrapTask { task: number });
            assert_ended_with(&interpreter, &error);
            assert!(
                interpreter.step().is_err(),
                "task {number}: nothing runs after it"
            );
        }
    }

    #[test]
    fn an_instruction_error_ends_the_program_too() {
        //a division by zero used to leave the program running, one instruction further on
        let mut interpreter = prepare("    move.l #1,d0\n    divu #0,d0\n    move.l #2,d1");
        let error = interpreter.run().expect_err("an error");
        assert_eq!(error, RuntimeError::DivisionByZero);
        assert_ended_with(&interpreter, &error);
        assert_eq!(
            data(&interpreter, 1),
            0,
            "the instruction after it never ran"
        );
    }

    #[test]
    fn a_string_with_no_nul_is_an_argument_error() {
        //memory starts as $FF, so a string at an address nothing wrote never ends
        let mut interpreter = prepare("    move.l #$100000,a1\n    move.b #13,d0\n    trap #15");
        let error = interpreter.run().expect_err("an error");
        assert!(
            matches!(&error, RuntimeError::InvalidTrapArgument { task: 13, reason } if reason.contains("no NUL")),
            "{:?}",
            error
        );
        assert_ended_with(&interpreter, &error);
    }

    #[test]
    fn undoing_a_failed_trap_brings_the_program_back_running() {
        let mut interpreter = with_history("    move.b #99,d0\n    trap #15\n    move.l #5,d2");
        interpreter.step().expect("the move");
        let failed = interpreter.step().expect_err("the trap fails");
        assert_eq!(failed, RuntimeError::UnsupportedTrapTask { task: 99 });
        assert!(interpreter.has_terminated());
        interpreter.undo().expect("the trap is undone");
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
        assert!(!interpreter.has_terminated());
        assert_eq!(interpreter.get_termination(), None, "and the end with it");
        assert_eq!(interpreter.get_pc(), 0x1004, "on the trap again");
        //a poke can put a task there that works, and the program goes on
        interpreter
            .begin_poke()
            .expect("a poke between instructions");
        interpreter.set_register_value(RegisterOperand::Data(0), 12, Size::Byte);
        interpreter.end_poke().expect("the poke");
        assert_eq!(
            interpreter.run().expect("the run"),
            InterpreterStatus::Terminated
        );
        assert_eq!(data(&interpreter, 2), 5);
    }

    #[test]
    fn the_termination_says_why_the_program_ended() {
        let mut ended = prepare("    move.b #9,d0\n    trap #15\n    nop");
        assert_eq!(ended.get_termination(), None, "nothing has ended yet");
        ended.run().expect("the run");
        assert_eq!(ended.get_termination(), Some(&Termination::TerminateTask));

        let mut ran_off = prepare("    nop");
        ran_off.run().expect("the run");
        assert_eq!(ran_off.get_termination(), Some(&Termination::EndOfProgram));

        let empty = prepare("    ORG $1000\n");
        assert!(empty.has_terminated());
        assert_eq!(empty.get_termination(), Some(&Termination::EndOfProgram));

        let mut stopped = prepare("    move.b #70,d0\n    trap #15\n    nop");
        run_to_interrupt(&mut stopped);
        stopped
            .answer_interrupt(InterruptResult::Terminate)
            .expect("the answer");
        assert_eq!(
            stopped.get_termination(),
            Some(&Termination::TerminatedByHost)
        );

        //an answer to the last instruction's interrupt ends the program at the end of it
        let mut last = prepare("    move.b #3,d0\n    trap #15");
        run_to_interrupt(&mut last);
        last.answer_interrupt(InterruptResult::DisplayNumber)
            .expect("the answer");
        assert_eq!(last.get_termination(), Some(&Termination::EndOfProgram));

        let mut undone = with_history("    move.b #9,d0\n    trap #15\n    nop");
        undone.run().expect("the run");
        undone.undo().expect("task 9");
        assert_eq!(undone.get_termination(), None);
        assert!(
            undone.get_current_interrupt().is_err(),
            "and its interrupt is gone"
        );
    }
}

mod answers {
    use super::*;

    /// One answer of every name, which is one per [`Interrupt`].
    fn every_answer() -> Vec<InterruptResult> {
        use InterruptResult::*;
        vec![
            DisplayStringWithCRLF,
            DisplayStringWithoutCRLF,
            ReadKeyboardString("x".to_string()),
            DisplayNumber,
            DisplayNumberInBase,
            ReadNumber("1".to_string()),
            ReadChar('x'),
            DisplayChar,
            GetTime(0),
            Terminate,
            Delay,
            DisplaySignedNumberInField,
            DisplayStringAndNumber,
            DisplayStringAndReadNumber("1".to_string()),
            CheckKeyboardInput(false),
            GetKeyState(KeyStateResult::LastKeys { up: 0, down: 0 }),
            ReadMouse {
                flags: 0,
                x: 0,
                y: 0,
            },
            SetSimulatorShortcuts,
            SetPenColor,
            SetFillColor,
            DrawPixel,
            GetPixelColor(0),
            DrawLine,
            DrawLineTo,
            MoveTo,
            DrawRectangle,
            DrawEllipse,
            FloodFill,
            DrawUnfilledRectangle,
            DrawUnfilledEllipse,
            SetDrawingMode,
            SetPenWidth,
            Repaint,
            DrawText,
            GetPenPosition(0, 0),
            SetScreenSize,
            GetScreenSize(0, 0),
            SetScreenMode,
            ClearScreen,
            SetTextCursorPosition,
            GetTextCursorPosition(0, 0),
            CloseAllFiles(true),
            OpenFile(None),
            NewFile(None),
            ReadFile(None),
            WriteFile(true),
            PositionFile(true),
            CloseFile(true),
            DeleteFile(true),
            FileDialog(None),
            FileExists(FileExistence::Missing),
            PlaySound(false),
            LoadSound,
            PlayLoadedSound(false),
            PlaySoundDirectX(false),
            LoadSoundDirectX(false),
            PlayLoadedSoundDirectX(false),
            ControlSound(false),
            ControlSoundDirectX(false),
        ]
    }

    /// One interrupt of every name.
    fn every_interrupt() -> Vec<Interrupt> {
        use Interrupt::*;
        let text = || "x".to_string();
        vec![
            DisplayStringWithCRLF(text()),
            DisplayStringWithoutCRLF(text()),
            ReadKeyboardString,
            DisplayNumber(text()),
            DisplayNumberInBase(text()),
            ReadNumber,
            ReadChar,
            DisplayChar('x'),
            GetTime,
            Terminate,
            Delay(0),
            DisplaySignedNumberInField(text()),
            DisplayStringAndNumber(text()),
            DisplayStringAndReadNumber(text()),
            CheckKeyboardInput,
            GetKeyState(KeyStateRequest::LastKeys),
            ReadMouse(0),
            SetSimulatorShortcuts(0),
            SetPenColor(0),
            SetFillColor(0),
            DrawPixel(0, 0),
            GetPixelColor(0, 0),
            DrawLine(0, 0, 0, 0),
            DrawLineTo(0, 0),
            MoveTo(0, 0),
            DrawRectangle(0, 0, 0, 0),
            DrawEllipse(0, 0, 0, 0),
            FloodFill(0, 0),
            DrawUnfilledRectangle(0, 0, 0, 0),
            DrawUnfilledEllipse(0, 0, 0, 0),
            SetDrawingMode(4),
            SetPenWidth(1),
            Repaint,
            DrawText(0, 0, text()),
            GetPenPosition,
            SetScreenSize(0, 0),
            GetScreenSize,
            SetScreenMode(1),
            ClearScreen,
            SetTextCursorPosition(0, 0),
            GetTextCursorPosition,
            CloseAllFiles,
            OpenFile(text()),
            NewFile(text()),
            ReadFile {
                handle: 0,
                count: 1,
            },
            WriteFile {
                handle: 0,
                bytes: Bytes(vec![1]),
            },
            PositionFile {
                handle: 0,
                offset: 0,
            },
            CloseFile(0),
            DeleteFile(text()),
            FileDialog {
                mode: FileDialogMode::Open,
                title: text(),
                filter: text(),
                path: text(),
            },
            FileExists(text()),
            PlaySound(text()),
            LoadSound {
                path: text(),
                index: 0,
            },
            PlayLoadedSound(0),
            PlaySoundDirectX(text()),
            LoadSoundDirectX {
                path: text(),
                index: 0,
            },
            PlayLoadedSoundDirectX(0),
            ControlSound {
                index: 0,
                control: 0,
            },
            ControlSoundDirectX {
                index: 0,
                control: 0,
            },
        ]
    }

    #[test]
    fn every_interrupt_has_one_answer_of_its_name() {
        let interrupts: BTreeSet<&str> = every_interrupt().iter().map(Interrupt::name).collect();
        let answers: BTreeSet<&str> = every_answer().iter().map(InterruptResult::name).collect();
        assert_eq!(
            interrupts.len(),
            every_interrupt().len(),
            "names are unique"
        );
        assert_eq!(answers.len(), every_answer().len(), "names are unique");
        assert_eq!(interrupts, answers);
        //and the name is the `type` each crosses into JavaScript with
        for interrupt in every_interrupt() {
            let value = serde_json::to_value(&interrupt).expect("an interrupt serialises");
            assert_eq!(value["type"], interrupt.name());
        }
        for answer in every_answer() {
            let value = serde_json::to_value(&answer).expect("an answer serialises");
            assert_eq!(value["type"], answer.name());
        }
    }

    #[test]
    fn every_answer_but_its_own_and_terminate_is_refused() {
        //a file read, a display task, a key, the dialog and a sound, each waiting
        let programs = [
            task(53, "    lea buffer,a1\n    move.l #0,d1\n    move.l #4,d2"),
            task(3, ""),
            task(5, ""),
            task(58, "    move.l #0,d1\n    lea path,a3"),
            task(77, ""),
        ];
        for program in programs {
            let (mut interpreter, pending) = waiting(&program);
            for answer in every_answer() {
                if answer.name() == pending.name() || answer.name() == "Terminate" {
                    continue;
                }
                match interpreter.answer_interrupt(answer.clone()) {
                    Err(RuntimeError::InvalidAnswer { interrupt, .. }) => {
                        assert_eq!(interrupt, pending.name())
                    }
                    other => panic!("{} answered {}: {:?}", pending.name(), answer.name(), other),
                }
                assert_eq!(
                    *interpreter.get_status(),
                    InterpreterStatus::Interrupt,
                    "{} still waits after {}",
                    pending.name(),
                    answer.name()
                );
            }
            assert_eq!(
                interpreter
                    .get_current_interrupt()
                    .expect("it waits")
                    .name(),
                pending.name()
            );
            let own = every_answer()
                .into_iter()
                .find(|answer| answer.name() == pending.name())
                .expect("an answer of its name");
            interpreter
                .answer_interrupt(own)
                .unwrap_or_else(|error| panic!("{} takes its own: {:?}", pending.name(), error));
        }
    }

    #[test]
    fn a_key_state_answer_has_to_be_the_form_asked_for() {
        let (mut keys, _) = waiting("    move.l #$41424344,d1\n    move.b #19,d0\n    trap #15");
        assert!(matches!(
            keys.answer_interrupt(InterruptResult::GetKeyState(KeyStateResult::LastKeys {
                up: 0,
                down: 0
            })),
            Err(RuntimeError::InvalidAnswer { .. })
        ));
        keys.answer_interrupt(InterruptResult::GetKeyState(KeyStateResult::Keys(
            [true; 4],
        )))
        .expect("the four keys");

        let (mut last, _) = waiting("    move.l #0,d1\n    move.b #19,d0\n    trap #15");
        assert!(matches!(
            last.answer_interrupt(InterruptResult::GetKeyState(KeyStateResult::Keys(
                [false; 4]
            ))),
            Err(RuntimeError::InvalidAnswer { .. })
        ));
    }

    #[test]
    fn an_open_answer_with_a_file_number_past_7_is_refused() {
        let (mut opening, _) = waiting(&task(51, "    lea path,a1"));
        assert!(matches!(
            opening.answer_interrupt(InterruptResult::OpenFile(Some(OpenedFile {
                handle: 8,
                read_only: false
            }))),
            Err(RuntimeError::InvalidAnswer { .. })
        ));
        let (mut creating, _) = waiting(&task(52, "    lea path,a1"));
        assert!(matches!(
            creating.answer_interrupt(InterruptResult::NewFile(Some(200))),
            Err(RuntimeError::InvalidAnswer { .. })
        ));
        assert_eq!(*creating.get_status(), InterpreterStatus::Interrupt);
    }

    #[test]
    fn an_answer_with_nothing_pending_is_refused_with_its_own_error() {
        let mut fresh = prepare("    nop");
        assert_eq!(
            fresh.answer_interrupt(InterruptResult::DisplayChar),
            Err(RuntimeError::NoPendingInterrupt)
        );
        //task 9 leaves its interrupt behind, ended, and nothing answers it
        let mut ended = prepare("    move.b #9,d0\n    trap #15\n    nop");
        ended.run().expect("the run");
        assert_eq!(
            ended.answer_interrupt(InterruptResult::Terminate),
            Err(RuntimeError::NoPendingInterrupt)
        );
    }

    #[test]
    fn undoing_a_trap_whose_interrupt_waits_takes_the_interrupt_back() {
        //it used to leave the interrupt and the executing instruction behind, so the next
        //poke was refused and the interrupt could still be answered
        let mut interpreter = with_history("    move.b #4,d0\n    trap #15\n    nop");
        interpreter.step().expect("the move");
        interpreter.step().expect("the trap");
        assert!(interpreter.is_executing());
        interpreter.undo().expect("the trap is undone");
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
        assert!(!interpreter.is_executing());
        assert!(interpreter.get_current_interrupt().is_err());
        assert_eq!(
            interpreter.answer_interrupt(InterruptResult::ReadNumber("1".to_string())),
            Err(RuntimeError::NoPendingInterrupt)
        );
        interpreter.begin_poke().expect("a poke is allowed again");
        interpreter.end_poke().expect("and ends");
        assert_eq!(
            interpreter.step().expect("the trap again"),
            InterpreterStatus::Interrupt
        );
    }
}
