//! The Assembler: the front end that turns source Files into a Program and
//! Diagnostics.
//!
//! It replaces the regex lexer, the semantic checker and the compiler of 1.4.2
//! (`src/lexer.rs`, `src/semantic_checker.rs`, `src/compiler.rs`), which were
//! deleted once the Interpreter took the [`program::Program`].
//! The specification is [the design
//! record](../../docs/design/assembler-rewrite.md) and, for everything the
//! parser accepts, [the grammar](../../docs/grammar.md); the terms are the
//! glossary's ([CONTEXT.md](../../CONTEXT.md)).
//!
//! # What is here
//!
//! * [`source`] — the [`source::Files`] of a Project, the
//!   [`Span`](source::Span) of a token inside a line and the
//!   [`Location`](source::Location) a Diagnostic points at.
//! * [`diagnostics`] — the [`Severity`](diagnostics::Severity), the
//!   [`diagnostics::DiagnosticKind`] of every finding of every
//!   phase, and the [`diagnostics::Diagnostic`] that carries one.
//! * [`token`] — what a token is, and what a number, a quote and a name are
//!   made of.
//! * [`tokenizer`] — the walk over one line that produces them.
//! * [`ast`] — the tree one line parses into.
//! * [`include`](mod@include) — the Project as one assembled sequence of lines, `include`
//!   expanded and the Include chain of every one of them.
//! * [`names`] — the two questions the parser asks about an Operation's name,
//!   and the only two.
//! * [`parser`] — one line to an [`ast::Line`], against the grammar's sections
//!   2 and 3, with the Pratt loop for Expressions.
//! * [`instructions`] — the instruction table, the single source of truth of
//!   ADR 0003, the lowering from a checked Operation to an encoded
//!   [`Instruction`](instructions::Instruction), and the encoded types
//!   themselves.
//! * [`analyzer`] — every Operation judged against that table, and the
//!   Diagnostics that say what was found, what is allowed and what was probably
//!   meant.
//!
//! * [`expr`] — the value of an Expression, against the Symbols and the
//!   current address, with the Diagnostics of what it cannot work out.
//! * [`symbols`] — the four kinds of Symbol, their definitions, and the scope
//!   a Local label is read in.
//! * [`layout`] — the two passes: where every line goes, and what every line
//!   assembles to.
//! * [`program`] — the `Program` all of that builds, which is what the
//!   Interpreter runs.
//!
//! # What is still to come
//!
//! Every Directive and every Mnemonic of the design record is implemented, and
//! **nothing in the crate says "yet" any more**: `rte`, `stop`, `reset`,
//! `move usp,an`, macros with conditional assembly, `memory` and the
//! structured-control keywords are the whole of what is refused, each with the
//! reason it carries in the instruction table or in [`layout`]'s own list.
//! Real instruction sizes are the one decision the design record leaves open:
//! an instruction is four bytes here.

pub mod analyzer;
pub mod ast;
pub mod diagnostics;
pub mod expr;
pub mod include;
pub mod instructions;
pub mod layout;
pub mod names;
pub mod parser;
pub mod program;
pub mod source;
pub mod symbols;
pub mod token;
pub mod tokenizer;

use diagnostics::Diagnostic;
use program::Program;
use source::Files;

/// What the Assembler makes of a Project: everything it found, and the Program
/// when there is one.
///
/// `program` is `Some` exactly when no Diagnostic is an error (CONTEXT.md,
/// "Program"), so a caller that only checks the source — the asm-editor's live
/// checking — reads `diagnostics` and ignores the rest.
#[derive(Debug)]
pub struct Assembly {
    /// Everything every phase found, in source order.
    pub diagnostics: Vec<Diagnostic>,
    /// The Program, when the source builds one.
    pub program: Option<Program>,
}

impl Assembly {
    /// Whether any Diagnostic stops the Program from being built.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Assemble a Project, starting from the Entry file at `entry`.
///
/// This is the 2.0 API's `S68k.assemble`. It reads the Entry file and every
/// File the `include` Directives under it name ([`include::expand`]), parses
/// each of them once, lays the program out and assembles it; the Diagnostics of
/// every phase come back in one list, in the order of the assembled sequence,
/// and the Program comes back only when none of them is an error.
///
/// Assembly carries on after an error as far as it sensibly can (ADR 0003), so
/// a student sees every mistake of a build and not the first one. The one
/// failure that leaves nothing to assemble is an Entry file the Project has not
/// got, or one that holds bytes: that is one `unreadable_file` and no Program.
pub fn assemble(files: &Files, entry: &str) -> Assembly {
    let (unit, found) = match include::expand(files, entry) {
        Ok(expanded) => expanded,
        Err(diagnostic) => {
            return Assembly {
                diagnostics: vec![*diagnostic],
                program: None,
            }
        }
    };
    let (program, diagnostics) = layout::lay_out(&unit, found);
    let program = match diagnostics.iter().any(Diagnostic::is_error) {
        true => None,
        false => Some(program),
    };
    Assembly {
        diagnostics,
        program,
    }
}

/// Assemble one source string, which is the Project of one File the 2.0 API
/// wraps as [`DEFAULT_ENTRY_PATH`](source::DEFAULT_ENTRY_PATH).
pub fn assemble_source(text: &str) -> Assembly {
    assemble(&Files::from_source(text), source::DEFAULT_ENTRY_PATH)
}

/// Read one Source line into its four fields, for the editor's hover.
///
/// This is the 2.0 API's `parseLine`, which replaces 1.4.2's `lexOne`: the
/// [`Line`](ast::Line) it gives back carries the Label, the Operation with its
/// name and size, and every Operand with its Addressing mode and its
/// [`Span`](source::Span), so that the editor can say what the Operand under
/// the cursor is. It is [`Serialize`](serde::Serialize), which is how it
/// crosses into TypeScript.
///
/// Diagnostics are deliberately dropped here: hovering is not checking, and a
/// line out of its File cannot know a Symbol, an address or a Macro. Use
/// [`parser::parse_line`] to see them, and
/// [`parser::parse_file`] to read a whole File.
pub fn parse_line(text: &str) -> ast::Line {
    parser::parse_line(text, source::DEFAULT_ENTRY_PATH, 0).0
}
