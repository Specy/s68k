//! One golden case per Diagnostic the Assembler can raise.
//!
//! The design record's "Tests" item 3, and the acceptance test of [ADR
//! 0003](../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md):
//! every [`DiagnosticKind`](crate::assembler::diagnostics::DiagnosticKind) has
//! a small program in `tests/diagnostics/<code>.asm` that raises it, and the
//! snapshot beside it is the whole list of Diagnostics that program produces —
//! severity, code, message, hint, location and related locations, exactly as
//! the TypeScript side receives them.
//!
//! The snapshots are meant to be **read**: a message that does not tell a
//! student what to do about the line is a bug in the message, and the diff of
//! one of these files is where that shows.
//!
//! A case is one of two things:
//!
//! * `tests/diagnostics/<code>.asm`, one File of source, which is the shape
//!   every case had before phase 4;
//! * `tests/diagnostics/<code>/`, a **Project**: every file under it is a File,
//!   named by its path inside the directory, and `main.asm` is the Entry file.
//!   A `.bin` file is a binary File. This is what a case about `include` needs,
//!   since one File cannot raise a Diagnostic about another.
//!
//! [`every_diagnostic_kind_has_a_case`] is what keeps the set complete: a new
//! kind fails it until it is given a program or written into
//! [`WITHOUT_A_CASE`].

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::assembler::diagnostics::{Diagnostic, ALL_CODES};
use crate::assembler::source::Files;

/// The codes no case in this directory can raise, and why.
///
/// **None**, since phase 4. `unreadable_file` was the last one — no single File
/// of source could name another File to fail to read — and the Project-shaped
/// cases below give it one; `register_list_in_expression` was the one before
/// that, until phase 2's `reg` made a Register list something a File can
/// define.
const WITHOUT_A_CASE: [&str; 0] = [];

/// Everything the Assembler finds in one File, in source order.
///
/// This is [`assemble`](crate::assembler::assemble) over a Project of one File:
/// the whole pipeline, parser, Layout, evaluator and analyzer alike. The
/// snapshots beside the cases are what it answers, which is what the TypeScript
/// side receives.
fn diagnostics_of(file: &str, text: &str) -> Vec<Diagnostic> {
    let mut files = Files::new();
    files.insert_text(file, text);
    crate::assembler::assemble(&files, file).diagnostics
}

/// Everything the Assembler finds in a Project-shaped case.
///
/// Every file under the case directory is a File of the Project, named by its
/// path inside it, and `main.asm` — or `main.bin`, which is how the case for an
/// Entry file that holds bytes is written — is the Entry file.
fn diagnostics_of_project(directory: &Path) -> Vec<Diagnostic> {
    let mut files = Files::new();
    for path in project_files(directory) {
        let name = path
            .strip_prefix(directory)
            .expect("a path inside the case directory")
            .to_str()
            .expect("a file name")
            .replace('\\', "/");
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("bin") => {
                files.insert_bytes(&name, fs::read(&path).expect("a readable case file"));
            }
            _ => {
                files.insert_text(
                    &name,
                    fs::read_to_string(&path).expect("a readable case file"),
                );
            }
        }
    }
    let entry = match files.contains("main.asm") {
        true => "main.asm",
        false => "main.bin",
    };
    crate::assembler::assemble(&files, entry).diagnostics
}

/// Every file under a case directory, in sorted order, subdirectories included.
fn project_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut directories = vec![directory.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|e| panic!("cannot read {}: {}", directory.display(), e))
        {
            let path = entry.expect("a readable directory entry").path();
            match path.is_dir() {
                true => directories.push(path),
                false => files.push(path),
            }
        }
    }
    files.sort();
    files
}

/// The `tests/diagnostics` directory.
fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("diagnostics")
}

/// Every case, by name, so that the order does not depend on the file system.
///
/// A case is a `.asm` file or a directory; `snapshots/` is neither.
fn case_files() -> Vec<PathBuf> {
    let directory = cases_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", directory.display(), e))
        .map(|entry| entry.expect("a readable directory entry").path())
        .filter(|path| match path.is_dir() {
            true => path.file_name().and_then(|name| name.to_str()) != Some("snapshots"),
            false => path.extension().and_then(|e| e.to_str()) == Some("asm"),
        })
        .collect();
    files.sort();
    files
}

/// The code a case file is named after.
fn code_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .expect("a case file name")
        .to_string()
}

fn settings(path: &Path) -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(cases_dir().join("snapshots"));
    settings.set_prepend_module_to_snapshot(false);
    settings.set_input_file(path);
    settings
}

/// Every case program raises the code it is named after, and this is everything
/// it raises.
#[test]
fn diagnostic_cases() {
    for path in case_files() {
        let code = code_of(&path);
        let diagnostics = match path.is_dir() {
            true => diagnostics_of_project(&path),
            false => {
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
                diagnostics_of(&format!("{code}.asm"), &text)
            }
        };
        let codes: Vec<&str> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect();
        assert!(
            codes.contains(&code.as_str()),
            "tests/diagnostics/{code} is the case for `{code}` and raises {codes:?}"
        );
        settings(&path).bind(|| {
            insta::assert_json_snapshot!(code, diagnostics);
        });
    }
}

/// Every kind has a case, and every case is named after a kind.
///
/// This is the acceptance test of ADR 0003: the Diagnostics are the product, so
/// a kind with no program that raises it has never been read by anyone.
#[test]
fn every_diagnostic_kind_has_a_case() {
    let cases: Vec<String> = case_files().iter().map(|path| code_of(path)).collect();
    for code in ALL_CODES {
        if WITHOUT_A_CASE.contains(code) {
            assert!(
                !cases.contains(&code.to_string()),
                "`{code}` has a case, so it is reachable: take it out of WITHOUT_A_CASE"
            );
            continue;
        }
        assert!(
            cases.contains(&code.to_string()),
            "`{code}` has no case; write tests/diagnostics/{code}.asm, or a \
             tests/diagnostics/{code}/ project when one file cannot raise it"
        );
    }
    for case in &cases {
        assert!(
            ALL_CODES.contains(&case.as_str()),
            "tests/diagnostics/{case} is named after no diagnostic code"
        );
    }
}

/// The analyzer has nothing to say about any program the asm-editor ships.
///
/// The strongest evidence there is that the instruction table is the old
/// checker's superset where it matters: all 30 programs of `tests/corpus/editor`
/// assemble on 1.4.2, and not one line of them trips a rule of the new table.
/// A change to the table that starts refusing real code fails here, and the
/// corpus fixtures beside it say what the old pipeline made of the same lines.
#[test]
fn the_analyzer_is_silent_on_every_editor_program() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("editor");
    let mut programs: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", directory.display(), e))
        .map(|entry| entry.expect("a readable directory entry").path())
        .filter(|path| match path.extension().and_then(|e| e.to_str()) {
            Some(extension) => matches!(extension.to_lowercase().as_str(), "asm" | "x68"),
            None => false,
        })
        .collect();
    programs.sort();
    assert_eq!(
        programs.len(),
        30,
        "the corpus holds the 30 editor programs"
    );
    for path in programs {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a file name")
            .to_string();
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
        let found: Vec<String> = diagnostics_of(&name, &text)
            .iter()
            .map(|diagnostic| {
                format!(
                    "{}:{}: {}",
                    name,
                    diagnostic.location.line + 1,
                    diagnostic.message()
                )
            })
            .collect();
        assert!(found.is_empty(), "{found:#?}");
    }
}

/// A well-formed program raises nothing at all.
///
/// The corpus proves this over thirty real programs; this is the small version
/// of it, and it is here so that a change to a message or a rule that starts
/// firing on correct code fails a test named after the mistake.
#[test]
fn a_correct_program_is_silent() {
    let source = "\
* A correct program, and the assembler has nothing to say about it.
start:
    move.l #10,d0
    lea buffer,a0
    movem.l d0-d2/a0,-(a7)
    asl.l #2,d0
    cmp.b (a0)+,(a1)+
    beq.s done
    dbra d0,start
done:
    movem.l (a7)+,d0-d2/a0
    trap #15
    rts
buffer: ds.b 8
";
    let diagnostics = diagnostics_of("main.m68k", source);
    let codes: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code())
        .collect();
    assert!(codes.is_empty(), "a correct program raised {codes:?}");
}

/// The three EASy68K originals raise nothing but the features s68k does not
/// implement.
///
/// The design record's "Tests" item 2. On 1.4.2 these three raised 88, 138 and
/// 64 errors, nearly all of them the old checker misreading a Label in column 1
/// (`tests/corpus/README.md`, "Diagnostics fixtures"). What is left is counted
/// below, code by code, and not one of them is a syntax error:
///
/// * `unimplemented_operation` — the Macro definition, every invocation of it,
///   the conditional and structured-control keywords, and `rte`. `simhalt` was
///   among them until phase 2 implemented it, and `andi.w #$00,SR` raised
///   `unimplemented_addressing_mode` until phase 3 implemented the status
///   register;
/// * `bare_comment` — the once-a-File suggestion for EASy68K's own comment
///   field;
/// * `entry_point_case_mismatch` — a **warning**: `mouseWindowSize.X68` writes
///   its Label `start` and its last line `END START`, which is the evidence
///   that EASy68K's symbol look-up is not case sensitive. The Entry point is
///   taken from the Label, the program builds, and the warning says the case
///   differs (`layout::entry_by_case`, ADR 0001).
#[test]
fn the_easy68k_originals_raise_only_what_is_not_implemented() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("easy68k");
    let mut programs: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", directory.display(), e))
        .map(|entry| entry.expect("a readable directory entry").path())
        .collect();
    programs.sort();
    let mut found = Vec::new();
    for path in programs {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a file name")
            .to_string();
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
        let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
        for diagnostic in diagnostics_of(&name, &text) {
            *counted.entry(diagnostic.code()).or_default() += 1;
        }
        let counts: Vec<String> = counted
            .iter()
            .map(|(code, count)| format!("{code} {count}"))
            .collect();
        found.push(format!("{name}: {}", counts.join(", ")));
    }
    assert_eq!(
        found,
        vec![
            "clockDigital.X68: bare_comment 1, unimplemented_operation 14",
            "graphicSound.X68: bare_comment 1, unimplemented_operation 2",
            "mouseWindowSize.X68: entry_point_case_mismatch 1, unimplemented_operation 18",
        ]
    );
}
