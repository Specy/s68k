//! `include` and `incbin` over a Project of Files.
//!
//! The design record's "Files, `include`, `incbin`" and the glossary's
//! "Include", "Incbin" and "Include chain" (CONTEXT.md), one test per rule they
//! state. The rules that are about the *shape* of the assembled sequence —
//! path resolution, cycles, the depth backstop, the chain itself — are tested
//! beside the code that builds it, in
//! [`assembler::include`](crate::assembler::include); what is here is what the
//! Layout, the Program and the Interpreter make of it.
//!
//! The corpus-scale version of the first test in this file is
//! `editor_programs_split_across_files_assemble_the_same` in
//! [`corpus`](super::corpus): three real programs cut across Files, assembling
//! byte for byte to what they assemble to whole.

use crate::assembler::diagnostics::Diagnostic;
use crate::assembler::program::{MemoryContent, Program};
use crate::assembler::source::Files;
use crate::assembler::Assembly;

/// A Project of text Files.
fn project(files: &[(&str, &str)]) -> Files {
    let mut project = Files::new();
    for (path, text) in files {
        project.insert_text(path, *text);
    }
    project
}

/// Assemble a Project from `main.m68k`.
fn assemble(files: &Files) -> Assembly {
    crate::assembler::assemble(files, "main.m68k")
}

/// The Program a Project builds, or a panic naming what stopped it.
fn program(files: &Files) -> Program {
    let assembly = assemble(files);
    match assembly.program {
        Some(program) => program,
        None => panic!(
            "the project did not assemble: {:#?}",
            assembly
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .map(|diagnostic| format!(
                    "{}:{}: {}",
                    diagnostic.location.file,
                    diagnostic.location.line,
                    diagnostic.message()
                ))
                .collect::<Vec<_>>()
        ),
    }
}

/// The codes a Project raises, in the order they come back.
fn codes(files: &Files) -> Vec<&'static str> {
    assemble(files)
        .diagnostics
        .iter()
        .map(Diagnostic::code)
        .collect()
}

/// The Diagnostics a Project raises, as `code file:line`.
fn found(files: &Files) -> Vec<String> {
    assemble(files)
        .diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {}:{}",
                diagnostic.code(),
                diagnostic.location.file,
                diagnostic.location.line
            )
        })
        .collect()
}

/// The addresses of a Program's instructions, with the File and line each came
/// from.
fn instructions(files: &Files) -> Vec<String> {
    program(files)
        .instructions()
        .iter()
        .map(|instruction| {
            format!(
                "${:x} {}:{}",
                instruction.address, instruction.location.file, instruction.location.line
            )
        })
        .collect()
}

/// The value of a Symbol of a built Program.
fn symbol(files: &Files, name: &str) -> i64 {
    program(files)
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no symbol named `{name}`"))
        .value
}

/// The initial memory of a Program, as `(address, bytes)`.
fn memory(files: &Files) -> Vec<(usize, String)> {
    program(files)
        .memory()
        .iter()
        .map(|run| {
            (
                run.address,
                match &run.content {
                    MemoryContent::Bytes { bytes } => {
                        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
                    }
                    MemoryContent::Reserved { length } => format!("{length} reserved"),
                },
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// A textual include
// ---------------------------------------------------------------------------

#[test]
fn the_included_lines_are_assembled_where_the_include_line_is() {
    let files = project(&[
        ("main.m68k", "    nop\n    include lib.m68k\n    rts\n"),
        ("lib.m68k", "    nop\n    nop\n"),
    ]);
    assert_eq!(
        instructions(&files),
        vec![
            "$1000 main.m68k:0",
            "$1004 lib.m68k:0",
            "$1008 lib.m68k:1",
            "$100c main.m68k:2",
        ],
        "the included lines take the addresses they would have had if they had \
         been written where the `include` line is"
    );
}

#[test]
fn an_included_file_is_in_the_same_section_and_at_the_same_address() {
    let files = project(&[
        (
            "main.m68k",
            "    section 1\n    org $3000\n    include lib.m68k\n    dc.b 2\n",
        ),
        ("lib.m68k", "    dc.b 1\n"),
    ]);
    assert_eq!(
        memory(&files),
        vec![(0x3000, "01".to_string()), (0x3001, "02".to_string())],
        "the section and the current address of the including line are the \
         section and the address of the first included line"
    );
}

#[test]
fn there_is_one_symbol_namespace_and_it_reaches_both_ways() {
    let files = project(&[
        (
            "main.m68k",
            "start:\n    bsr routine\n    move.l #size,d0\n    include lib.m68k\n",
        ),
        (
            "lib.m68k",
            "size equ 12\nroutine:\n    bra start\n    rts\n",
        ),
    ]);
    assert_eq!(symbol(&files, "size"), 12);
    assert_eq!(symbol(&files, "routine"), 0x1008);
    assert_eq!(symbol(&files, "start"), 0x1000);
    assert!(codes(&files).is_empty(), "{:?}", codes(&files));
}

#[test]
fn local_label_scopes_run_across_the_boundary() {
    // `.loop` in the included file belongs to `outer`, which is declared in the
    // entry file above the `include`; the Global label the included file
    // declares closes that scope, so the `.loop` below the `include` line is a
    // different Symbol.
    let files = project(&[
        (
            "main.m68k",
            "outer:\n    include lib.m68k\n.loop:\n    bra .loop\n",
        ),
        ("lib.m68k", ".loop:\n    bra .loop\ninner:\n"),
    ]);
    let program = program(&files);
    let names: Vec<&str> = program.symbols().keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec!["inner", "inner:loop", "outer", "outer:loop"],
        "`outer:loop` is the included file's, and `inner:loop` is the entry \
         file's — the scope the included file opened is still open below the \
         `include` line"
    );
    assert!(
        codes(&files).is_empty(),
        "a local label written after the include, under a global label the \
         included file declared, is that scope's: {:?}",
        codes(&files)
    );
}

#[test]
fn a_set_variable_sees_the_latest_definition_above_it_in_the_assembled_sequence() {
    // This is the first of the two places phases 1 to 3 compared Source line
    // indexes, and it is why `include` had to make them positions: every `dc.b`
    // below reads the `set` above it, whichever File either of them is in.
    let files = project(&[
        (
            "main.m68k",
            "size set 1\n    dc.b size\n    include lib.m68k\n    dc.b size\nsize set 4\n    dc.b size\n",
        ),
        ("lib.m68k", "    dc.b size\nsize set 3\n    dc.b size\n"),
    ]);
    let bytes: Vec<String> = memory(&files).into_iter().map(|(_, bytes)| bytes).collect();
    assert_eq!(bytes, vec!["01", "01", "03", "03", "04"]);
}

#[test]
fn a_register_list_has_to_be_defined_above_the_movem_in_the_assembled_sequence() {
    // The second of the two, and the same reasoning: a `reg` list in a File
    // included above the `movem` is defined above it.
    let above = project(&[
        (
            "main.m68k",
            "    include lib.m68k\n    movem.l saved,-(a7)\n",
        ),
        ("lib.m68k", "saved reg d0-d2\n"),
    ]);
    assert!(codes(&above).is_empty(), "{:?}", codes(&above));

    let below = project(&[
        (
            "main.m68k",
            "    movem.l saved,-(a7)\n    include lib.m68k\n",
        ),
        ("lib.m68k", "saved reg d0-d2\n"),
    ]);
    assert_eq!(codes(&below), vec!["register_list_not_defined_yet"]);
}

#[test]
fn a_label_on_an_include_line_names_the_first_included_byte() {
    let files = project(&[
        ("main.m68k", "    org $2000\ntable: include data.m68k\n"),
        ("data.m68k", "    dc.b 1,2,3\n"),
    ]);
    assert_eq!(symbol(&files, "table"), 0x2000);
    assert_eq!(memory(&files), vec![(0x2000, "010203".to_string())]);
}

#[test]
fn the_same_file_may_be_included_twice() {
    let files = project(&[
        ("main.m68k", "    include lib.m68k\n    include lib.m68k\n"),
        ("lib.m68k", "    dc.b 7\n"),
    ]);
    assert!(codes(&files).is_empty(), "{:?}", codes(&files));
    assert_eq!(
        memory(&files),
        vec![(0x1000, "07".to_string()), (0x1001, "07".to_string())],
        "the lines are assembled twice, which is what pasting them twice means"
    );
}

#[test]
fn a_file_included_twice_says_so_where_that_defines_a_name_twice() {
    let files = project(&[
        ("main.m68k", "    include lib.m68k\n    include lib.m68k\n"),
        ("lib.m68k", "size equ 4\n"),
    ]);
    let assembly = assemble(&files);
    let diagnostic = assembly
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == "symbol_already_defined")
        .expect("the name is defined twice");
    let related: Vec<String> = diagnostic
        .related
        .iter()
        .map(|(location, message)| format!("{}:{} {message}", location.file, location.line))
        .collect();
    assert_eq!(
        related,
        vec![
            "lib.m68k:0 `size` is a constant defined here",
            "main.m68k:0 `lib.m68k` is included here",
            "main.m68k:1 and included again here, so every name in `lib.m68k` is defined twice",
            "main.m68k:1 included from `main.m68k`",
        ],
        "the two definitions, the two `include` lines, and the chain the \
         second one was reached through"
    );
}

#[test]
fn two_definitions_in_one_copy_of_a_file_say_nothing_about_include() {
    let files = project(&[
        ("main.m68k", "    include lib.m68k\n"),
        ("lib.m68k", "size equ 4\nsize equ 5\n"),
    ]);
    let assembly = assemble(&files);
    let diagnostic = &assembly.diagnostics[0];
    assert_eq!(diagnostic.code(), "symbol_already_defined");
    let messages: Vec<&str> = diagnostic
        .related
        .iter()
        .map(|(_, message)| message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "`size` is a constant defined here",
            "included from `main.m68k`"
        ],
        "the include chain, and nothing about the file being included twice"
    );
}

#[test]
fn end_belongs_in_the_entry_file() {
    let files = project(&[
        (
            "main.m68k",
            "start:\n    include lib.m68k\n    nop\n    end start\n",
        ),
        ("lib.m68k", "    end\n    nop\n"),
    ]);
    assert_eq!(codes(&files), vec!["end_in_an_included_file"]);
    let assembly = assemble(&files);
    assert_eq!(assembly.diagnostics[0].location.file, "lib.m68k");
    // The line is refused and assembly carries on, so the `nop` under it is
    // still laid out and the entry point is still the entry file's own.
    let files = project(&[
        ("main.m68k", "start:\n    nop\n    include lib.m68k\n"),
        ("lib.m68k", "    end\n    nop\n"),
    ]);
    let assembly = assemble(&files);
    assert_eq!(assembly.diagnostics.len(), 1);
    assert!(assembly.program.is_none(), "the line is an error");
}

#[test]
fn an_include_after_end_is_still_read_and_its_lines_are_not_assembled() {
    // The expansion is textual and knows nothing about `end`, so the File is
    // read and a mistake in it is still reported; the lines it brings in are
    // held back by the rule that holds back every line after `end`, and the
    // first of them warns once.
    let files = project(&[
        ("main.m68k", "    nop\n    end\n    include lib.m68k\n"),
        ("lib.m68k", "    nop\n"),
    ]);
    assert_eq!(
        codes(&files),
        vec!["end_without_an_address", "code_after_end"]
    );
    assert_eq!(instructions(&files), vec!["$1000 main.m68k:0"]);
}

// ---------------------------------------------------------------------------
// The Include chain
// ---------------------------------------------------------------------------

#[test]
fn a_diagnostic_in_an_included_file_carries_the_include_chain() {
    let files = project(&[
        ("main.m68k", "    nop\n    include a.m68k\n"),
        ("a.m68k", "    include b.m68k\n"),
        ("b.m68k", "    move.l d0,#5\n"),
    ]);
    let assembly = assemble(&files);
    let diagnostic = &assembly.diagnostics[0];
    assert_eq!(diagnostic.location.file, "b.m68k");
    let related: Vec<String> = diagnostic
        .related
        .iter()
        .map(|(location, message)| format!("{}:{} {message}", location.file, location.line))
        .collect();
    assert_eq!(
        related,
        vec![
            "a.m68k:0 included from `a.m68k`",
            "main.m68k:1 included from `main.m68k`",
        ],
        "innermost first"
    );
}

#[test]
fn diagnostics_come_back_in_the_order_of_the_assembled_sequence() {
    let files = project(&[
        (
            "main.m68k",
            "    move.l d0,#1\n    include lib.m68k\n    move.l d0,#3\n",
        ),
        ("lib.m68k", "    move.l d0,#2\n"),
    ]);
    assert_eq!(
        found(&files),
        vec![
            "invalid_addressing_mode main.m68k:0",
            "invalid_addressing_mode lib.m68k:0",
            "invalid_addressing_mode main.m68k:2",
        ],
        "an included file's diagnostics sit between the two halves of the file \
         that includes it"
    );
}

#[test]
fn an_instruction_carries_the_include_chain_it_was_reached_through() {
    let files = project(&[
        ("main.m68k", "    include a.m68k\n    nop\n"),
        ("a.m68k", "    include b.m68k\n"),
        ("b.m68k", "    rts\n"),
    ]);
    let program = program(&files);
    let chains: Vec<Vec<String>> = program
        .instructions()
        .iter()
        .map(|instruction| {
            instruction
                .include_chain
                .iter()
                .map(|location| format!("{}:{}", location.file, location.line))
                .collect()
        })
        .collect();
    assert_eq!(
        chains,
        vec![
            vec!["a.m68k:0".to_string(), "main.m68k:0".to_string()],
            vec![]
        ],
        "the `rts` of `b.m68k` was reached through two include lines, and the \
         `nop` of the entry file through none"
    );
}

#[test]
fn the_two_copies_of_a_file_differ_only_in_their_chain() {
    let files = project(&[
        ("main.m68k", "    include lib.m68k\n    include lib.m68k\n"),
        ("lib.m68k", "    nop\n"),
    ]);
    let program = program(&files);
    let copies: Vec<(usize, String, usize)> = program
        .instructions()
        .iter()
        .map(|instruction| {
            (
                instruction.address,
                instruction.location.file.clone(),
                instruction.include_chain[0].line,
            )
        })
        .collect();
    assert_eq!(
        copies,
        vec![
            (0x1000, "lib.m68k".to_string(), 0),
            (0x1004, "lib.m68k".to_string(), 1)
        ],
        "one Location, two include lines: the chain is what tells the two \
         copies apart"
    );
}

#[test]
fn a_breakpoint_stops_in_an_included_file() {
    use crate::interpreter::{Breakpoint, Interpreter, InterpreterStatus};
    let files = project(&[
        (
            "main.m68k",
            "    move.l #1,d0\n    include lib.m68k\n    move.b #9,d0\n    trap #15\n",
        ),
        ("lib.m68k", "    move.l #2,d1\n    move.l #3,d2\n"),
    ]);
    let mut interpreter = Interpreter::new(program(&files), None);
    let breakpoints = [Breakpoint::new("lib.m68k", 1)];
    assert_eq!(
        interpreter.get_breakpoint_addresses(&breakpoints),
        [0x1008].into_iter().collect(),
        "a breakpoint is a (file, line) and the included file is a file"
    );
    let status = interpreter
        .run_with_breakpoints(&breakpoints, None)
        .expect("to stop at the breakpoint");
    assert_eq!(
        status,
        InterpreterStatus::Running,
        "stopped, not terminated"
    );
    assert_eq!(interpreter.get_pc(), 0x1008);
    assert_eq!(
        interpreter
            .get_current_location()
            .map(|location| location.file.clone()),
        Some("lib.m68k".to_string())
    );
}

#[test]
fn a_file_included_twice_stops_at_both_copies() {
    use crate::interpreter::{Breakpoint, Interpreter};
    let files = project(&[
        (
            "main.m68k",
            "    include lib.m68k\n    include lib.m68k\n    move.b #9,d0\n    trap #15\n",
        ),
        ("lib.m68k", "    nop\n"),
    ]);
    let interpreter = Interpreter::new(program(&files), None);
    let mut addresses: Vec<usize> = interpreter
        .get_breakpoint_addresses(&[Breakpoint::new("lib.m68k", 0)])
        .into_iter()
        .collect();
    addresses.sort();
    assert_eq!(
        addresses,
        vec![0x1000, 0x1004],
        "one line of one file, laid out twice, is two places to stop"
    );
}

// ---------------------------------------------------------------------------
// `incbin`
// ---------------------------------------------------------------------------

/// A Project whose `main.m68k` is `source` and which holds `sprite.bin`.
fn with_bytes(source: &str, bytes: Vec<u8>) -> Files {
    let mut files = Files::new();
    files.insert_text("main.m68k", source);
    files.insert_bytes("sprite.bin", bytes);
    files
}

#[test]
fn incbin_places_the_bytes_of_a_file_at_the_current_address() {
    let files = with_bytes(
        "    org $2000\n    incbin sprite.bin\n    dc.b $ff\n",
        vec![0xde, 0xad, 0xbe],
    );
    assert_eq!(
        memory(&files),
        vec![(0x2000, "deadbe".to_string()), (0x2003, "ff".to_string())]
    );
}

#[test]
fn incbin_is_a_dc_b_of_the_whole_file() {
    // The rule, written as the comparison it is: the same bytes written out by
    // hand assemble to the same memory, at the same addresses, alignment and
    // all.
    let bytes = vec![1, 2, 3, 4, 5];
    let files = with_bytes("    dc.b 0\nhere: incbin sprite.bin\n", bytes.clone());
    let written = Files::from_source("    dc.b 0\nhere: dc.b 1,2,3,4,5\n");
    assert_eq!(memory(&files), memory(&written));
    assert_eq!(symbol(&files, "here"), symbol(&written, "here"));
}

#[test]
fn a_label_on_an_incbin_names_its_first_byte_and_nothing_is_aligned() {
    let files = with_bytes(
        "    org $2000\n    dc.b 0\ndata: incbin sprite.bin\n",
        vec![0x10, 0x20],
    );
    assert_eq!(
        symbol(&files, "data"),
        0x2001,
        "an odd address is where the bytes go: `incbin` is a byte run and \
         aligns nothing"
    );
    assert_eq!(
        memory(&files),
        vec![(0x2000, "00".to_string()), (0x2001, "1020".to_string())]
    );
}

#[test]
fn incbin_of_a_text_file_contributes_its_latin_1_bytes() {
    let files = project(&[
        ("main.m68k", "    incbin note.txt\n"),
        ("note.txt", "città\n"),
    ]);
    assert_eq!(
        memory(&files),
        vec![(0x1000, "63697474e00a".to_string())],
        "`à` is one byte, $e0, and the newline is one byte (ADR 0004)"
    );
}

#[test]
fn a_character_above_latin_1_in_an_incbin_file_is_an_error_where_it_is() {
    let files = project(&[
        ("main.m68k", "    nop\n    incbin note.txt\n"),
        ("note.txt", "ok\nthen \u{2014}\n"),
    ]);
    let assembly = assemble(&files);
    assert_eq!(
        assembly
            .diagnostics
            .iter()
            .map(|diagnostic| format!(
                "{} {}:{}",
                diagnostic.code(),
                diagnostic.location.file,
                diagnostic.location.line
            ))
            .collect::<Vec<_>>(),
        vec!["character_above_latin1 note.txt:1"]
    );
    assert_eq!(
        assembly.diagnostics[0].related[0].1,
        "read into memory by this `incbin`"
    );
}

#[test]
fn incbin_of_a_missing_file_says_what_include_says() {
    let files = project(&[
        ("main.m68k", "    incbin pixels.bin\n"),
        ("data/pixels.bin", "\n"),
    ]);
    let assembly = assemble(&files);
    assert_eq!(assembly.diagnostics[0].code(), "unreadable_file");
    assert_eq!(
        assembly.diagnostics[0].hint().as_deref(),
        Some("Did you mean `data/pixels.bin`?"),
        "the same suggestions `include` gives"
    );
}

#[test]
fn incbin_needs_a_file_name() {
    let files = project(&[("main.m68k", "    incbin\n    include\n")]);
    assert_eq!(
        codes(&files),
        vec!["wrong_operand_count", "wrong_operand_count"]
    );
}

#[test]
fn incbin_inside_an_offset_region_produces_no_bytes_and_says_so() {
    let files = with_bytes(
        "    offset 0\n    incbin sprite.bin\n    org *\n",
        vec![1, 2],
    );
    assert_eq!(codes(&files), vec!["no_bytes_in_an_offset_region"]);
}

#[test]
fn an_incbin_of_a_source_file_is_allowed_and_an_include_of_a_binary_one_is_not() {
    // `incbin` takes either kind of File — "the data from the included file is
    // not processed in any way" — and `include` takes only source.
    let files = with_bytes("    incbin sprite.bin\n", vec![0xaa]);
    assert!(codes(&files).is_empty());
    let files = with_bytes("    include sprite.bin\n", vec![0xaa]);
    assert_eq!(codes(&files), vec!["unreadable_file"]);
}
