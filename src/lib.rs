//! s68k: the M68K Assembler and Interpreter behind the asm-editor.
//!
//! Two halves, and the [`Program`] between them:
//!
//! * [`assembler`] reads the source Files of a Project and answers with the
//!   [`Diagnostic`]s it found and, when none of them is an error, the Program;
//! * [`interpreter`] runs that Program — registers, memory, the step, run,
//!   undo and interrupt operations — and reaches the source only through each
//!   instruction's [`Location`](assembler::source::Location).
//!
//! [`assembler::assemble`] and [`assembler::parse_line`] are the whole of the
//! entry to the first, and [`interpreter::Interpreter::new`] the whole of the
//! entry to the second.
//!
//! # The WebAssembly API, `@specy/s68k` 2.0
//!
//! This module is the boundary: everything JavaScript can reach is here or is
//! a `wasm_*` method of [`Interpreter`],
//! [`Cpu`](interpreter::Cpu) and [`Register`](interpreter::Register). The
//! regex lexer, the semantic checker and the compiler of 1.4.2 are gone, and
//! with them the `S68k` class that wrapped them, `semanticCheck`, `compile`,
//! `lex`, `lexOne` and `SemanticError`.
//!
//! | JavaScript | here |
//! |---|---|
//! | `wasm_assemble(files, entry)` | [`wasm_assemble`] |
//! | `assembly.wasm_get_diagnostics()` | [`WasmAssembly::wasm_get_diagnostics`] |
//! | `assembly.wasm_get_program_info()` | [`WasmAssembly::wasm_get_program_info`] |
//! | `assembly.wasm_get_instruction_addresses()` | [`WasmAssembly::wasm_get_instruction_addresses`] |
//! | `new Interpreter(assembly, options)` | [`Interpreter::wasm_new`] |
//! | `wasm_parse_line(text)` | [`wasm_parse_line`] |
//!
//! The three shapes that cross as plain objects — the Diagnostic, the
//! [`ParsedLine`] and the [`ProgramInfo`] — are serialised once, on their way
//! out, and their TypeScript declarations are in `src/ts_types.rs`. `ts-lib`
//! wraps this surface in the `S68k`, `Program` and `Interpreter` classes the
//! editor actually calls; nothing here knows about them.

pub mod assembler;
pub mod instructions;
pub mod interpreter;

pub mod debugger;
mod math;
mod ts_types;

#[cfg(test)]
mod test;

use std::collections::BTreeMap;

use js_sys::{Array, Object, Uint8Array};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::assembler::ast::{CommentKind, SizeSuffix};
use crate::assembler::diagnostics::Diagnostic;
use crate::assembler::names;
use crate::assembler::program::{Program, ProgramSymbol};
use crate::assembler::source::{self, Files, Span};
use crate::assembler::Assembly;
use crate::interpreter::{Interpreter, InterpreterOptions};

/// Turn a Rust panic into a readable JavaScript error, once per process.
///
/// 1.4.2 called `console_error_panic_hook::set_once()` at the top of every
/// exported method; this is that call, in one place, and it is a no-op when the
/// crate is built without the feature that provides the hook.
fn set_panic_hook() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Turn a serialisation failure at the boundary into a JavaScript error.
fn to_js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Serialise a value as the plain JavaScript data the 2.0 API promises.
///
/// It is `serde_wasm_bindgen` with one setting changed: a map crosses as a
/// plain object and not as a JavaScript `Map`, so that the Symbols of a Program
/// arrive as `{ start: {...} }` — readable, `JSON.stringify`-able, and the same
/// shape the fixtures record — rather than as something the caller has to know
/// to call `.get` on.
fn to_plain_js(value: &impl Serialize) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true))
        .map_err(to_js_error)
}

// ---------------------------------------------------------------------------
// assemble
// ---------------------------------------------------------------------------

/// What the Assembler made of a Project, held on the WebAssembly side.
///
/// It is [`Assembly`] behind a handle: the Diagnostics are read out of it as
/// plain objects, and the [`Program`] stays here, where the Interpreter can
/// take a copy of it without it ever crossing into JavaScript. One assembly
/// builds any number of Interpreters, which is what restarting a program does.
///
/// JavaScript owns the handle and must `free()` it; the `ts-lib` wrapper does
/// that for the caller as soon as it knows there is no Program to keep.
#[wasm_bindgen]
pub struct WasmAssembly {
    diagnostics: Vec<Diagnostic>,
    program: Option<Program>,
}

impl WasmAssembly {
    /// Wrap what [`assembler::assemble`] answered.
    pub fn new(assembly: Assembly) -> Self {
        Self {
            diagnostics: assembly.diagnostics,
            program: assembly.program,
        }
    }

    /// Everything the Assembler found, in source order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The Program, when the source built one.
    pub fn program(&self) -> Option<&Program> {
        self.program.as_ref()
    }
}

#[wasm_bindgen]
impl WasmAssembly {
    /// Every Diagnostic as a plain object
    /// (`{ severity, code, message, hint, location, related }`), in source
    /// order.
    pub fn wasm_get_diagnostics(&self) -> Result<JsValue, JsValue> {
        set_panic_hook();
        to_plain_js(&self.diagnostics)
    }

    /// Whether a Diagnostic stopped the Program from being built.
    pub fn wasm_has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Whether there is a Program, which is the same question the other way
    /// round: a Program exists exactly when no Diagnostic is an error.
    pub fn wasm_has_program(&self) -> bool {
        self.program.is_some()
    }

    /// What the built Program says about itself ([`ProgramInfo`]), or `null`
    /// when the source did not build one.
    ///
    /// The instructions themselves stay on this side: the Interpreter answers
    /// `wasm_get_instruction_at` for the one the editor is looking at, and a
    /// program of ten thousand instructions is not worth copying into
    /// JavaScript to show a symbol table.
    pub fn wasm_get_program_info(&self) -> Result<JsValue, JsValue> {
        set_panic_hook();
        match &self.program {
            Some(program) => to_plain_js(&ProgramInfo::of(program)),
            None => Ok(JsValue::NULL),
        }
    }

    /// Every assembled instruction address, in ascending order, or `null`
    /// when the source did not build a Program.
    ///
    /// This is a compact index for editor features that annotate the whole
    /// build. The full instructions remain on the WebAssembly side and are
    /// still read individually through the Interpreter.
    pub fn wasm_get_instruction_addresses(&self) -> Result<JsValue, JsValue> {
        set_panic_hook();
        match &self.program {
            Some(program) => to_plain_js(
                &program
                    .instructions()
                    .iter()
                    .map(|instruction| instruction.address)
                    .collect::<Vec<_>>(),
            ),
            None => Ok(JsValue::NULL),
        }
    }
}

/// What a built Program says about itself.
///
/// Serialised camelCase, as the whole Assembler surface is.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramInfo<'a> {
    /// The address the program starts running at.
    pub entry_point: usize,
    /// One past the last byte of the last instruction.
    pub end_address: usize,
    /// How many instructions were assembled.
    pub instruction_count: usize,
    /// Every Symbol, by full name.
    pub symbols: &'a BTreeMap<String, ProgramSymbol>,
}

impl<'a> ProgramInfo<'a> {
    /// Read a Program's own account of itself.
    pub fn of(program: &'a Program) -> Self {
        Self {
            entry_point: program.entry(),
            end_address: program.end_address(),
            instruction_count: program.instructions().len(),
            symbols: program.symbols(),
        }
    }
}

#[wasm_bindgen]
extern "C" {
    /// The Files of a Project as JavaScript hands them over: an object of
    /// root-relative path to a `string` (a source File) or a `Uint8Array` (a
    /// binary File).
    ///
    /// It is a JavaScript value with a TypeScript name: the declaration is
    /// `SourceFiles` in `src/ts_types.rs`, so that `wasm_assemble` says in the
    /// generated `.d.ts` what a Project is instead of taking `any`.
    #[wasm_bindgen(typescript_type = "SourceFiles")]
    pub type SourceFiles;
}

/// Assemble a Project: `files` is an object of path to `string` (a source File)
/// or `Uint8Array` (a binary File), and `entry` is the path of the Entry file.
///
/// This is `S68k.assemble`. It always answers a [`WasmAssembly`] — a source
/// that does not build asks for its Diagnostics and drops it — and throws only
/// when `files` is not the object it says it is, which is a mistake in the
/// caller and not in the program being assembled.
///
/// An Entry file that is missing, or that is binary, is itself a Diagnostic
/// (`unreadable_file`), so the editor's "the file you are looking at has been
/// renamed" is a message in the list like any other. So is a File named by an
/// `include` or an `incbin` line that the object has not got: everything the
/// assembly reads is in `files`, and nothing here touches a disk.
#[wasm_bindgen]
pub fn wasm_assemble(files: &SourceFiles, entry: &str) -> Result<WasmAssembly, JsValue> {
    set_panic_hook();
    let files = files_from_js(files.as_ref())?;
    Ok(WasmAssembly::new(assembler::assemble(&files, entry)))
}

/// Read a JavaScript object of path to `string | Uint8Array` as the Files of a
/// Project.
///
/// A `string` becomes a [`FileContent::Text`](assembler::source::FileContent)
/// and a `Uint8Array` — a Node `Buffer` among them, which is one — becomes a
/// [`FileContent::Bytes`](assembler::source::FileContent), which is the only
/// way a File of bytes reaches `incbin`. The paths are taken as they are
/// written and normalised by [`Files`] (`\` to `/`), so the editor may hand
/// over its own paths.
fn files_from_js(value: &JsValue) -> Result<Files, JsValue> {
    let object = value.dyn_ref::<Object>().ok_or_else(|| {
        JsValue::from_str("assemble expects an object of path -> string | Uint8Array")
    })?;
    let mut files = Files::new();
    for entry in Object::entries(object).iter() {
        let pair: Array = entry.unchecked_into();
        let path = pair.get(0).as_string().ok_or_else(|| {
            JsValue::from_str("assemble expects an object of path -> string | Uint8Array")
        })?;
        let content = pair.get(1);
        if let Some(text) = content.as_string() {
            files.insert_text(&path, text);
        } else if let Some(bytes) = content.dyn_ref::<Uint8Array>() {
            files.insert_bytes(&path, bytes.to_vec());
        } else {
            return Err(JsValue::from_str(&format!(
                "the file `{path}` is neither a string nor a Uint8Array"
            )));
        }
    }
    Ok(files)
}

// ---------------------------------------------------------------------------
// The Interpreter's constructor
// ---------------------------------------------------------------------------

#[wasm_bindgen]
impl Interpreter {
    /// An Interpreter over the Program of `assembly`, which is
    /// `new Interpreter(program, options)`.
    ///
    /// It **throws** when the assembly built no Program, because there is
    /// nothing to run and a caller that has not looked at its Diagnostics is a
    /// caller with a bug. `options` is 1.4.2's
    /// `{ keep_history, history_size }`; `null` and `undefined` mean the
    /// defaults.
    ///
    /// The Program is copied out of the assembly, so the same assembly can be
    /// run again — which is what restarting a program in the editor does — and
    /// so that nothing the Interpreter does can reach the Diagnostics beside
    /// it.
    #[wasm_bindgen(constructor)]
    pub fn wasm_new(assembly: &WasmAssembly, options: JsValue) -> Result<Interpreter, JsValue> {
        set_panic_hook();
        let program = assembly.program.clone().ok_or_else(|| {
            JsValue::from_str(
                "this source did not assemble: there is no program to interpret. \
                 Read the diagnostics first.",
            )
        })?;
        let options: Option<InterpreterOptions> = match options.is_undefined() || options.is_null()
        {
            true => None,
            false => Some(serde_wasm_bindgen::from_value(options).map_err(|error| {
                JsValue::from_str(&format!(
                    "invalid interpreter options, expected {{ keep_history, history_size }}: {error}"
                ))
            })?),
        };
        Ok(Interpreter::new(program, options))
    }
}

// ---------------------------------------------------------------------------
// parseLine
// ---------------------------------------------------------------------------

/// Read one Source line into its four fields, for the editor's hover.
///
/// This is `S68k.parseLine`, which replaces 1.4.2's `lexOne`. It never throws
/// and never reports anything: a line out of its File cannot know a Symbol, an
/// address or an instruction's operand rules, so what comes back is what was
/// *written* — the Label, the Operation with its size, every Operand with its
/// Addressing mode, and the Comment — each with the columns it covers, so that
/// the editor can say what is under the cursor.
#[wasm_bindgen]
pub fn wasm_parse_line(text: &str) -> Result<JsValue, JsValue> {
    set_panic_hook();
    to_plain_js(&ParsedLine::of(text))
}

/// A range of a line, in **characters**, as `parseLine` reports one.
///
/// Characters and not bytes, so that it lines up with the columns of a
/// [`Location`](assembler::source::Location) and with what an editor counts;
/// `end` is exclusive.
#[derive(Debug, Serialize)]
pub struct LineSpan {
    /// The column of the first character.
    pub start: usize,
    /// The column one past the last character.
    pub end: usize,
}

impl LineSpan {
    /// The columns `span` covers on `line`.
    fn of(line: &str, span: Span) -> Self {
        let (start, end) = source::columns_of(line, span);
        Self { start, end }
    }
}

/// What a Source line turns out to be.
///
/// The Operation decides it: a line with one is an `instruction` or a
/// `directive` by its name, and `unknown` when the name is neither — a typo, or
/// the Macro call this phase does not have. A line with no Operation is what is
/// left of it.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParsedLineKind {
    /// Nothing at all on the line.
    Blank,
    /// A Comment and nothing else.
    Comment,
    /// A Label and nothing else.
    Label,
    /// An Operation whose name is a Mnemonic.
    Instruction,
    /// An Operation whose name is a Directive's.
    Directive,
    /// An Operation whose name is neither.
    Unknown,
}

/// The Label field, as `parseLine` reports it.
#[derive(Debug, Serialize)]
pub struct ParsedLabel {
    /// The name, without the colon.
    pub name: String,
    /// Whether it was written with a colon.
    pub colon: bool,
    /// The columns the name covers.
    pub span: LineSpan,
}

/// One Operand, as `parseLine` reports it.
#[derive(Debug, Serialize)]
pub struct ParsedOperand {
    /// The Addressing mode, as the Program writes it: `immediate`,
    /// `data_register_direct`, `displacement`, …
    pub mode: String,
    /// The same in English ("an immediate"), which is what a Diagnostic about
    /// this Operand would call it.
    pub description: String,
    /// The Operand exactly as it was written.
    pub text: String,
    /// The columns it covers.
    pub span: LineSpan,
}

/// The raw Operand field of an Operation that takes one — the file name of
/// `include`, the message of `fail` — as `parseLine` reports it.
#[derive(Debug, Serialize)]
pub struct ParsedText {
    /// The text, exactly as it was written, without the Comment.
    pub text: String,
    /// The columns it covers.
    pub span: LineSpan,
}

/// The Operation field, as `parseLine` reports it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedOperation {
    /// The Mnemonic or Directive name, as it was written.
    pub name: String,
    /// The size suffix — `byte`, `word`, `long` or `short` — when one was
    /// written.
    pub size: Option<SizeSuffix>,
    /// The columns the name covers.
    pub name_span: LineSpan,
    /// The columns the size suffix covers, dot included.
    pub size_span: Option<LineSpan>,
    /// The columns the whole Operation covers, Operand field included.
    pub span: LineSpan,
    /// The Operands, in the order they were written.
    pub operands: Vec<ParsedOperand>,
    /// The raw Operand field, for the Operations that take one instead of
    /// Operands.
    pub text: Option<ParsedText>,
}

/// The Comment field, as `parseLine` reports it.
#[derive(Debug, Serialize)]
pub struct ParsedComment {
    /// `line` (the whole line is a Comment), `explicit` (`;` or `*`) or `bare`
    /// (EASy68K's markerless comment field).
    pub kind: CommentKind,
    /// The text, marker included.
    pub text: String,
    /// The columns it covers.
    pub span: LineSpan,
}

/// One Source line as `parseLine` answers it: what was written, and where.
///
/// It is a reading of [`Line`](assembler::ast::Line) and not the tree itself: the tree carries
/// every Expression of every Operand, which the editor has no use for and which
/// would be a TypeScript declaration nobody could keep true.
#[derive(Debug, Serialize)]
pub struct ParsedLine {
    /// What the line turned out to be.
    pub kind: ParsedLineKind,
    /// The Label it declares, if any.
    pub label: Option<ParsedLabel>,
    /// What it does, if anything.
    pub operation: Option<ParsedOperation>,
    /// Its Comment field, if any.
    pub comment: Option<ParsedComment>,
}

impl ParsedLine {
    /// Read `text` as one Source line.
    pub fn of(text: &str) -> Self {
        let line = assembler::parse_line(text);
        let kind = match &line.operation {
            Some(operation) => {
                let name = operation.lowercase_name();
                if names::is_mnemonic(&name) {
                    ParsedLineKind::Instruction
                } else if names::is_directive(&name) {
                    ParsedLineKind::Directive
                } else {
                    ParsedLineKind::Unknown
                }
            }
            None if line.label.is_some() => ParsedLineKind::Label,
            None if line.comment.is_some() => ParsedLineKind::Comment,
            None => ParsedLineKind::Blank,
        };
        Self {
            kind,
            label: line.label.map(|label| ParsedLabel {
                span: LineSpan::of(text, label.span),
                name: label.name,
                colon: label.colon,
            }),
            operation: line.operation.map(|operation| ParsedOperation {
                name: operation.name,
                size: operation.size,
                name_span: LineSpan::of(text, operation.name_span),
                size_span: operation.size_span.map(|span| LineSpan::of(text, span)),
                span: LineSpan::of(text, operation.span),
                operands: operation
                    .operands
                    .iter()
                    .map(|operand| ParsedOperand {
                        mode: operand.mode_name().to_string(),
                        description: operand.description().to_string(),
                        text: operand.span().text(text).to_string(),
                        span: LineSpan::of(text, operand.span()),
                    })
                    .collect(),
                text: operation.text.map(|field| ParsedText {
                    span: LineSpan::of(text, field.span),
                    text: field.text,
                }),
            }),
            comment: line.comment.map(|comment| ParsedComment {
                kind: comment.kind,
                span: LineSpan::of(text, comment.span),
                text: comment.text,
            }),
        }
    }
}

#[cfg(test)]
mod api_tests {
    use super::*;
    use serde_json::{json, Value};

    fn parsed(text: &str) -> Value {
        serde_json::to_value(ParsedLine::of(text)).expect("a parsed line serialises")
    }

    #[test]
    fn a_parsed_line_names_its_operands_and_where_they_are() {
        let line = parsed("start:  move.w #$10,(a0)+  ; go");
        assert_eq!(line["kind"], "instruction");
        assert_eq!(line["label"]["name"], "start");
        assert_eq!(line["label"]["colon"], true);
        assert_eq!(line["operation"]["name"], "move");
        assert_eq!(line["operation"]["size"], "word");
        assert_eq!(line["operation"]["operands"][0]["mode"], "immediate");
        assert_eq!(line["operation"]["operands"][0]["text"], "#$10");
        assert_eq!(line["operation"]["operands"][1]["mode"], "postincrement");
        assert_eq!(
            line["operation"]["operands"][1]["description"],
            "a postincrement operand"
        );
        assert_eq!(
            line["operation"]["operands"][1]["span"],
            json!({"start": 20, "end": 25})
        );
        assert_eq!(line["comment"]["kind"], "explicit");
        assert_eq!(line["comment"]["text"], "; go");
    }

    #[test]
    fn a_parsed_line_says_which_of_the_six_kinds_it_is() {
        assert_eq!(parsed("")["kind"], "blank");
        assert_eq!(parsed("   ")["kind"], "blank");
        assert_eq!(parsed("* a comment line")["kind"], "comment");
        assert_eq!(parsed("done:")["kind"], "label");
        assert_eq!(parsed("    rts")["kind"], "instruction");
        assert_eq!(parsed("    org $1000")["kind"], "directive");
        assert_eq!(parsed("    mvoe.w d0,d1")["kind"], "unknown");
    }

    #[test]
    fn a_parsed_span_counts_characters_and_not_bytes() {
        // `é` is two bytes of UTF-8 and one column, as a Location counts them.
        let line = parsed("    dc.b 'é',1");
        assert_eq!(line["operation"]["operands"][1]["text"], "1");
        assert_eq!(line["operation"]["operands"][1]["span"]["start"], 13);
    }

    #[test]
    fn a_line_read_as_text_keeps_it() {
        let line = parsed("    include 'io library.x68'");
        assert_eq!(line["kind"], "directive");
        assert_eq!(line["operation"]["operands"].as_array().unwrap().len(), 0);
        assert_eq!(line["operation"]["text"]["text"], "'io library.x68'");
    }

    #[test]
    fn parse_line_answers_a_shape_for_anything() {
        for text in [
            "",
            ";",
            "    move.w",
            "    move.w #,",
            "label",
            "    dc.b 'unterminated",
            "  ((((((((",
            "\t\tmove.l\t(a0,d1.w),-(sp)",
        ] {
            let line = parsed(text);
            assert!(
                line.get("kind").is_some(),
                "every line has a kind: {text:?}"
            );
        }
    }

    #[test]
    fn the_program_info_is_what_a_program_says_about_itself() {
        let assembly = assembler::assemble_source("    org $1000\nstart:  nop\n    end start\n");
        assert!(
            assembly.diagnostics.is_empty(),
            "{:?}",
            assembly.diagnostics
        );
        let assembly = WasmAssembly::new(assembly);
        let program = assembly.program().expect("a program");
        let info = serde_json::to_value(ProgramInfo::of(program)).expect("info serialises");
        assert_eq!(info["entryPoint"], 0x1000);
        assert_eq!(info["endAddress"], 0x1004);
        assert_eq!(info["instructionCount"], 1);
        assert_eq!(info["symbols"]["start"]["value"], 0x1000);
        assert_eq!(info["symbols"]["start"]["kind"], "label");
        assert_eq!(info["symbols"]["start"]["location"]["line"], 1);
        assert_eq!(
            info["symbols"]["start"]["location"]["endColumn"], 5,
            "a Location is camelCase wherever it appears"
        );
    }

    #[test]
    fn an_assembly_with_an_error_carries_diagnostics_and_no_program() {
        let assembly = WasmAssembly::new(assembler::assemble_source("    move.w d0,#1\n"));
        assert!(assembly.program().is_none());
        let diagnostics =
            serde_json::to_value(assembly.diagnostics()).expect("diagnostics serialise");
        assert_eq!(diagnostics[0]["severity"], "error");
        assert_eq!(diagnostics[0]["code"], "invalid_addressing_mode");
        assert_eq!(diagnostics[0]["location"]["line"], 0);
        assert!(diagnostics[0]["location"]["endColumn"].is_number());
    }
}
