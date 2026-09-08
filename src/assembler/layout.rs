//! The Layout: two passes over the lines, and the Program they build.
//!
//! EASy68K assembles in two passes and so does this. **Pass 1** walks the lines
//! in order, follows the Directives that decide where things go, defines every
//! Symbol and gives every line that produces bytes an address. **Pass 2** walks
//! them again, now that every name has a value: it evaluates the Operands,
//! runs the [analyzer](super::analyzer) over every Operation and lowers what
//! survives into the encoded instructions and the bytes of memory.
//!
//! The split is what the forward-reference rule is made of (CONTEXT.md,
//! "Forward reference"): a value pass 1 needs — an `org` address, a `ds` or
//! `dcb` count, an `equ` or a `set` — cannot wait for a definition further
//! down, and one pass 2 needs — an instruction Operand, a `dc` item — can.
//!
//! # The rules this module implements
//!
//! * The default origin is `$1000` and `org` may go anywhere, forwards or
//!   backwards. An odd origin is a warning and rounds up.
//! * An instruction is 4 bytes and starts on an even address.
//! * `dc`, `ds` and `dcb` default to `.w`; a word or a long one starts on an
//!   even address, a byte one wherever it is (`Directives/ds.htm`,
//!   `Directives/dc.htm`). A `dc.w` after an odd `dc.b` is therefore padded,
//!   which 1.4.2 did not do.
//! * `ds` reserves its room and writes nothing (`Directives/ds.htm`); 1.4.2
//!   wrote zeros over an eighth of it.
//! * Two lines laid out over the same address are an error at the second, with
//!   the first as a related Location (the design record, "Layout").
//! * The Entry point is `end`'s Operand, else a Label named `START`, else the
//!   first instruction (CONTEXT.md, "Entry point").
//! * A line after `end` is not assembled, and the first one carrying a Label or
//!   an Operation says so.
//! * `reg` names a `movem` register list, `fail` reports the rest of its line
//!   as the program's own error, and `simhalt` is an executable item of four
//!   bytes that ends the run.
//! * A Directive that gives a name to something needs a Label and `page` and
//!   the conditional-assembly Directives take none (`label_rule_of`).
//! * `section` picks one of sixteen location counters and `offset` opens a
//!   region that moves an address and places nothing, both of them read back
//!   through the one current address (`Directives/section.htm`,
//!   `Directives/offset.htm`).

use std::collections::HashSet;

use super::analyzer::{Analyzer, Context, DEFAULT_ORIGIN};
use super::ast::{
    Expr, Line, Operand, Operation, Register, RegisterKind, RegisterListItem, SizeSuffix,
};
use super::diagnostics::{Diagnostic, DiagnosticKind};
use super::expr::{self, Site};
use super::instructions::encoded::Instruction;
use super::instructions::lowering;
use super::instructions::table;
use super::names;
use super::parser::ParsedFile;
use super::program::{AssembledInstruction, MemoryContent, MemoryRun, Program};
use super::source::{Location, SourceFile, Span};
use super::symbols::{self, SymbolKind, SymbolTable, SymbolValue};

/// The whole of the 68000's address space as s68k simulates it, 16 MB.
///
/// Nothing may be laid out past it: the Interpreter's memory is exactly this
/// long, and an address beyond it is a mistake in the source rather than a
/// program that needs a bigger machine.
pub const ADDRESS_SPACE: i64 = 0x0100_0000;

/// How many bytes every instruction takes up.
///
/// The real sizes are a later decision (the design record, "Scope"); the
/// Program stores a size per instruction so that the decision is this constant
/// and not a rewrite.
pub const INSTRUCTION_SIZE: usize = 4;

/// How many location counters a program has, one for each `section`.
///
/// EASy68K's own range: "`<number>` must be in the range 0..15. No section
/// numbers are reserved in any way. By default, the assembler will begin with
/// section 0" (`Directives/section.htm`).
pub const SECTION_COUNT: usize = 16;

/// What a `fail` with no message says, word for word EASy68K's own
/// ("If no message is provided the message: \"ERROR: Unspecified user defined
/// error.\" is used", `Directives/fail.htm`); the "ERROR:" of it is that
/// assembler's prefix on every error and is the [`Severity`] here.
///
/// [`Severity`]: super::diagnostics::Severity
pub const UNSPECIFIED_FAILURE: &str = "Unspecified user defined error";

/// Lay one File out and assemble it.
///
/// The Program is always built, from whatever the two passes could make of the
/// lines; the caller ([`assemble`](super::assemble)) is what decides whether it
/// may be handed out, which is only when no Diagnostic is an error.
pub fn lay_out(source: &SourceFile, parsed: &ParsedFile) -> (Program, Vec<Diagnostic>) {
    let mut layout = Layout::new(source, parsed);
    layout.pass_one();
    layout.pass_two();
    layout.finish()
}

/// What one Source line puts in the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// Nothing at all: a blank line, a Comment, a Label on its own, `equ`, and
    /// every Directive that produces no bytes.
    Nothing,
    /// An instruction, [`INSTRUCTION_SIZE`] bytes at the line's address.
    Instruction,
    /// `dc` or `dcb`: bytes, whose values pass 2 works out.
    Data(usize),
    /// `ds`: room reserved and not written.
    Reserved(usize),
}

/// Where a line sits and what it puts there, worked out by pass 1 and read by
/// pass 2.
#[derive(Debug, Clone)]
struct LinePlan {
    /// The address the line's item is laid out at, alignment applied.
    address: i64,
    /// The Global label in force on the line, which is what its Local labels
    /// are scoped by.
    scope: Option<String>,
    /// What the line puts in the program.
    item: Item,
}

/// What [`Layout::resolve_register_lists`] made of a line's Operands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RegisterLists {
    /// A `reg` Symbol was read: these are the Operands to judge, with the name
    /// replaced by the register list it stands for.
    Resolved(Vec<Operand>),
    /// A name where a register list belongs is not one, and has been reported.
    Refused,
}

/// One run of addresses a line takes up, for the overlap check.
struct Placement {
    start: i64,
    end: i64,
    line: usize,
}

/// The two passes, and everything they work out on the way.
struct Layout<'a> {
    source: &'a SourceFile<'a>,
    parsed: &'a ParsedFile,
    diagnostics: Vec<Diagnostic>,
    symbols: SymbolTable,
    /// Every full name the File defines, collected before pass 1, which is what
    /// tells a forward reference from a name that is defined nowhere.
    declared: HashSet<String>,
    plans: Vec<LinePlan>,
    placements: Vec<Placement>,
    /// The address pass 1 has reached in each of the sixteen sections;
    /// [`Layout::address`] reads the one in force out of it. Section 0 starts
    /// at the default origin and the other fifteen at zero
    /// ([`sections_at_the_start`]).
    sections: [i64; SECTION_COUNT],
    /// Which section is in force, `0..SECTION_COUNT` (`Directives/section.htm`).
    section: usize,
    /// The counter of an open `offset` region, which **shadows** the section's
    /// own while it is open: everything that would have placed bytes moves this
    /// instead and places nothing, so closing the region leaves the section's
    /// counter exactly where `offset` found it, which is the address `org *`
    /// restores (`Directives/offset.htm`).
    offset: Option<i64>,
    /// The address the first item of the program is laid out at, which is what
    /// tells `move.l 5,d0` from `move.l label,d0`.
    origin: Option<i64>,
    /// The Global label in force.
    scope: Option<String>,
    /// `end`'s Operand, and the line it is on.
    end: Option<(Expr, usize)>,
    /// Whether `end` has been read, after which nothing is assembled.
    ended: bool,
    /// Whether the `code_after_end` warning has been given.
    warned_after_end: bool,
    instructions: Vec<AssembledInstruction>,
    memory: Vec<MemoryRun>,
}

impl<'a> Layout<'a> {
    fn new(source: &'a SourceFile<'a>, parsed: &'a ParsedFile) -> Self {
        Self {
            source,
            parsed,
            diagnostics: Vec::new(),
            symbols: SymbolTable::new(),
            declared: declared_names(parsed),
            plans: Vec::with_capacity(parsed.lines.len()),
            placements: Vec::new(),
            sections: sections_at_the_start(),
            section: 0,
            offset: None,
            origin: None,
            scope: None,
            end: None,
            ended: false,
            warned_after_end: false,
            instructions: Vec::new(),
            memory: Vec::new(),
        }
    }

    // -- the two passes ----------------------------------------------------

    /// Pass 1: the Symbols, the addresses, and everything the Layout depends
    /// on.
    fn pass_one(&mut self) {
        for index in 0..self.parsed.lines.len() {
            let plan = self.plan_line(index);
            let plan = self.hold_back_in_an_offset_region(index, plan);
            self.plans.push(plan);
        }
        self.report_overlaps();
    }

    /// Pass 2: the values, the checks and the encoded instructions.
    fn pass_two(&mut self) {
        for index in 0..self.parsed.lines.len() {
            self.assemble_line(index);
        }
    }

    /// The Program and every Diagnostic of both passes, in source order.
    fn finish(mut self) -> (Program, Vec<Diagnostic>) {
        let entry = self.entry_point();
        let program = Program::new(
            std::mem::take(&mut self.instructions),
            std::mem::take(&mut self.memory),
            &self.symbols,
            entry,
        );
        let mut diagnostics = self.diagnostics;
        diagnostics
            .sort_by_key(|diagnostic| (diagnostic.location.line, diagnostic.location.column));
        (program, diagnostics)
    }

    // -- pass 1 ------------------------------------------------------------

    /// Read one line: its Label, its Operation, and where what it produces
    /// goes.
    fn plan_line(&mut self, index: usize) -> LinePlan {
        let line = self.line(index);
        if self.ended {
            // `end` is the last line the assembler reads. A Comment or a blank
            // line after it is what every EASy68K program has, and says
            // nothing.
            if !self.warned_after_end && (line.label.is_some() || line.operation.is_some()) {
                self.warned_after_end = true;
                let location = Location::whole_line(self.source.path(), index, self.text(index));
                self.diagnostics
                    .push(Diagnostic::new(DiagnosticKind::CodeAfterEnd, location));
            }
            return self.nothing();
        }
        let operation = match &line.operation {
            None => {
                self.define_label(index);
                return self.nothing();
            }
            Some(operation) => operation,
        };
        let name = operation.lowercase_name();
        if table::lookup(&name).is_some() {
            return self.plan_instruction(index);
        }
        if names::is_directive(&name) {
            return self.plan_directive(index, &name);
        }
        // A word this assembler does not know: the analyzer says so in pass 2,
        // and the Label on the line still names the address it is at.
        self.define_label(index);
        self.nothing()
    }

    /// An instruction: 4 bytes, on an even address.
    fn plan_instruction(&mut self, index: usize) -> LinePlan {
        self.align(2);
        self.define_label(index);
        let address = self.address();
        self.place(index, INSTRUCTION_SIZE);
        self.plan(address, Item::Instruction)
    }

    /// One Directive, in pass 1.
    fn plan_directive(&mut self, index: usize, name: &str) -> LinePlan {
        self.check_label_rule(index, name);
        match name {
            "org" => self.plan_org(index),
            "equ" | "set" => self.plan_equate(index, name),
            "dc" => self.plan_dc(index),
            "ds" => self.plan_ds(index),
            "dcb" => self.plan_dcb(index),
            "end" => self.plan_end(index),
            "reg" => self.plan_reg(index),
            "fail" => self.plan_fail(index),
            "simhalt" => self.plan_simhalt(index),
            "offset" => self.plan_offset(index),
            "section" => self.plan_section(index),
            // `opt`, `list`, `nolist` and `page` are display settings and are
            // accepted with nothing to say (the design record, "Directives").
            "opt" | "list" | "nolist" | "page" => {
                self.define_label(index);
                self.nothing()
            }
            _ => {
                self.unimplemented_directive(index, name);
                self.define_label(index);
                self.nothing()
            }
        }
    }

    /// `org expr`: the address the next item goes to, and the end of an
    /// `offset` region.
    ///
    /// The `*` of `org (*+1)&-2` is the address `org` is about to leave, so the
    /// value is worked out before anything moves. Inside an `offset` region it
    /// is something else: "ORG * restores the code to the address in use prior
    /// to the OFFSET" (`Directives/offset.htm`), so in this one Operand `*` is
    /// the address the region is shadowing and not the region's own counter.
    /// Every other `*` inside a region — `here equ *`, `dc.l *` — is the
    /// counter, because that is what the current address means where the
    /// Labels are offsets.
    fn plan_org(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "org");
        let Some(operand) = self.one_operand(index, "org") else {
            self.close_the_offset_region();
            self.define_label(index);
            return self.nothing();
        };
        let star = match self.offset {
            Some(_) => self.resume_address(),
            None => self.address(),
        };
        let value = self.value_now_at(index, "org", &operand, star);
        // Whatever the address turns out to be, the `offset` region ends here:
        // `org` is the Directive the help names for it, and a region left open
        // would swallow the rest of the File.
        self.close_the_offset_region();
        if let Some(value) = value {
            let span = operand.span();
            if !(0..ADDRESS_SPACE).contains(&value) {
                self.raise(
                    index,
                    span,
                    DiagnosticKind::ValueOutOfRange {
                        subject: "the address of `org`".to_string(),
                        value,
                        min: 0,
                        max: ADDRESS_SPACE - 1,
                        advice: Some("s68k has 16 MB of memory".to_string()),
                    },
                );
            } else {
                // An `org` that lands where the address already is moves
                // nothing, so there is nothing to round up and nothing to say —
                // and `org *` after a `dc.b`, which is how an `offset` region
                // is ended, would otherwise be told its own address is odd.
                let address = match value % 2 == 0 || value == star {
                    true => value,
                    false => {
                        self.raise(index, span, DiagnosticKind::OddOrigin { address: value });
                        value + 1
                    }
                };
                self.set_address(address);
            }
        }
        // The Label of an `org` line names where the program goes on from, not
        // where it was.
        self.define_label(index);
        self.nothing()
    }

    /// `label equ expr` and `label set expr`.
    fn plan_equate(&mut self, index: usize, name: &str) -> LinePlan {
        self.no_size(index, name);
        // `check_label_rule` has already said that a value Directive needs a
        // name, so a line with none is simply not defined here.
        let line = self.line(index);
        let Some(label) = line.label.as_ref() else {
            return self.nothing();
        };
        let (label_name, label_span) = (label.name.clone(), label.span);
        // The name is defined whatever the value turns out to be: a value that
        // cannot be worked out has already been reported, and leaving the name
        // undefined would report it again at every use of it.
        let value = match self.one_operand(index, name) {
            Some(operand) => self.value_now(index, name, &operand).unwrap_or(0),
            None => 0,
        };
        let kind = match name {
            "set" => SymbolKind::Variable,
            _ => SymbolKind::Constant,
        };
        self.define(
            index,
            &label_name,
            label_span,
            kind,
            SymbolValue::Number(value),
        );
        self.nothing()
    }

    /// `dc.size item,item,…`: the bytes are pass 2's, the length is pass 1's.
    fn plan_dc(&mut self, index: usize) -> LinePlan {
        let size = self.data_size(index, "dc");
        self.align(alignment_of(size));
        self.define_label(index);
        let address = self.address();
        let line = self.line(index);
        let operands = line
            .operation
            .as_ref()
            .map(|operation| operation.operands.as_slice())
            .unwrap_or_default();
        if operands.is_empty() {
            // `dc` takes a *list*, so its count is a minimum: `dc.b 1,2,3` is
            // three Operands and assembles.
            self.wrong_item_count(index, &format!("dc.{}", size.letter()));
            return self.plan(address, Item::Data(0));
        }
        let length: usize = operands.iter().map(|item| item_length(item, size)).sum();
        self.place(index, length);
        self.plan(address, Item::Data(length))
    }

    /// `ds.size count`: room reserved and not written.
    fn plan_ds(&mut self, index: usize) -> LinePlan {
        let size = self.data_size(index, "ds");
        self.align(alignment_of(size));
        self.define_label(index);
        let address = self.address();
        let name = format!("ds.{}", size.letter());
        let Some(operand) = self.one_operand(index, &name) else {
            return self.plan(address, Item::Reserved(0));
        };
        let count = self.count_now(index, &name, &operand).unwrap_or(0);
        let length = count * bytes_of(size);
        self.place(index, length);
        self.plan(address, Item::Reserved(length))
    }

    /// `dcb.size count,value`: `count` copies of `value`.
    fn plan_dcb(&mut self, index: usize) -> LinePlan {
        let size = self.data_size(index, "dcb");
        self.align(alignment_of(size));
        self.define_label(index);
        let address = self.address();
        let operands = self.operands(index);
        let name = format!("dcb.{}", size.letter());
        if operands.len() != 2 {
            self.wrong_operand_count(index, &name, operands.len(), vec![2]);
            return self.plan(address, Item::Data(0));
        }
        let count = self.count_now(index, &name, &operands[0]).unwrap_or(0);
        let length = count * bytes_of(size);
        self.place(index, length);
        self.plan(address, Item::Data(length))
    }

    /// `end [expr]`: the Entry point, and the last line that is assembled.
    fn plan_end(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "end");
        // `end` ends an open `offset` region as `org` does. The help says only
        // that an `org` ends one, but nothing after `end` is assembled anyway,
        // and a Label on the `end` line itself is an address and not an offset.
        self.close_the_offset_region();
        self.define_label(index);
        let operands = self.operands(index);
        match operands.len() {
            0 => {
                let span = self.operation_name_span(index);
                self.raise(index, span, DiagnosticKind::EndWithoutAnAddress);
            }
            1 => {
                // The Entry point does not decide the Layout, so its value is
                // pass 2's and a forward reference is allowed in it.
                if let Some(value) = self.expression_of(index, "end", &operands[0]) {
                    self.end = Some((value, index));
                }
            }
            found => self.wrong_operand_count(index, "end", found, vec![0, 1]),
        }
        self.ended = true;
        self.nothing()
    }

    /// `label reg d0-d3/a0-a2`: a name for a `movem` register list.
    ///
    /// `Directives/reg.htm` gives the whole of it — `AllRegs REG D0-D7/A0-A6`,
    /// then `MOVEM.L AllRegs,-(SP)`. The Symbol holds the mask and not a
    /// number: a Register list has no value in an Expression, which is what
    /// [`RegisterListInExpression`](DiagnosticKind::RegisterListInExpression)
    /// answers, and [`Layout::resolve_register_lists`] is where `movem` reads
    /// it back.
    fn plan_reg(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "reg");
        // `check_label_rule` has already said that `reg` needs a name.
        let Some(label) = self.line(index).label.as_ref() else {
            return self.nothing();
        };
        let (name, span) = (label.name.clone(), label.span);
        let mask = match self.one_operand(index, "reg") {
            Some(operand) => self.register_mask_of(index, &operand),
            None => None,
        };
        // The name is defined whatever the list turns out to be, for the same
        // reason `equ` defines its name: a list that could not be read has been
        // reported once already, and leaving the name undefined would report it
        // again at every `movem` that uses it.
        self.define(
            index,
            &name,
            span,
            SymbolKind::RegisterList,
            SymbolValue::RegisterList(mask.unwrap_or(0)),
        );
        self.nothing()
    }

    /// The `movem` mask an Operand stands for.
    ///
    /// A register list is one; so is a single register, which is a list of one
    /// (`docs/grammar.md` 1.12, and `movem.l d1,-(a7)` is line 178 of
    /// `tests/corpus/easy68k/clockDigital.X68`). Anything else is
    /// `register_list_expected`.
    fn register_mask_of(&mut self, index: usize, operand: &Operand) -> Option<u16> {
        match operand {
            Operand::RegisterList { items, .. } => Some(lowering::register_mask(items)),
            Operand::DataRegisterDirect { register, .. }
            | Operand::AddressRegisterDirect { register, .. } => Some(1 << register.mask_index()),
            other => {
                self.raise(
                    index,
                    other.span(),
                    DiagnosticKind::RegisterListExpected {
                        found: other.description().to_string(),
                    },
                );
                None
            }
        }
    }

    /// `fail [message]`: the program's own error (`Directives/fail.htm`).
    ///
    /// The message is the text after the Directive, verbatim — the parser keeps
    /// it raw, commas and spaces included — or EASy68K's own default when the
    /// line writes none. The assembly carries on, as the help says it does, and
    /// the error is what stops the Program from being built.
    fn plan_fail(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "fail");
        // A Label on a `fail` names the address the line sits at, like a Label
        // on any Directive that produces nothing.
        self.define_label(index);
        let operation = self.line(index).operation.as_ref();
        let (message, span, written) = match operation.and_then(|operation| operation.text.as_ref())
        {
            Some(field) => (field.text.clone(), field.span, true),
            None => (
                UNSPECIFIED_FAILURE.to_string(),
                operation
                    .map(|operation| operation.name_span)
                    .unwrap_or(Span::empty(0)),
                false,
            ),
        };
        self.raise(
            index,
            span,
            DiagnosticKind::UserDefinedError { message, written },
        );
        self.nothing()
    }

    /// `simhalt`: four bytes that end the run.
    ///
    /// EASy68K assembles it to the object code `$FFFFFFFF`, which its simulator
    /// reads as "halt" (`Directives/simhalt.htm`). Here it is an executable
    /// item of the Program like an instruction — the same size, the same
    /// alignment, and a Label on it names its address — and pass 2 lowers it to
    /// [`Instruction::SIMHALT`].
    ///
    /// Whatever follows it on the line is a **comment** and is not looked at,
    /// which is the help's own usage line, `LABEL SIMHALT comment`, and what
    /// `page`, `list` and `nolist` already do here. Without that rule
    /// `SIMHALT                 Halt Simulator` — line 206 of
    /// `tests/corpus/easy68k/graphicSound.X68` — would be an Operand `Halt` and
    /// a Comment `Simulator`, because the Operand field ends at the first
    /// whitespace that is not beside a comma (`docs/grammar.md` 1.5) and no
    /// rule of the parser can know that this Operation takes none.
    fn plan_simhalt(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "simhalt");
        self.align(2);
        self.define_label(index);
        let address = self.address();
        self.place(index, INSTRUCTION_SIZE);
        self.plan(address, Item::Instruction)
    }

    /// `offset expr`: a temporary origin that produces no bytes
    /// (`Directives/offset.htm`).
    ///
    /// From here to the `org` that ends the region every name in a Label field
    /// takes an offset rather than an address and nothing is placed at all: the
    /// region's counter shadows the section's, so closing it leaves the section
    /// exactly where it was, which is what `org *` restores. A Label on the
    /// `offset` line itself names the offset the region starts at, the rule a
    /// Label on an `org` follows.
    ///
    /// The Expression decides the Layout, so a forward reference is refused in
    /// it. The value is **not** held to the 16 MB of memory, because it is not
    /// an address: the help's own stack frame counts from `-3*4`.
    fn plan_offset(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "offset");
        // The region opens whatever the Operand turned out to be, for the
        // reason `equ` defines its name whatever its value is: a region that
        // did not open would lay every line of the table into memory and answer
        // one mistake, already reported, with another at every line below it.
        let value = match self.one_operand(index, "offset") {
            Some(operand) => self.value_now(index, "offset", &operand).unwrap_or(0),
            None => 0,
        };
        self.offset = Some(value);
        self.define_label(index);
        self.nothing()
    }

    /// `section [n]`: one of the sixteen location counters
    /// (`Directives/section.htm`).
    ///
    /// The counter is "restored to the address following the last location
    /// allocated in the indicated section (or to zero if used for the first
    /// time)", which is what the sixteen counters hold; an `org` inside a
    /// section writes the one in force. A number outside `0..15` is
    /// [`ValueOutOfRange`](DiagnosticKind::ValueOutOfRange), the kind the
    /// address of an `org` and the count of a `ds` already answer with, named
    /// by its subject. The number may be a Symbol — the help writes
    /// `SECTION DATA` against `DATA EQU 1` — so it is an ordinary Expression,
    /// and it decides the Layout, so a forward reference is refused in it.
    ///
    /// With no number the Directive requires a Label and gives it the number of
    /// the section in force. That rule cannot be read off the Directive's name,
    /// which is why [`label_rule_of`] answers `Optional` for `section` and this
    /// raises [`DirectiveNeedsALabel`](DiagnosticKind::DirectiveNeedsALabel)
    /// itself.
    fn plan_section(&mut self, index: usize) -> LinePlan {
        self.no_size(index, "section");
        let operands = self.operands(index);
        let operand = match operands.len() {
            0 => return self.name_the_current_section(index),
            1 => operands.into_iter().next().expect("one operand"),
            found => {
                self.wrong_operand_count(index, "section", found, vec![0, 1]);
                self.define_label(index);
                return self.nothing();
            }
        };
        let number = self.value_now(index, "section", &operand);
        if let Some(number) = number.filter(|number| self.is_a_section(index, &operand, *number)) {
            // A `section` sets the current address, so it ends an `offset`
            // region as `org` does. The help says nothing about the two
            // together and silence is answered the lenient way (ADR 0001):
            // refusing the line would refuse a program EASy68K assembles.
            self.close_the_offset_region();
            self.section = number as usize;
        }
        // The Label names where the section goes on from, which is the rule a
        // Label on an `org` follows.
        self.define_label(index);
        self.nothing()
    }

    /// Whether a `section` number is one of the sixteen, saying so when it is
    /// not.
    fn is_a_section(&mut self, index: usize, operand: &Operand, number: i64) -> bool {
        let last = SECTION_COUNT as i64 - 1;
        if (0..=last).contains(&number) {
            return true;
        }
        self.raise(
            index,
            operand.span(),
            DiagnosticKind::ValueOutOfRange {
                subject: "the number of `section`".to_string(),
                value: number,
                min: 0,
                max: last,
                advice: Some(
                    "a program has sixteen sections, `section 0` to `section 15`, and none of \
                     them is reserved"
                        .to_string(),
                ),
            },
        );
        false
    }

    /// `label section`: the number of the section in force.
    ///
    /// "If no section number is specified then a label is required and will be
    /// set to the value of the current section (0..15)"
    /// (`Directives/section.htm`). The value is a number and not an address, so
    /// the Symbol is a **Constant**, which is what the help's own Macro reads
    /// back with `SECTION SECT\@`.
    fn name_the_current_section(&mut self, index: usize) -> LinePlan {
        let Some(label) = self.line(index).label.as_ref() else {
            let span = self.operation_name_span(index);
            self.raise(
                index,
                span,
                DiagnosticKind::DirectiveNeedsALabel {
                    directive: "section".to_string(),
                },
            );
            return self.nothing();
        };
        let (name, span) = (label.name.clone(), label.span);
        let number = self.section as i64;
        self.define(
            index,
            &name,
            span,
            SymbolKind::Constant,
            SymbolValue::Number(number),
        );
        self.nothing()
    }

    /// What a line that would put something in the program makes inside an
    /// `offset` region: nothing, and a Diagnostic when bytes were meant.
    ///
    /// "No machine code is generated by instructions or directives following an
    /// OFFSET directive" (`Directives/offset.htm`). `ds` is what an offset
    /// table is made of and says nothing: its counter has already moved and
    /// there is no memory here to reserve. An instruction, a `simhalt` and a
    /// `dc` are another matter — they would have produced bytes that reach no
    /// memory, and a student who wrote code inside a region is told so rather
    /// than handed a Program silently short of it.
    fn hold_back_in_an_offset_region(&mut self, index: usize, plan: LinePlan) -> LinePlan {
        if self.offset.is_none() {
            return plan;
        }
        match plan.item {
            Item::Nothing => return plan,
            // `ds` is the Directive an offset table is made of: its counter has
            // moved and there is no memory here to reserve.
            Item::Reserved(_) => {}
            Item::Instruction | Item::Data(_) => {
                let item = self.offset_item_name(index);
                let span = self.operation_name_span(index);
                self.raise(
                    index,
                    span,
                    DiagnosticKind::NoBytesInAnOffsetRegion { item },
                );
            }
        }
        LinePlan {
            item: Item::Nothing,
            ..plan
        }
    }

    /// How a Diagnostic names what a line inside an `offset` region would have
    /// put there: the Directive as it is written, or "an instruction".
    fn offset_item_name(&self, index: usize) -> String {
        let Some(operation) = self.line(index).operation.as_ref() else {
            return "this line".to_string();
        };
        let name = operation.lowercase_name();
        if table::lookup(&name).is_some() {
            return "an instruction".to_string();
        }
        match operation.size {
            Some(size) => format!("`{name}.{}`", size.letter()),
            None => format!("`{name}`"),
        }
    }

    /// The §2.6 label rule of a Directive, checked before the Directive itself.
    ///
    /// Two of EASy68K's errors, and [`label_rule_of`] is the list: "Label
    /// required with this directive" for the Directives that give a name to
    /// something, and "Label is not allowed" for `page` and the
    /// conditional-assembly Directives (`errors.htm`, `Directives/page.htm`,
    /// `Directives/conditional.htm`).
    ///
    /// A Label that is not allowed is **still defined** at the current address.
    /// The line is already an error, so nothing is built either way, and
    /// leaving the name undefined would report it again at every use of it —
    /// the rule `plan_equate` follows for a value it cannot work out.
    fn check_label_rule(&mut self, index: usize, directive: &str) {
        let line = self.line(index);
        match (label_rule_of(directive), line.label.as_ref()) {
            (LabelRule::Required, None) => {
                let span = self.operation_name_span(index);
                self.raise(
                    index,
                    span,
                    DiagnosticKind::DirectiveNeedsALabel {
                        directive: directive.to_string(),
                    },
                );
            }
            (LabelRule::Forbidden, Some(label)) => {
                let (name, span) = (label.name.clone(), label.span);
                self.raise(
                    index,
                    span,
                    DiagnosticKind::LabelNotAllowed {
                        directive: directive.to_string(),
                        name,
                    },
                );
            }
            _ => {}
        }
    }

    /// A Directive s68k reads and does not implement in this phase.
    fn unimplemented_directive(&mut self, index: usize, name: &str) {
        let (reason, alternative) = unimplemented_reason(name);
        let span = self.operation_name_span(index);
        self.raise(
            index,
            span,
            DiagnosticKind::UnimplementedOperation {
                name: name.to_string(),
                reason: reason.to_string(),
                alternative: alternative.map(str::to_string),
            },
        );
    }

    // -- pass 2 ------------------------------------------------------------

    /// Assemble one line, now that every Symbol has a value.
    fn assemble_line(&mut self, index: usize) {
        let plan = self.plans[index].clone();
        let line = self.line(index);
        // The analyzer judges every Operation, whatever pass 1 made of it: an
        // unknown Mnemonic, an instruction that is not implemented and a
        // Directive it has nothing to say about all reach it the same way.
        let instruction = self.analyze(index, &plan);
        match plan.item {
            Item::Instruction => {
                if let Some(instruction) = instruction {
                    self.instructions.push(AssembledInstruction {
                        address: plan.address as usize,
                        size: INSTRUCTION_SIZE,
                        instruction,
                        location: Location::whole_line(self.source.path(), index, self.text(index)),
                        source: self.text(index).to_string(),
                    });
                }
            }
            Item::Data(length) => {
                let name = line
                    .operation
                    .as_ref()
                    .map(Operation::lowercase_name)
                    .unwrap_or_default();
                let bytes = match name.as_str() {
                    "dcb" => self.block_bytes(index, &plan, length),
                    _ => self.constant_bytes(index, &plan),
                };
                self.memory.push(MemoryRun {
                    address: plan.address as usize,
                    content: MemoryContent::Bytes { bytes },
                    location: Location::whole_line(self.source.path(), index, self.text(index)),
                });
            }
            Item::Reserved(length) => self.memory.push(MemoryRun {
                address: plan.address as usize,
                content: MemoryContent::Reserved { length },
                location: Location::whole_line(self.source.path(), index, self.text(index)),
            }),
            Item::Nothing => {}
        }
    }

    /// Run the analyzer over the line's Operation, after evaluating the
    /// Expressions of its Operands so that an undefined name is one message and
    /// not a silently skipped check.
    fn analyze(&mut self, index: usize, plan: &LinePlan) -> Option<Instruction> {
        let line = self.line(index);
        let operation = line.operation.as_ref()?;
        let name = operation.lowercase_name();
        // A Directive is pass 1's; a word that is neither a Mnemonic nor a
        // Directive still goes to the analyzer, which is where
        // `unknown_mnemonic` is raised.
        let spec = table::lookup(&name);
        if spec.is_none() && names::is_directive(&name) {
            // `simhalt` is the one Directive that puts an executable item in
            // the Program, and pass 1 has already given it its four bytes.
            return match name.as_str() {
                "simhalt" if plan.item == Item::Instruction => Some(Instruction::SIMHALT),
                _ => None,
            };
        }
        let mut parser_reported_an_error = self
            .parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.location.line == index && diagnostic.is_error());
        // A `reg` Symbol standing where `movem` wants a register list is read
        // before anything else looks at the Operand: it is not an Expression
        // and must not be evaluated as one.
        let resolved = match (spec, parser_reported_an_error) {
            (Some(spec), false) => {
                self.resolve_register_lists(index, plan, spec, &operation.operands)
            }
            _ => None,
        };
        // A name this could not read has been answered by name; judging it
        // against the instruction table as well would say it is an absolute
        // address, which is not the mistake.
        if matches!(resolved, Some(RegisterLists::Refused)) {
            parser_reported_an_error = true;
        }
        let rewritten: Option<Line> = match &resolved {
            Some(RegisterLists::Resolved(operands)) => {
                Some(rewrite_operands(line, operands.clone()))
            }
            _ => None,
        };
        let line: &Line = rewritten.as_ref().unwrap_or(line);
        if let (Some(spec), false) = (spec, parser_reported_an_error) {
            // The Operands of a line the parser failed on are whatever recovery
            // left behind, and a name out of one of them is not the mistake.
            for (position, operand) in operation.operands.iter().enumerate() {
                if spec.takes_a_register_list(position) && self.names_a_register_list(plan, operand)
                {
                    continue;
                }
                for expression in expressions_of(operand) {
                    self.value_later(index, plan, expression);
                }
            }
        }
        let (instruction, found) = {
            let symbols = self.symbols.in_scope(plan.scope.as_deref(), index, None);
            let context = Context {
                symbols: &symbols,
                current_address: plan.address,
                origin: self.origin.unwrap_or(DEFAULT_ORIGIN),
                macros: &self.parsed.macros,
            };
            let mut analyzer = Analyzer::new(self.source.path(), index, self.text(index), &context);
            let instruction = analyzer.analyze_line(line, parser_reported_an_error);
            (instruction, analyzer.finish())
        };
        self.diagnostics.extend(found);
        instruction
    }

    /// Read a `reg` Symbol standing where `movem` wants a register list.
    ///
    /// `Directives/reg.htm`'s whole example is `AllRegs REG D0-D7/A0-A6` and
    /// then `MOVEM.L AllRegs,-(SP)`. The parser cannot know what `AllRegs` is —
    /// a bare name is always an `absolute` (`docs/grammar.md` 1.12) — so the
    /// Symbol is read here, in whichever of `movem`'s two positions the
    /// direction puts the list, and the Operand becomes the list it stands for.
    /// From there it is lowered exactly as a written list is, the predecrement
    /// mask reversal included, because it *is* one.
    ///
    /// Three names are refused instead, each by its own sentence:
    ///
    /// * a Symbol of another kind — EASy68K's "Symbol is not a register list
    ///   symbol";
    /// * a `reg` Symbol defined further down — its "Register list symbol not
    ///   previously defined". This is the one forward reference that is refused
    ///   in an instruction Operand, and the reason is that a register list is
    ///   not a value the Layout can put off: it decides how the instruction is
    ///   encoded.
    /// * a name that is defined nowhere is **not** refused here. The evaluator
    ///   answers it with `undefined_symbol` and its "did you mean", and the
    ///   analyzer adds what `movem` takes in that position, which between them
    ///   are the diagnosis of a missing `reg` line.
    fn resolve_register_lists(
        &mut self,
        index: usize,
        plan: &LinePlan,
        spec: &'static table::InstructionSpec,
        operands: &[Operand],
    ) -> Option<RegisterLists> {
        // A name is only *required* to be a register list where nothing else
        // fits: in `movem.l table,d0-d2` the list is the second Operand and
        // `table` is an ordinary address, and the first position accepts one in
        // the other direction's Form. So the two "this is not a register list"
        // sentences are held back unless the line has the right number of
        // Operands and no Form of it fits as written.
        let must_be_a_list =
            spec.arities().contains(&operands.len()) && !spec.has_a_form_that_fits(operands);
        let mut resolved: Option<Vec<Operand>> = None;
        let mut refused = false;
        for (position, operand) in operands.iter().enumerate() {
            if !spec.takes_a_register_list(position) {
                continue;
            }
            let Some((name, span)) = bare_symbol(operand) else {
                continue;
            };
            let Some(symbol) = self.symbols.resolve(name, plan.scope.as_deref()) else {
                continue;
            };
            let (kind, definition) = (symbol.kind, symbol.location.clone());
            let mask = match symbol.value {
                SymbolValue::RegisterList(mask) => mask,
                SymbolValue::Number(_) if must_be_a_list => {
                    self.raise(
                        index,
                        span,
                        DiagnosticKind::NotARegisterList {
                            name: name.to_string(),
                            kind: kind.description().to_string(),
                        },
                    );
                    refused = true;
                    continue;
                }
                // A name that stands for a value where a value also fits: it is
                // the address it names, and nothing here has anything to say.
                SymbolValue::Number(_) => continue,
            };
            // A name that *is* a register list can be nothing else, wherever
            // it stands: it has no value at all, so `must_be_a_list` does not
            // come into it.
            if definition.line > index {
                let location =
                    Location::from_span(self.source.path(), index, self.text(index), span);
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::RegisterListNotDefinedYet {
                            name: name.to_string(),
                        },
                        location,
                    )
                    .with_related(definition, "the register list is defined here"),
                );
                refused = true;
                continue;
            }
            resolved.get_or_insert_with(|| operands.to_vec())[position] =
                register_list_operand(mask, span);
        }
        match (refused, resolved) {
            (true, _) => Some(RegisterLists::Refused),
            (false, Some(operands)) => Some(RegisterLists::Resolved(operands)),
            (false, None) => None,
        }
    }

    /// Whether an Operand is a bare name that stands for a `reg` Symbol.
    ///
    /// The evaluator answers such a name with `register_list_in_expression`,
    /// which is right everywhere but in a `movem` register-list position: there
    /// the name *is* the Operand and is never read as an Expression.
    fn names_a_register_list(&self, plan: &LinePlan, operand: &Operand) -> bool {
        let Some((name, _)) = bare_symbol(operand) else {
            return false;
        };
        matches!(
            self.symbols
                .resolve(name, plan.scope.as_deref())
                .map(|symbol| symbol.value),
            Some(SymbolValue::RegisterList(_))
        )
    }

    /// The bytes of a `dc`: every item in turn, a quoted literal as its Latin-1
    /// bytes and everything else as a value of the Directive's size.
    fn constant_bytes(&mut self, index: usize, plan: &LinePlan) -> Vec<u8> {
        let size = self.stored_size(index);
        let unit = bytes_of(size);
        let operands = self.operands(index);
        let mut bytes = Vec::with_capacity(operands.len() * unit);
        for operand in &operands {
            match string_bytes(operand) {
                Some(text) => {
                    bytes.extend_from_slice(text);
                    // A string is padded up to a whole number of items, which
                    // is what 1.4.2 did and what keeps `dc.w 'abc'` two words.
                    while bytes.len() % unit != 0 {
                        bytes.push(0);
                    }
                }
                None => {
                    let value = self
                        .expression_of(index, &format!("dc.{}", size.letter()), operand)
                        .and_then(|expression| self.value_later(index, plan, &expression));
                    let value = match value {
                        Some(value) => {
                            self.check_data_range(index, operand.span(), "dc", size, value);
                            value
                        }
                        None => 0,
                    };
                    bytes.extend_from_slice(&value_bytes(value, size));
                }
            }
        }
        bytes
    }

    /// The bytes of a `dcb`: the fill value, `count` times.
    fn block_bytes(&mut self, index: usize, plan: &LinePlan, length: usize) -> Vec<u8> {
        let size = self.stored_size(index);
        let unit = bytes_of(size);
        let operands = self.operands(index);
        let Some(operand) = operands.get(1) else {
            return Vec::new();
        };
        let value = self
            .expression_of(index, &format!("dcb.{}", size.letter()), operand)
            .and_then(|expression| self.value_later(index, plan, &expression));
        let value = match value {
            Some(value) => {
                self.check_data_range(index, operand.span(), "dcb", size, value);
                value
            }
            None => 0,
        };
        let filling = value_bytes(value, size);
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length / unit {
            bytes.extend_from_slice(&filling);
        }
        bytes
    }

    /// The Entry point: `end`'s Operand, else a Label named `START`, else the
    /// first instruction.
    ///
    /// Both name look-ups fall back on an ASCII case-insensitive match over the
    /// Labels, which is the one place s68k reads a name case insensitively; the
    /// reason is in [`entry_by_case`](Layout::entry_by_case).
    fn entry_point(&mut self) -> usize {
        if let Some((expression, index)) = self.end.clone() {
            let plan = self.plans[index].clone();
            if let Some(address) = self.entry_by_case(index, &plan, &expression) {
                return address;
            }
            if let Some(value) = self.value_later(index, &plan, &expression) {
                return value.clamp(0, ADDRESS_SPACE - 1) as usize;
            }
        }
        if let Some(address) = self.label_named_start() {
            return address;
        }
        self.instructions
            .first()
            .map(|instruction| instruction.address)
            .unwrap_or(DEFAULT_ORIGIN as usize)
    }

    /// The Entry point of an `end` whose Operand is a Label written in another
    /// case: `END START` against a Label written `start`.
    ///
    /// EASy68K's own `mouseWindowSize.X68` declares `start` in column 1 and
    /// ends with `END    START`, which is the primary evidence that its symbol
    /// look-up is not case sensitive; the help never claims either way. [ADR
    /// 0001](../../../docs/adr/0001-easy68k-is-the-reference-dialect.md)
    /// promises that an EASy68K program assembles here unchanged, and s68k's
    /// Symbols are case sensitive (CONTEXT.md, "Symbol"), so the two are
    /// reconciled where they meet and nowhere else: the Entry point, and only
    /// when the exact name is defined nowhere. The case rule is unchanged for
    /// every other use of a name, and the warning says so.
    ///
    /// Two Labels differing only in case are not guessed between: nothing is
    /// answered and the ordinary `undefined_symbol` follows.
    fn entry_by_case(&mut self, index: usize, plan: &LinePlan, expression: &Expr) -> Option<usize> {
        let written = expression.as_symbol()?;
        if self
            .symbols
            .resolve(written, plan.scope.as_deref())
            .is_some()
        {
            return None;
        }
        let (found, address) = self.label_ignoring_case(written)?;
        self.raise(
            index,
            expression.span(),
            DiagnosticKind::EntryPointCase {
                written: written.to_string(),
                found,
            },
        );
        Some(address.clamp(0, ADDRESS_SPACE - 1) as usize)
    }

    /// The fallback Entry point: a Label named `START`, whatever its case.
    ///
    /// The name is s68k's own convention and not a word the program wrote
    /// (CONTEXT.md, "Entry point"), so there is nothing to warn about here and
    /// nothing for a student to correct: five of the thirty editor programs
    /// declare `start:` and write no `end` at all.
    fn label_named_start(&self) -> Option<usize> {
        let address = match self.symbols.get("START") {
            Some(symbol) if symbol.kind == SymbolKind::Label => match symbol.value {
                SymbolValue::Number(address) => Some(address),
                _ => None,
            },
            _ => None,
        };
        let address = match address {
            Some(address) => address,
            None => self.label_ignoring_case("START")?.1,
        };
        Some(address.clamp(0, ADDRESS_SPACE - 1) as usize)
    }

    /// The one Label whose name is `name` but for ASCII case, and its address.
    ///
    /// `None` when there is none, when the Label has no address, or when two
    /// Labels match, which is a program the assembler has no business guessing
    /// about.
    fn label_ignoring_case(&self, name: &str) -> Option<(String, i64)> {
        let mut found = None;
        for symbol in self.symbols.iter() {
            if symbol.kind != SymbolKind::Label || !symbol.name.eq_ignore_ascii_case(name) {
                continue;
            }
            let SymbolValue::Number(address) = symbol.value else {
                continue;
            };
            if found.is_some() {
                return None;
            }
            found = Some((symbol.name.clone(), address));
        }
        found
    }

    // -- the pieces both passes use ----------------------------------------

    /// The `index`th parsed line.
    fn line(&self, index: usize) -> &'a Line {
        &self.parsed.lines[index]
    }

    /// The text of the `index`th Source line.
    fn text(&self, index: usize) -> &'a str {
        self.source.line(index).unwrap_or("")
    }

    /// The Operands of the `index`th line, cloned so that the line is not
    /// borrowed while a Diagnostic is raised about it.
    fn operands(&self, index: usize) -> Vec<Operand> {
        self.line(index)
            .operation
            .as_ref()
            .map(|operation| operation.operands.clone())
            .unwrap_or_default()
    }

    /// The size suffix the `index`th line's Operation carries.
    fn size_of(&self, index: usize) -> Option<SizeSuffix> {
        self.line(index)
            .operation
            .as_ref()
            .and_then(|operation| operation.size)
    }

    /// A plan for a line that puts nothing in the program.
    fn nothing(&mut self) -> LinePlan {
        let address = self.address();
        self.plan(address, Item::Nothing)
    }

    /// The address the next item goes to: the counter of an open `offset`
    /// region, else the counter of the section in force.
    fn address(&self) -> i64 {
        match self.offset {
            Some(counter) => counter,
            None => self.sections[self.section],
        }
    }

    /// Move the current address, whichever of the two counters that is.
    fn set_address(&mut self, address: i64) {
        match self.offset.as_mut() {
            Some(counter) => *counter = address,
            None => self.sections[self.section] = address,
        }
    }

    /// The address an `org` inside an `offset` region comes back to: the
    /// counter of the section in force, which the region has not touched.
    fn resume_address(&self) -> i64 {
        self.sections[self.section]
    }

    /// End an open `offset` region, which reveals the section's own counter
    /// exactly where the region found it.
    fn close_the_offset_region(&mut self) {
        self.offset = None;
    }

    /// The Span of the `index`th line's Operation name, or an empty one when
    /// the line has no Operation at all.
    fn operation_name_span(&self, index: usize) -> Span {
        self.line(index)
            .operation
            .as_ref()
            .map(|operation| operation.name_span)
            .unwrap_or(Span::empty(0))
    }

    /// A plan, with the scope the line ended in.
    fn plan(&mut self, address: i64, item: Item) -> LinePlan {
        LinePlan {
            address,
            scope: self.scope.clone(),
            item,
        }
    }

    /// Raise a Diagnostic about a span of the `index`th line.
    fn raise(&mut self, index: usize, span: Span, kind: DiagnosticKind) {
        let location = Location::from_span(self.source.path(), index, self.text(index), span);
        self.diagnostics.push(Diagnostic::new(kind, location));
    }

    /// Move the current address up to a multiple of `alignment`.
    ///
    /// The remainder is Euclidean, so that a negative address rounds *up* the
    /// way a positive one does and `-11` aligns to `-10`. Only an `offset`
    /// region can hold a negative current address, and the help's own stack
    /// frame is one.
    fn align(&mut self, alignment: i64) {
        let address = self.address();
        let remainder = address.rem_euclid(alignment);
        if alignment > 1 && remainder != 0 {
            // Saturating, because only an `offset` region's counter can be
            // anywhere near the end of the 64 bits an Expression is computed in
            // and a region that far out has been reported already.
            self.set_address(address.saturating_add(alignment - remainder));
        }
    }

    /// Take `length` bytes at the current address and move on.
    ///
    /// A run that would reach past the end of memory is refused and takes
    /// nothing, so that the addresses after it are still the ones the source
    /// asked for.
    fn place(&mut self, index: usize, length: usize) {
        if length == 0 {
            return;
        }
        if self.offset.is_some() {
            // Inside an `offset` region nothing is placed: the counter moves so
            // that the names below take their offsets, and no byte of the run
            // exists for the overlap sweep to find. The counter is not an
            // address either, so it is not held to the 16 MB — the region is a
            // table of offsets and the help's own starts at `-3*4`.
            self.set_address(self.address().saturating_add(length as i64));
            return;
        }
        let end = self.address() + length as i64;
        if end > ADDRESS_SPACE {
            let location = Location::whole_line(self.source.path(), index, self.text(index));
            self.diagnostics.push(Diagnostic::new(
                DiagnosticKind::ValueOutOfRange {
                    subject: "the last address of this line".to_string(),
                    value: end - 1,
                    min: 0,
                    max: ADDRESS_SPACE - 1,
                    advice: Some("s68k has 16 MB of memory".to_string()),
                },
                location,
            ));
            return;
        }
        if self.origin.is_none() {
            self.origin = Some(self.address());
        }
        self.placements.push(Placement {
            start: self.address(),
            end,
            line: index,
        });
        self.set_address(end);
    }

    /// Define the Label of the `index`th line at the current address, and open
    /// a new scope when it is a Global one.
    ///
    /// Inside an `offset` region the name is a **Constant** and not a Label:
    /// its value is an offset into a structure, no line of the program is laid
    /// out at it, and calling it a Label would put an address that does not
    /// exist in the symbol listing and in the corpus fixture
    /// (`tests/corpus/README.md`, "labels"). It is still the Label field, so a
    /// Global one still opens a scope for the Local names under it.
    fn define_label(&mut self, index: usize) {
        let Some(label) = self.line(index).label.as_ref() else {
            return;
        };
        let (name, span) = (label.name.clone(), label.span);
        if !symbols::is_local(&name) {
            self.scope = Some(name.clone());
        }
        let kind = match self.offset {
            Some(_) => SymbolKind::Constant,
            None => SymbolKind::Label,
        };
        let address = self.address();
        self.define(index, &name, span, kind, SymbolValue::Number(address));
    }

    /// Define one Symbol, and say where it was already defined when it was.
    fn define(
        &mut self,
        index: usize,
        name: &str,
        span: Span,
        kind: SymbolKind,
        value: SymbolValue,
    ) {
        // A Local label is defined inside the scope above it; a Global one has
        // just opened its own.
        let scope = match symbols::is_local(name) {
            true => self.scope.clone(),
            false => None,
        };
        let location = Location::from_span(self.source.path(), index, self.text(index), span);
        if let Err(diagnostic) =
            self.symbols
                .define(name, scope.as_deref(), kind, value, location, index)
        {
            self.diagnostics.push(*diagnostic);
        }
    }

    /// The one Operand of a Directive that takes exactly one.
    fn one_operand(&mut self, index: usize, directive: &str) -> Option<Operand> {
        let operands = self.operands(index);
        match operands.len() {
            1 => operands.into_iter().next(),
            found => {
                self.wrong_operand_count(index, directive, found, vec![1]);
                None
            }
        }
    }

    /// The Expression an Operand of a Directive holds, or the `value_expected`
    /// error when the Operand is an Addressing mode instead.
    fn expression_of(&mut self, index: usize, directive: &str, operand: &Operand) -> Option<Expr> {
        match operand {
            Operand::Absolute { value, .. } => Some(value.clone()),
            other => {
                let advice = self.rewrite_of_an_equate(index, directive, other);
                self.raise(
                    index,
                    other.span(),
                    DiagnosticKind::ValueExpected {
                        directive: directive.to_string(),
                        found: other.description().to_string(),
                        advice,
                    },
                );
                None
            }
        }
    }

    /// How to rewrite the two `equ` forms 1.4.2 accepted and this does not.
    ///
    /// [ADR
    /// 0001](../../../docs/adr/0001-easy68k-is-the-reference-dialect.md) drops
    /// the text aliasing of `ten equ #10` and `reg equ d1` and promises that
    /// "the error for the old forms says how to rewrite them", so the two
    /// shapes it names by hand get the sentence that rewrites them; every other
    /// Operand keeps the generic hint. `equ` and `set` both define a value and
    /// both reach this.
    fn rewrite_of_an_equate(
        &self,
        index: usize,
        directive: &str,
        operand: &Operand,
    ) -> Option<String> {
        if directive != "equ" && directive != "set" {
            return None;
        }
        // Only the value Directives reach this with a Label: `plan_equate`
        // stops at `directive_needs_a_label` when there is none.
        let name = self.line(index).label.as_ref()?.name.clone();
        match operand {
            Operand::Immediate { value, .. } => {
                let written = value.span().text(self.text(index));
                Some(format!(
                    "write `{name} {directive} {written}`, and keep the `#` where the value is \
                     used, as in `move.l #{name},d0`"
                ))
            }
            Operand::DataRegisterDirect { .. }
            | Operand::AddressRegisterDirect { .. }
            | Operand::SpecialRegister { .. }
            | Operand::RegisterList { .. } => Some(format!(
                "`{directive}` names a value, not a register: write the register itself where \
                 `{name}` is used, or a `reg` list once `reg` is implemented"
            )),
            _ => None,
        }
    }

    /// The value of a Directive's Operand in **pass 1**, where a forward
    /// reference is refused.
    fn value_now(&mut self, index: usize, directive: &str, operand: &Operand) -> Option<i64> {
        self.value_now_at(index, directive, operand, self.address())
    }

    /// The same, with a value of its own for `*`, which only an `org` that ends
    /// an `offset` region needs (`Directives/offset.htm`).
    fn value_now_at(
        &mut self,
        index: usize,
        directive: &str,
        operand: &Operand,
        star: i64,
    ) -> Option<i64> {
        let expression = self.expression_of(index, directive, operand)?;
        let mut problems = Vec::new();
        let value = {
            let declared = |name: &str| self.declared.contains(name);
            let symbols = self
                .symbols
                .in_scope(self.scope.as_deref(), index, Some(&declared));
            expr::evaluate(&expression, &symbols, star, &mut problems)
        };
        let site = Site {
            file: self.source.path(),
            line_index: index,
            line_text: self.text(index),
            refused_by: Some(directive),
        };
        self.diagnostics.extend(expr::diagnose(&problems, &site));
        value
    }

    /// A count in pass 1 — `ds` and `dcb` — which is a value that also has to
    /// fit in memory.
    fn count_now(&mut self, index: usize, directive: &str, operand: &Operand) -> Option<usize> {
        let value = self.value_now(index, directive, operand)?;
        if !(0..=ADDRESS_SPACE).contains(&value) {
            self.raise(
                index,
                operand.span(),
                DiagnosticKind::ValueOutOfRange {
                    subject: format!("the count of `{directive}`"),
                    value,
                    min: 0,
                    max: ADDRESS_SPACE,
                    advice: Some("s68k has 16 MB of memory".to_string()),
                },
            );
            return None;
        }
        Some(value as usize)
    }

    /// The value of an Expression in **pass 2**, where every Symbol is defined
    /// and a forward reference is no longer one.
    fn value_later(&mut self, index: usize, plan: &LinePlan, expression: &Expr) -> Option<i64> {
        let mut problems = Vec::new();
        let value = {
            let symbols = self.symbols.in_scope(plan.scope.as_deref(), index, None);
            expr::evaluate(expression, &symbols, plan.address, &mut problems)
        };
        let site = Site {
            file: self.source.path(),
            line_index: index,
            line_text: self.text(index),
            refused_by: None,
        };
        self.diagnostics.extend(expr::diagnose(&problems, &site));
        value
    }

    /// The size of a data Directive: what it wrote, or `.w`
    /// (`Directives/ds.htm`, `Directives/dcb.htm`; the help is silent for `dc`
    /// and every corpus program writes the size).
    fn data_size(&mut self, index: usize, directive: &str) -> SizeSuffix {
        // `.s` is a branch displacement and no width at all.
        if self.size_of(index) == Some(SizeSuffix::Short) {
            self.invalid_size(index, directive, &[".b", ".w", ".l"]);
        }
        self.stored_size(index)
    }

    /// The same size, without a word about it: what pass 2 stores the values at
    /// once pass 1 has said whatever there was to say.
    fn stored_size(&self, index: usize) -> SizeSuffix {
        match self.size_of(index) {
            Some(SizeSuffix::Short) | None => SizeSuffix::Word,
            Some(size) => size,
        }
    }

    /// A Directive that carries no size at all.
    fn no_size(&mut self, index: usize, directive: &str) {
        if self.size_of(index).is_some() {
            self.invalid_size(index, directive, &[]);
        }
    }

    fn invalid_size(&mut self, index: usize, directive: &str, allowed: &[&str]) {
        let Some(operation) = self.line(index).operation.as_ref() else {
            return;
        };
        let (size, span) = match (operation.size, operation.size_span) {
            (Some(size), Some(span)) => (size.suffix().to_string(), span),
            _ => return,
        };
        self.raise(
            index,
            span,
            DiagnosticKind::InvalidSize {
                mnemonic: directive.to_string(),
                size,
                allowed: allowed.iter().map(|size| size.to_string()).collect(),
            },
        );
    }

    fn wrong_operand_count(
        &mut self,
        index: usize,
        directive: &str,
        found: usize,
        expected: Vec<usize>,
    ) {
        self.raise_operand_count(index, directive, found, expected, false);
    }

    /// A Directive that takes a *list* and was given nothing: `dc.b` on its
    /// own.
    ///
    /// Its own sentence, because "`dc` takes one operand" is not true of a
    /// Directive whose whole point is that `dc.b 1,2,3` assembles.
    fn wrong_item_count(&mut self, index: usize, directive: &str) {
        self.raise_operand_count(index, directive, 0, vec![1], true);
    }

    fn raise_operand_count(
        &mut self,
        index: usize,
        directive: &str,
        found: usize,
        expected: Vec<usize>,
        at_least: bool,
    ) {
        let span = self
            .line(index)
            .operation
            .as_ref()
            .map(|operation| operation.span)
            .unwrap_or(Span::empty(0));
        self.raise(
            index,
            span,
            DiagnosticKind::WrongOperandCount {
                mnemonic: directive.to_string(),
                found,
                expected,
                at_least,
            },
        );
    }

    /// A `dc` or `dcb` value against the size it is stored at, the same range
    /// an immediate of that size holds: signed at the bottom, unsigned at the
    /// top, so that `-1` and `$ff` are both a byte.
    fn check_data_range(
        &mut self,
        index: usize,
        span: Span,
        directive: &str,
        size: SizeSuffix,
        value: i64,
    ) {
        let (min, max) = match size {
            SizeSuffix::Byte => (-128, 255),
            SizeSuffix::Word => (-32768, 65535),
            _ => (-2_147_483_648, 4_294_967_295),
        };
        if !(min..=max).contains(&value) {
            self.raise(
                index,
                span,
                DiagnosticKind::ValueOutOfRange {
                    subject: format!("an item of `{directive}.{}`", size.letter()),
                    value,
                    min,
                    max,
                    advice: match size {
                        SizeSuffix::Byte => Some("`.w` holds it".to_string()),
                        SizeSuffix::Word => Some("`.l` holds it".to_string()),
                        _ => None,
                    },
                },
            );
        }
    }

    /// Every pair of lines laid out over the same address, reported at the
    /// second of the two.
    fn report_overlaps(&mut self) {
        let mut placements: Vec<(i64, i64, usize)> = self
            .placements
            .iter()
            .map(|placement| (placement.start, placement.end, placement.line))
            .collect();
        placements.sort_by_key(|(start, end, line)| (*start, *end, *line));
        let mut reached: Option<(i64, usize)> = None;
        for (start, end, line) in placements {
            match reached {
                Some((furthest, earlier)) if start < furthest => {
                    // The later of the two lines is the one that is wrong, and
                    // the earlier one is where the address already went.
                    let (first, second) = match earlier < line {
                        true => (earlier, line),
                        false => (line, earlier),
                    };
                    let location =
                        Location::whole_line(self.source.path(), second, self.text(second));
                    let related = Location::whole_line(self.source.path(), first, self.text(first));
                    self.diagnostics.push(
                        Diagnostic::new(
                            DiagnosticKind::AddressUsedTwice { address: start },
                            location,
                        )
                        .with_related(related, "this line is laid out here"),
                    );
                    reached = match end > furthest {
                        true => Some((end, line)),
                        false => Some((furthest, earlier)),
                    };
                }
                _ => reached = Some((end, line)),
            }
        }
    }
}

/// Every full name the File defines, whatever pass 1 makes of it.
///
/// It is collected before pass 1 so that a name used above its definition can
/// be told from a name that is defined nowhere: the first is
/// `forward_reference_not_allowed` and the second is `undefined_symbol`, and
/// only this set tells them apart.
fn declared_names(parsed: &ParsedFile) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut scope: Option<String> = None;
    for line in &parsed.lines {
        let Some(label) = line.label.as_ref() else {
            continue;
        };
        if symbols::is_local(&label.name) {
            names.insert(symbols::qualify(&label.name, scope.as_deref()));
        } else {
            scope = Some(label.name.clone());
            names.insert(label.name.clone());
        }
    }
    names
}

/// The sixteen location counters as a program starts.
///
/// Section 0 is at the default origin and the other fifteen at zero. EASy68K's
/// own rule is "zero if used for the first time" for every section
/// (`Directives/section.htm`) and s68k's own is a default origin of `$1000`
/// (ADR 0001); the two meet at section 0, which is the section a program starts
/// in — "by default, the assembler will begin with section 0" — and therefore
/// the one the default origin is a statement about. The fifteen others are
/// EASy68K's zero, so a program that writes `section 1` and no `org` lays its
/// data out from 0 exactly as EASy68K does.
fn sections_at_the_start() -> [i64; SECTION_COUNT] {
    let mut sections = [0; SECTION_COUNT];
    sections[0] = DEFAULT_ORIGIN;
    sections
}

/// How many bytes one item of a size takes up.
fn bytes_of(size: SizeSuffix) -> usize {
    match size {
        SizeSuffix::Byte => 1,
        SizeSuffix::Word | SizeSuffix::Short => 2,
        SizeSuffix::Long => 4,
    }
}

/// What a data Directive of that size is aligned to: a word and a long start on
/// an even address, a byte anywhere (`Directives/dc.htm`).
fn alignment_of(size: SizeSuffix) -> i64 {
    match size {
        SizeSuffix::Byte => 1,
        _ => 2,
    }
}

/// The Latin-1 bytes of a `dc` item that is one quoted literal, or `None` when
/// the item is a value.
///
/// A literal that is part of an Expression (`'A'+1`) is a value, not a string:
/// only a bare one is laid out as its bytes (`docs/grammar.md` 1.9).
fn string_bytes(operand: &Operand) -> Option<&[u8]> {
    match operand {
        Operand::Absolute {
            value: Expr::CharacterLiteral { bytes, .. },
            ..
        } => Some(bytes),
        _ => None,
    }
}

/// How many bytes one `dc` item takes up.
fn item_length(operand: &Operand, size: SizeSuffix) -> usize {
    let unit = bytes_of(size);
    match string_bytes(operand) {
        Some(bytes) => bytes.len().div_ceil(unit) * unit,
        None => unit,
    }
}

/// A value as the bytes of its size, highest byte first.
fn value_bytes(value: i64, size: SizeSuffix) -> Vec<u8> {
    match size {
        SizeSuffix::Byte => vec![value as u8],
        SizeSuffix::Word | SizeSuffix::Short => (value as u16).to_be_bytes().to_vec(),
        SizeSuffix::Long => (value as u32).to_be_bytes().to_vec(),
    }
}

/// Every Expression one Operand holds, which is what pass 2 evaluates so that
/// an undefined name is reported once and by name.
fn expressions_of(operand: &Operand) -> Vec<&Expr> {
    match operand {
        Operand::Immediate { value, .. } | Operand::Absolute { value, .. } => vec![value],
        Operand::Displacement { displacement, .. }
        | Operand::PcDisplacement { displacement, .. } => vec![displacement],
        Operand::Index { displacement, .. } | Operand::PcIndex { displacement, .. } => {
            displacement.iter().collect()
        }
        Operand::DataRegisterDirect { .. }
        | Operand::AddressRegisterDirect { .. }
        | Operand::SpecialRegister { .. }
        | Operand::Indirect { .. }
        | Operand::Postincrement { .. }
        | Operand::Predecrement { .. }
        | Operand::RegisterList { .. } => Vec::new(),
    }
}

/// What a Directive's label field has to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelRule {
    /// The Directive gives a name to something and cannot be written without
    /// one: EASy68K's "Label required with this directive".
    Required,
    /// The Directive takes no label at all: EASy68K's "Label is not allowed".
    Forbidden,
    /// A Label is allowed and names the address the line sits at, which is
    /// every other Directive.
    Optional,
}

/// The label rule of one Directive (`docs/grammar.md` 2.6).
///
/// **Required** is the Directives that give a name to something: `equ`, `set`
/// and `reg`, whose usage lines are `label EQU value`, `Size SET 38` and
/// `AllRegs REG D0-D7/A0-A6`. `section` with no number joins them ("If no
/// section number is specified then a label is required",
/// `Directives/section.htm`), and it is the one rule that cannot be read off
/// the name of the Directive — it depends on the Operand — so `section` is
/// `Optional` here and [`Layout::plan_section`] raises
/// [`DirectiveNeedsALabel`](DiagnosticKind::DirectiveNeedsALabel) itself.
///
/// **Forbidden** is EASy68K's own list and nothing beyond it: `page` ("No label
/// is permitted", `Directives/page.htm`) and the conditional-assembly
/// Directives ("IFxx and ENDC directives may not be labeled",
/// `Directives/conditional.htm`). The `macro` Directive takes the Macro's name
/// in its label field and is not in the list; the structured-control keywords
/// are not either, because the help says nothing about them and silence is
/// answered by the lenient reading (ADR 0001).
///
/// A Directive s68k refuses whole is still checked, so `skip ifeq debug` is
/// answered both about its label and about conditional assembly: the two are
/// separate mistakes at separate places in the line, and the label rule is true
/// of the shape of the line whether or not the feature is implemented.
fn label_rule_of(directive: &str) -> LabelRule {
    match directive {
        "equ" | "set" | "reg" => LabelRule::Required,
        "page" | "ifeq" | "ifne" | "iflt" | "ifle" | "ifgt" | "ifge" | "ifc" | "ifnc" | "ifarg"
        | "endc" => LabelRule::Forbidden,
        _ => LabelRule::Optional,
    }
}

/// The name an Operand that is one bare Symbol carries, with its Span.
///
/// This is the shape a `reg` Symbol reaches `movem` in: a bare name always
/// parses as an `absolute` whose Expression is one `symbol_reference`
/// (`docs/grammar.md` 1.12), and only the Symbol table can say what it means.
fn bare_symbol(operand: &Operand) -> Option<(&str, Span)> {
    match operand {
        Operand::Absolute {
            value: Expr::Symbol { name, span },
            ..
        } => Some((name.as_str(), *span)),
        _ => None,
    }
}

/// The same Line with different Operands, which is what a resolved register
/// list gives the analyzer to judge.
fn rewrite_operands(line: &Line, operands: Vec<Operand>) -> Line {
    let mut rewritten = line.clone();
    if let Some(operation) = rewritten.operation.as_mut() {
        operation.operands = operands;
    }
    rewritten
}

/// The Operand a `movem` register-list mask stands for, at the span of the name
/// that named it.
///
/// One item a register, which is all the lowering reads: the mask it computes
/// back is the one this was built from, and the fixture printer prints the mask
/// and never the items. The span is the name's, so a Diagnostic about the
/// Operand lands on the word the source wrote.
fn register_list_operand(mask: u16, span: Span) -> Operand {
    let items = (0u8..16)
        .filter(|index| mask & (1 << index) != 0)
        .map(|index| RegisterListItem::Single {
            register: Register {
                kind: match index < 8 {
                    true => RegisterKind::Data,
                    false => RegisterKind::Address,
                },
                number: index & 7,
                span,
            },
            span,
        })
        .collect();
    Operand::RegisterList { items, span }
}

/// Why a Directive is not implemented, and what to write instead.
///
/// The design record's "Directives" says which bucket each one is in: `memory`,
/// the Macro and conditional-assembly Directives and the structured-control
/// keywords are refused, and `include` and `incbin` arrive in phase 4, which is
/// what "yet" says. Phase 2 has taken every other Directive out of this list. The Operation names that reach this are the ones
/// [`names::is_directive`] knows and [`Layout::plan_directive`] does not.
fn unimplemented_reason(name: &str) -> (&'static str, Option<&'static str>) {
    match name {
        "include" => (
            "assembling several files together is not implemented yet",
            Some("the file's lines here"),
        ),
        "incbin" => (
            "reading a file's bytes into memory is not implemented yet",
            Some("`dc.b` with the bytes written out"),
        ),
        "memory" => (
            "s68k has one memory of 16 MB and no access levels in it",
            None,
        ),
        // No "yet" in either of these two, and the reason is the design
        // record's own: macros with conditional assembly are "maybe a later
        // milestone" and no phase of this plan adds them, where `include` and
        // `incbin` above are phase 4's. "Yet" says a later phase brings it
        // (the implementation notes, phase 3), and a sentence that promises
        // more than the plan does is a sentence a student would be right to
        // believe.
        "macro" | "endm" | "mexit" => (
            "macros are not assembled",
            Some("the lines of the macro where it is used"),
        ),
        "ifeq" | "ifne" | "iflt" | "ifle" | "ifgt" | "ifge" | "ifc" | "ifnc" | "ifarg" | "endc" => {
            (
                "conditional assembly is not implemented",
                Some("the lines the condition would have kept"),
            )
        }
        _ => (
            "structured control is not assembled here",
            Some("a comparison and a branch, `cmp.w #10,d0` then `bge done`"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::instructions::encoded::Operand as EncodedOperand;
    use crate::assembler::instructions::encoded::TargetDirection;
    use crate::assembler::parser;
    use crate::assembler::source::Files;

    /// Lay a program out and give back its Program and its Diagnostics.
    fn assemble(source: &str) -> (Program, Vec<Diagnostic>) {
        let files = Files::from_source(source);
        let text = files.text("main.m68k").expect("the source");
        let file = SourceFile::new("main.m68k", text);
        let parsed = parser::parse_file("main.m68k", text);
        let (program, mut diagnostics) = lay_out(&file, &parsed);
        let mut all = parsed.diagnostics.clone();
        all.append(&mut diagnostics);
        all.sort_by_key(|diagnostic| (diagnostic.location.line, diagnostic.location.column));
        (program, all)
    }

    /// The codes a program raises, in source order.
    fn codes(source: &str) -> Vec<&'static str> {
        assemble(source)
            .1
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect()
    }

    /// The addresses of a program's instructions.
    fn addresses(source: &str) -> Vec<usize> {
        assemble(source)
            .0
            .instructions()
            .iter()
            .map(|instruction| instruction.address)
            .collect()
    }

    /// The initial memory of a program, as `(address, bytes or reserved)`.
    fn memory(source: &str) -> Vec<(usize, String)> {
        assemble(source)
            .0
            .memory()
            .iter()
            .map(|run| {
                (
                    run.address,
                    match &run.content {
                        MemoryContent::Bytes { bytes } => bytes
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>(),
                        MemoryContent::Reserved { length } => format!("{length} reserved"),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_program_starts_at_the_default_origin_and_steps_four_bytes() {
        assert_eq!(
            addresses("    nop\n    nop\n    rts\n"),
            vec![0x1000, 0x1004, 0x1008]
        );
    }

    #[test]
    fn org_moves_the_address_anywhere() {
        let source = "    org $2000\n    nop\n    org $1000\n    nop\n";
        // Sorted by address, which is not the source order here.
        assert_eq!(addresses(source), vec![0x1000, 0x2000]);
        assert!(codes(source).is_empty());
    }

    #[test]
    fn an_odd_origin_warns_and_rounds_up() {
        let source = "    org $2001\n    nop\n";
        assert_eq!(codes(source), vec!["odd_origin"]);
        assert_eq!(addresses(source), vec![0x2002]);
    }

    #[test]
    fn an_org_that_moves_nothing_says_nothing_about_an_odd_address() {
        // `org *` is the idiom that ends an `offset` region
        // (`Directives/offset.htm`), and the address it comes back to is odd
        // whenever a `dc.b` left it so. It is the address the program was
        // legally at and the `org` does not move it, so it is neither rounded
        // up nor complained about; an odd address the source *chose* still is.
        let source = "    org $2000\n    dc.b 1\n    org *\n    dc.b 2\n";
        assert!(codes(source).is_empty());
        assert_eq!(
            memory(source),
            vec![(0x2000, "01".to_string()), (0x2001, "02".to_string())]
        );
    }

    #[test]
    fn an_instruction_after_an_odd_byte_starts_on_an_even_address() {
        let source = "    org $2000\n    dc.b 1\n    nop\n";
        assert_eq!(addresses(source), vec![0x2002]);
    }

    #[test]
    fn a_word_after_an_odd_byte_is_padded_as_easy68k_pads_it() {
        // `Directives/dc.htm`: the assembler adjusts the memory locations so
        // that a word or a long starts on an even address. 1.4.2 did not.
        let source = "    org $2000\ndata: dc.b 'abc'\nnext: dc.w 1\nbyte: dc.b 2\nlong: dc.l 3\n";
        assert_eq!(
            memory(source),
            vec![
                (0x2000, "616263".to_string()),
                (0x2004, "0001".to_string()),
                (0x2006, "02".to_string()),
                (0x2008, "00000003".to_string()),
            ]
        );
        let program = assemble(source).0;
        assert_eq!(program.symbols()["next"].value, 0x2004);
        assert_eq!(program.symbols()["long"].value, 0x2008);
    }

    #[test]
    fn ds_reserves_its_room_and_writes_nothing() {
        let source = "    org $2000\nbuffer: ds.b 34\ntable:  ds.w 10\n";
        assert_eq!(
            memory(source),
            vec![
                (0x2000, "34 reserved".to_string()),
                (0x2022, "20 reserved".to_string()),
            ]
        );
        let program = assemble(source).0;
        assert_eq!(program.symbols()["table"].value, 0x2022);
    }

    #[test]
    fn ds_w_zero_is_the_alignment_idiom() {
        // `Directives/ds.htm`: "DS.W 0 may be used to force even word
        // alignment".
        let source = "    org $2001\n";
        assert_eq!(codes(source), vec!["odd_origin"]);
        let source = "    org $2000\n    dc.b 1\n    ds.w 0\nhere: dc.b 2\n";
        assert_eq!(assemble(source).0.symbols()["here"].value, 0x2002);
    }

    #[test]
    fn dcb_fills_its_block() {
        assert_eq!(
            memory("    org $2000\n    dcb.b 4,$ff\n    dcb.w 2,1\n"),
            vec![
                (0x2000, "ffffffff".to_string()),
                (0x2004, "00010001".to_string())
            ]
        );
    }

    #[test]
    fn dc_lays_strings_out_in_latin_1_and_pads_them_to_its_size() {
        assert_eq!(
            memory("    org $2000\n    dc.b 'Hello',0\n    dc.w 'abc'\n"),
            vec![
                (0x2000, "48656c6c6f00".to_string()),
                (0x2006, "61626300".to_string()),
            ]
        );
    }

    #[test]
    fn a_character_above_latin_1_in_data_is_an_error() {
        // ADR 0004: one byte in Latin-1 everywhere, and a character with no
        // byte is refused where it would have to produce one.
        assert_eq!(codes("    dc.b 'né'\n"), Vec::<&str>::new());
        assert_eq!(
            codes("    dc.b '\u{2014}'\n"),
            vec!["character_above_latin1"]
        );
    }

    #[test]
    fn equ_names_a_value_and_the_program_keeps_it() {
        let source = "count equ 12\n    move.l #count,d0\n";
        let program = assemble(source).0;
        assert_eq!(program.symbols()["count"].value, 12);
        assert_eq!(program.symbols()["count"].kind, SymbolKind::Constant);
        assert!(codes(source).is_empty());
    }

    #[test]
    fn equ_without_a_name_says_so() {
        assert_eq!(codes("    equ 12\n"), vec!["directive_needs_a_label"]);
    }

    #[test]
    fn a_name_defined_twice_is_an_error_at_both_places() {
        let (_, diagnostics) = assemble("count equ 1\ncount equ 2\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), "symbol_already_defined");
        assert_eq!(diagnostics[0].location.line, 1);
        assert_eq!(diagnostics[0].related[0].0.line, 0);
    }

    #[test]
    fn a_forward_reference_is_refused_where_the_value_decides_the_layout() {
        assert_eq!(
            codes("    org here\nhere: nop\n"),
            vec!["forward_reference_not_allowed"]
        );
        assert_eq!(
            codes("    ds.b size\nsize equ 4\n"),
            vec!["forward_reference_not_allowed"]
        );
        assert_eq!(
            codes("x equ y\ny equ 1\n"),
            vec!["forward_reference_not_allowed"]
        );
    }

    #[test]
    fn a_forward_reference_is_allowed_in_an_operand_and_in_data() {
        assert!(codes("    bra done\ndone: rts\n").is_empty());
        assert!(codes("    dc.l done\ndone: rts\n").is_empty());
        assert!(codes("    move.l #size,d0\nsize equ 4\n").is_empty());
    }

    #[test]
    fn a_constant_is_defined_even_when_its_value_cannot_be_worked_out() {
        // One mistake, one message: the `equ` is reported, and the two uses of
        // the name it defines are not reported as well.
        assert_eq!(
            codes("x equ nowhere\n    move.l #x,d0\n    move.l #x,d1\n"),
            vec!["undefined_symbol"]
        );
        assert_eq!(
            codes("y equ\n    move.l #y,d0\n"),
            vec!["wrong_operand_count"]
        );
    }

    #[test]
    fn a_name_that_is_nowhere_is_undefined_and_not_a_forward_reference() {
        assert_eq!(codes("    org nowhere\n"), vec!["undefined_symbol"]);
        assert_eq!(codes("    bra nowhere\n"), vec!["undefined_symbol"]);
    }

    #[test]
    fn a_division_by_zero_is_reported_once() {
        assert_eq!(codes("    org 1/0\n"), vec!["division_by_zero"]);
        assert_eq!(codes("    move.l #1/0,d0\n"), vec!["division_by_zero"]);
    }

    #[test]
    fn two_lines_over_one_address_are_an_error_at_the_second() {
        let source = "    org $2000\n    dc.l 1\n    org $2002\n    dc.l 2\n";
        let (_, diagnostics) = assemble(source);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), "address_used_twice");
        assert_eq!(diagnostics[0].location.line, 3);
        assert_eq!(diagnostics[0].related[0].0.line, 1);
    }

    #[test]
    fn a_label_reads_the_address_of_the_line_it_is_on() {
        let program = assemble("start:\n    nop\ndone: rts\n").0;
        assert_eq!(program.symbols()["start"].value, 0x1000);
        assert_eq!(program.symbols()["done"].value, 0x1004);
    }

    #[test]
    fn a_local_label_is_scoped_by_the_global_label_above_it() {
        let source = "\
first:
    nop
.loop:
    bra .loop
second:
    nop
.loop:
    bra .loop
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["first:loop"].value, 0x1004);
        assert_eq!(program.symbols()["second:loop"].value, 0x100c);
        // Each `bra` reaches the `.loop` of its own scope.
        let targets: Vec<String> = program
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction.instruction {
                Instruction::BRA(address) => Some(format!("${address:x}")),
                _ => None,
            })
            .collect();
        assert_eq!(targets, vec!["$1004", "$100c"]);
    }

    #[test]
    fn the_entry_point_is_end_then_start_then_the_first_instruction() {
        assert_eq!(assemble("    nop\n    nop\n").0.entry(), 0x1000);
        assert_eq!(
            assemble("    nop\nSTART:\n    nop\n").0.entry(),
            0x1004,
            "a label named START beats the first instruction"
        );
        assert_eq!(
            assemble("    nop\nstart:\n    nop\n").0.entry(),
            0x1004,
            "and the fallback name is matched whatever its case, which is the one \
             place a name is read case insensitively"
        );
        assert_eq!(
            assemble("    nop\nStart:\n    nop\nSTART:\n    rts\n")
                .0
                .entry(),
            0x1008,
            "an exact `START` still wins over one that differs in case"
        );
        assert_eq!(
            assemble("    nop\nStart:\n    nop\nstart:\n    rts\n")
                .0
                .entry(),
            0x1000,
            "and two labels that differ only in case are not guessed between"
        );
        assert_eq!(
            assemble("    nop\nSTART:\n    nop\nlast:\n    rts\n    end last\n")
                .0
                .entry(),
            0x1008,
            "and `end` beats both"
        );
    }

    #[test]
    fn end_without_an_address_warns_and_falls_back() {
        let source = "    nop\n    end\n";
        assert_eq!(codes(source), vec!["end_without_an_address"]);
        assert_eq!(assemble(source).0.entry(), 0x1000);
    }

    /// `END START` against a Label written `start`, which is what EASy68K's own
    /// `mouseWindowSize.X68` does.
    ///
    /// The Entry point is taken from the Label and a warning says the case
    /// differs; the rule for every other use of a name is untouched, which the
    /// second half asserts.
    #[test]
    fn end_finds_a_label_that_differs_only_in_case_and_warns() {
        let source = "    nop\nstart:\n    nop\n    end START\n";
        assert_eq!(codes(source), vec!["entry_point_case_mismatch"]);
        assert_eq!(assemble(source).0.entry(), 0x1004);
        let diagnostics = assemble(source).1;
        assert_eq!(
            diagnostics[0].message(),
            "`START` is not defined, `start` is, and the program starts at `start`"
        );
        assert_eq!(
            diagnostics[0].hint(),
            Some("symbols are case sensitive here, so write `end start`".to_string())
        );
        // Everywhere else a name is still case sensitive, `end`'s own operand
        // included once it is an expression rather than one bare name.
        assert_eq!(
            codes("    nop\nstart:\n    nop\n    bra START\n"),
            vec!["undefined_symbol"]
        );
        assert_eq!(
            codes("    nop\nstart:\n    nop\n    end START+2\n"),
            vec!["undefined_symbol"]
        );
        // A Constant is not a Label, and the Entry point is a Label's address.
        assert_eq!(
            codes("start equ $2000\n    nop\n    end START\n"),
            vec!["undefined_symbol"]
        );
    }

    /// An invocation of a Macro the File defines names the feature.
    ///
    /// The parser skips a Macro's body and keeps its name
    /// ([`ParsedFile::macros`](super::parser::ParsedFile)), so a call site
    /// further down is not an unknown word: "start it in column 1 if `DELAY` is
    /// a label" would tell a student to make the program worse.
    #[test]
    fn a_macro_invocation_names_the_macro_and_not_a_label() {
        let source = "DELAY   macro\n    nop\n    endm\n    DELAY 1\n";
        assert_eq!(
            codes(source),
            vec!["unimplemented_operation", "unimplemented_operation"]
        );
        let invocation = assemble(source).1.remove(1);
        assert_eq!(
            invocation.message(),
            "`DELAY` is not implemented: it is a macro, and macros are not assembled"
        );
        assert_eq!(
            invocation.hint(),
            Some("write the lines of the macro here instead".to_string())
        );
        assert_eq!(invocation.related[0].0.line, 0, "the `macro` line");
        // A word that is no Macro of this File keeps the label hint, which is
        // the row of `label_rule` it belongs to.
        assert_eq!(codes("    DELAYS 1\n"), vec!["unknown_mnemonic"]);
    }

    #[test]
    fn a_line_after_end_is_not_assembled_and_says_so_once() {
        let source = "    nop\n    end $1000\n    nop\n    nop\n";
        assert_eq!(codes(source), vec!["code_after_end"]);
        assert_eq!(addresses(source), vec![0x1000]);
    }

    #[test]
    fn a_comment_after_end_is_what_every_easy68k_program_has() {
        let source = "    nop\n    end $1000\n* the editor writes these\n\n";
        assert!(codes(source).is_empty());
    }

    #[test]
    fn the_current_address_is_where_the_line_is_laid_out() {
        let program = assemble("    org $2000\nhere: dc.l *\n    dc.l *\n").0;
        assert_eq!(
            memory("    org $2000\nhere: dc.l *\n    dc.l *\n"),
            vec![
                (0x2000, "00002000".to_string()),
                (0x2004, "00002004".to_string())
            ]
        );
        assert_eq!(program.symbols()["here"].value, 0x2000);
    }

    #[test]
    fn org_reads_the_current_address_before_it_moves() {
        // `Directives/org.htm`'s own alignment idiom.
        let source = "    org $2000\n    dc.b 1\n    org (*+1)&-2\nhere: dc.w 2\n";
        assert_eq!(assemble(source).0.symbols()["here"].value, 0x2002);
    }

    #[test]
    fn a_directive_given_an_addressing_mode_says_a_value_was_expected() {
        assert_eq!(codes("    dc.b d0\n"), vec!["value_expected"]);
        assert_eq!(codes("    org (a0)\n"), vec!["value_expected"]);
    }

    #[test]
    fn a_directive_with_the_wrong_number_of_operands_says_so() {
        assert_eq!(codes("    org\n"), vec!["wrong_operand_count"]);
        assert_eq!(codes("    org $1000,$2000\n"), vec!["wrong_operand_count"]);
        assert_eq!(codes("    dcb.b 4\n"), vec!["wrong_operand_count"]);
        assert_eq!(codes("    dc.b\n"), vec!["wrong_operand_count"]);
    }

    #[test]
    fn a_directive_that_carries_no_size_says_so() {
        assert_eq!(codes("    org.w $1000\n"), vec!["invalid_size"]);
        assert_eq!(codes("    dc.s 1\n"), vec!["invalid_size"]);
    }

    #[test]
    fn a_data_item_that_does_not_fit_its_size_says_so() {
        assert_eq!(codes("    dc.b 300\n"), vec!["value_out_of_range"]);
        assert!(codes("    dc.b 255\n").is_empty());
        assert!(codes("    dc.b -1\n").is_empty());
        assert_eq!(codes("    dcb.w 2,$10000\n"), vec!["value_out_of_range"]);
    }

    #[test]
    fn a_count_that_does_not_fit_in_memory_says_so() {
        assert_eq!(codes("    ds.l $1000000\n"), vec!["value_out_of_range"]);
        assert_eq!(codes("    org $1000000\n"), vec!["value_out_of_range"]);
    }

    #[test]
    fn the_directives_of_the_later_phases_name_themselves() {
        for source in [
            "    include 'io.x68'\n",
            "    incbin 'sprite.bin'\n",
            "    memory $1000,$2000,ROM\n",
            "    macro foo\n    endm\n",
            "    ifeq 1\n    endc\n",
            "    if.l d0 <eq> #1 then.s\n",
        ] {
            let codes = codes(source);
            assert_eq!(
                codes.first(),
                Some(&"unimplemented_operation"),
                "`{source}` raised {codes:?}"
            );
        }
    }

    #[test]
    fn the_ignored_directives_are_ignored_in_silence() {
        assert!(codes("    opt cre\n    list\n    nolist\n    page\n").is_empty());
    }

    // -- `reg`, `fail` and `simhalt` (phase 2) -----------------------------

    #[test]
    fn reg_names_a_register_list_and_movem_reads_it_in_both_directions() {
        // `Directives/reg.htm`'s own example, and the mask it stands for is the
        // one the written list gives: a `reg` name is lowered exactly as the
        // list would be, predecrement reversal included.
        let source = "\
AllRegs reg d0-d2/a0
start:
    movem.l AllRegs,-(a7)
    movem.l (a7)+,AllRegs
    movem.l d0-d2/a0,-(a7)
    movem.l (a7)+,d0-d2/a0
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            program.symbols()["AllRegs"].kind,
            SymbolKind::RegisterList,
            "a `reg` symbol is a register list and not a value"
        );
        let masks: Vec<(u16, bool)> = program
            .instructions()
            .iter()
            .map(|assembled| match assembled.instruction {
                Instruction::MOVEM {
                    registers_mask,
                    direction,
                    ..
                } => (registers_mask, direction == TargetDirection::ToMemory),
                other => panic!("expected a movem, got {other:?}"),
            })
            .collect();
        assert_eq!(masks[0], masks[2], "out through `-(a7)`, mask reversed");
        assert_eq!(masks[1], masks[3], "back through `(a7)+`");
        assert_eq!(masks[1].0, 0b1_0000_0111, "d0, d1, d2 and a0");
        assert_eq!(masks[0].0, 0b1_0000_0111u16.reverse_bits());
    }

    #[test]
    fn reg_takes_a_single_register_as_a_list_of_one() {
        let source = "one reg d3
start:
    movem.w one,(a0)
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            program.symbols()["one"].value,
            0b1000,
            "a single register is a list of one"
        );
    }

    #[test]
    fn reg_without_a_name_says_so() {
        assert_eq!(
            codes(
                "    reg d0-d2
"
            ),
            ["directive_needs_a_label"]
        );
    }

    #[test]
    fn reg_takes_a_register_list_and_nothing_else() {
        assert_eq!(
            codes(
                "regs reg #5
"
            ),
            ["register_list_expected"]
        );
        assert_eq!(
            codes(
                "regs reg (a0)
"
            ),
            ["register_list_expected"]
        );
        assert_eq!(
            codes(
                "regs reg
"
            ),
            ["wrong_operand_count"]
        );
        assert_eq!(
            codes(
                "regs reg.w d0-d2
"
            ),
            ["invalid_size"]
        );
    }

    #[test]
    fn a_register_list_in_an_expression_is_refused() {
        // EASy68K's "Register list symbol used in an expression": the Symbol
        // stands for a `movem` operand and has no value at all.
        let source = "\
AllRegs reg d0-d2
start:
    move.l #AllRegs,d0
    move.l AllRegs,d1
    move.l AllRegs+1,d2
";
        assert_eq!(
            codes(source),
            [
                "register_list_in_expression",
                "register_list_in_expression",
                "register_list_in_expression"
            ]
        );
    }

    #[test]
    fn a_register_list_has_to_be_defined_above_the_movem_that_reads_it() {
        // EASy68K's "Register list symbol not previously defined". It is the one
        // forward reference that is refused in an instruction Operand: a
        // register list is not a value the second pass can fill in, it is how
        // the instruction is encoded.
        let source = "start:
    movem.l AllRegs,-(a7)
AllRegs reg d0-d2
";
        let (_, diagnostics) = assemble(source);
        let codes: Vec<&str> = diagnostics.iter().map(Diagnostic::code).collect();
        assert_eq!(codes, ["register_list_not_defined_yet"]);
        assert_eq!(
            diagnostics[0].related.len(),
            1,
            "the `reg` line is a related location"
        );
    }

    #[test]
    fn a_name_that_is_not_a_register_list_says_so() {
        // EASy68K's "Symbol is not a register list symbol". The mode check is
        // not made as well: `count` *is* a legal absolute address, and "it
        // cannot be an absolute address here" is not the mistake.
        let source = "count equ 4
start:
    movem.l count,-(a7)
";
        assert_eq!(codes(source), ["not_a_register_list"]);
    }

    #[test]
    fn a_name_in_the_memory_position_of_a_movem_is_an_address() {
        // `movem.l table,d0-d2` reads the registers back *from* `table`, so the
        // list is the second Operand and the first is an ordinary address. A
        // name there is judged as one and nothing about register lists is said.
        let source = "\
    org $2000
table: ds.l 3
start:
    movem.l table,d0-d2
    movem.l d0-d2,table
";
        assert!(codes(source).is_empty(), "{:?}", codes(source));
    }

    #[test]
    fn a_register_list_defined_below_is_refused_wherever_it_stands() {
        // A `reg` Symbol has no value at all, so it can never be read as the
        // address the other direction would allow there.
        let source = "start:\n    movem.l AllRegs,d0-d2\nAllRegs reg d0-d2\n";
        assert_eq!(codes(source), ["register_list_not_defined_yet"]);
    }

    #[test]
    fn a_movem_with_the_wrong_count_is_told_about_the_count_and_nothing_else() {
        // The name is still read as the list it is, so the only thing left to
        // say is how many operands `movem` takes.
        let source = "AllRegs reg d0-d2\nstart:\n    movem.l AllRegs\n";
        assert_eq!(codes(source), ["wrong_operand_count"]);
    }

    #[test]
    fn a_name_that_is_defined_nowhere_keeps_its_own_message() {
        // A name the program never defines is answered by the evaluator, which
        // knows the closest name there is, and by the analyzer, which knows
        // that a register list belongs there. Between them they are the
        // diagnosis of a missing `reg` line.
        let source = "start:
    movem.l AllRegs,-(a7)
";
        assert_eq!(
            codes(source),
            ["undefined_symbol", "invalid_addressing_mode"]
        );
    }

    #[test]
    fn fail_reports_its_message_word_for_word_and_the_assembly_carries_on() {
        // `Directives/fail.htm`: the message is the rest of the line, commas
        // and all, and "the assembly proceeds normally after the error has been
        // printed" — so the `move` below it is still laid out.
        let source = "\
start:
    fail ERROR, Argument missing in call to foo macro.
    move.l #1,d0
";
        let (program, diagnostics) = assemble(source);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
                .collect::<Vec<_>>(),
            [(
                "user_defined_error",
                "ERROR, Argument missing in call to foo macro.".to_string()
            )]
        );
        assert_eq!(
            program.instructions().len(),
            1,
            "the line after a `fail` is still assembled"
        );
    }

    #[test]
    fn fail_without_a_message_uses_easy68ks_default() {
        let (_, diagnostics) = assemble(
            "    fail
",
        );
        assert_eq!(diagnostics[0].code(), "user_defined_error");
        assert_eq!(diagnostics[0].message(), UNSPECIFIED_FAILURE);
    }

    #[test]
    fn a_label_on_a_fail_names_the_address_of_the_line() {
        let source = "    org $2000
here fail no good
    dc.b 1
";
        let (program, _) = assemble(source);
        assert_eq!(program.symbols()["here"].value, 0x2000);
    }

    #[test]
    fn simhalt_is_an_instruction_of_four_bytes() {
        let source = "start:
    nop
halt simhalt
    nop
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(addresses(source), vec![0x1000, 0x1004, 0x1008]);
        assert_eq!(
            program.symbols()["halt"].value,
            0x1004,
            "a label on `simhalt` names its address"
        );
        assert!(matches!(
            program.instructions()[1].instruction,
            Instruction::SIMHALT
        ));
    }

    #[test]
    fn simhalt_reads_the_rest_of_its_line_as_a_comment() {
        // `Directives/simhalt.htm`'s usage line is `LABEL SIMHALT comment`, and
        // line 206 of `tests/corpus/easy68k/graphicSound.X68` is
        // `SIMHALT                 Halt Simulator`.
        assert_eq!(
            codes("    simhalt                 Halt Simulator\n"),
            ["bare_comment"],
            "the comment is a comment and nothing in it is an operand"
        );
        assert_eq!(
            codes(
                "    simhalt.w
"
            ),
            ["invalid_size"]
        );
    }

    // -- the label rules of `docs/grammar.md` 2.6 --------------------------

    #[test]
    fn the_directives_that_give_a_name_to_something_need_a_label() {
        for source in [
            "    equ 12
",
            "    set 12
",
            "    reg d0-d2
",
        ] {
            assert_eq!(
                codes(source),
                ["directive_needs_a_label"],
                "`{source}` needs a label"
            );
        }
    }

    #[test]
    fn the_directives_that_take_no_label_say_so() {
        // EASy68K's own list: `page` ("No label is permitted") and the
        // conditional-assembly directives ("IFxx and ENDC directives may not be
        // labeled").
        assert_eq!(
            codes(
                "heading page
"
            ),
            ["label_not_allowed"]
        );
        assert_eq!(
            codes(
                "skip ifeq 1
"
            ),
            ["label_not_allowed", "unimplemented_operation"],
            "the label rule and the missing feature are separate mistakes"
        );
        assert_eq!(
            codes(
                "done endc
"
            ),
            ["label_not_allowed", "unimplemented_operation"]
        );
        // The name is still defined, so a use of it is not reported as well.
        let (program, _) = assemble(
            "    org $2000
heading page
    dc.b 1
",
        );
        assert_eq!(program.symbols()["heading"].value, 0x2000);
    }

    #[test]
    fn every_other_directive_takes_a_label_or_no_label() {
        assert!(codes(
            "here org $2000
"
        )
        .is_empty());
        assert!(codes(
            "here dc.b 1
"
        )
        .is_empty());
        assert!(codes(
            "here nolist
"
        )
        .is_empty());
        assert!(codes(
            "    org $2000
"
        )
        .is_empty());
    }

    // -- `section` and `offset` (phase 2) ----------------------------------

    #[test]
    fn a_program_starts_in_section_zero_at_the_default_origin() {
        // "By default, the assembler will begin with section 0"
        // (`Directives/section.htm`), and s68k's default origin is `$1000`, so
        // that is where section 0's counter starts.
        let source = "zero section\n    dc.b 1\n";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["zero"].value, 0);
        assert_eq!(memory(source), vec![(0x1000, "01".to_string())]);
    }

    #[test]
    fn section_switches_between_sixteen_location_counters() {
        // `Directives/section.htm`'s own example, with an instruction in place
        // of its `<code>`: each section goes on from where it left off.
        let source = "\
CODE    equ 0
DATA    equ 1
    section DATA
    org $2000
msg1 dc.b 'Hello',0
    section CODE
    org $1000
    nop
    nop
    section DATA
msg2 dc.b 'Bye',0
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        // `msg1` is six bytes from `$2000`, so `msg2` goes to `$2006` when the
        // program comes back to section 1, and the two `nop`s are in section 0.
        assert_eq!(program.symbols()["msg1"].value, 0x2000);
        assert_eq!(program.symbols()["msg2"].value, 0x2006);
        assert_eq!(addresses(source), vec![0x1000, 0x1004]);
    }

    #[test]
    fn org_inside_a_section_moves_that_sections_counter() {
        // "The ORG directive may be used within a section, at any time, to set
        // the current program location" — the current one, and no other.
        let source = "\
    section 1
    org $3000
    dc.b 1
    section 0
    dc.b 2
    section 1
    dc.b 3
";
        assert!(codes(source).is_empty());
        assert_eq!(
            memory(source),
            vec![
                (0x1000, "02".to_string()),
                (0x3000, "01".to_string()),
                (0x3001, "03".to_string()),
            ]
        );
    }

    #[test]
    fn a_label_on_a_section_names_the_address_it_goes_on_from() {
        // The rule a Label on an `org` follows: the line moves the address and
        // the name is where the program goes on from, not where it was.
        let source = "\
    section 1
    org $3000
    dc.b 1
    section 0
here section 1
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["here"].value, 0x3001);
        assert_eq!(program.symbols()["here"].kind, SymbolKind::Label);
    }

    #[test]
    fn a_section_that_is_used_for_the_first_time_starts_at_zero() {
        let source = "    section 1\n    dc.b 1\n";
        assert!(codes(source).is_empty());
        assert_eq!(memory(source), vec![(0, "01".to_string())]);
    }

    #[test]
    fn two_sections_over_one_address_are_still_an_overlap() {
        // EASy68K "does not check for overlapping sections"; s68k's overlap
        // error is a deliberate deviation (ADR 0001) and an address is an
        // address whichever section wrote it.
        let source = "\
    section 1
    org $1000
    dc.b 1
    section 0
    dc.b 2
";
        assert_eq!(codes(source), vec!["address_used_twice"]);
    }

    #[test]
    fn a_section_number_may_be_a_symbol_and_may_not_be_a_forward_reference() {
        assert!(codes("DATA equ 1\n    section DATA\n").is_empty());
        assert_eq!(
            codes("    section DATA\nDATA equ 1\n"),
            vec!["forward_reference_not_allowed"],
            "a section number decides the layout"
        );
    }

    #[test]
    fn a_section_number_outside_the_sixteen_says_so() {
        assert_eq!(codes("    section 16\n"), vec!["value_out_of_range"]);
        assert_eq!(codes("    section -1\n"), vec!["value_out_of_range"]);
        assert!(codes("    section 15\n").is_empty());
        let (_, diagnostics) = assemble("    section 16\n");
        assert_eq!(
            diagnostics[0].message(),
            "the number of `section` is 0 to 15, and `16` is outside it"
        );
        // The section in force does not change, so the line below the refused
        // one is laid out where it would have been.
        assert_eq!(
            memory("    section 16\n    dc.b 1\n"),
            vec![(0x1000, "01".to_string())]
        );
    }

    #[test]
    fn section_with_no_number_names_the_section_in_force() {
        let source = "    section 3\nhere section\n";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["here"].value, 3);
        assert_eq!(
            program.symbols()["here"].kind,
            SymbolKind::Constant,
            "the number of a section is a value, not an address"
        );
    }

    #[test]
    fn section_with_no_number_needs_a_label() {
        // The one label rule that depends on the Operand and not on the name of
        // the Directive, which is why `label_rule_of` does not carry it.
        assert_eq!(codes("    section\n"), vec!["directive_needs_a_label"]);
        assert!(codes("here section 1\n").is_empty());
    }

    #[test]
    fn offset_moves_an_address_and_places_nothing() {
        // `Directives/offset.htm`'s first example.
        let source = "\
    offset 0
label1 ds.w 1
label2 ds.b 2
    org *
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["label1"].value, 0);
        assert_eq!(program.symbols()["label2"].value, 2);
        assert!(
            program.memory().is_empty(),
            "an offset region reserves nothing: {:?}",
            program.memory()
        );
    }

    #[test]
    fn a_name_defined_in_an_offset_region_is_a_constant() {
        let source = "\
    offset 0
field ds.w 1
    org *
here dc.b 1
";
        let (program, _) = assemble(source);
        assert_eq!(
            program.symbols()["field"].kind,
            SymbolKind::Constant,
            "an offset is a value and no line of the program is laid out at it"
        );
        assert_eq!(
            program.symbols()["here"].kind,
            SymbolKind::Label,
            "and the region is over by then"
        );
    }

    #[test]
    fn org_star_restores_the_address_the_offset_region_shadowed() {
        // "ORG * restores the code to the address in use prior to the OFFSET"
        // (`Directives/offset.htm`). Every other `*` inside the region is the
        // region's own counter.
        let source = "\
    org $2000
    dc.b 1
    offset 0
first ds.w 1
mark equ *
    org *
here dc.b 2
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["mark"].value, 2, "`*` is the counter");
        assert_eq!(program.symbols()["here"].value, 0x2001);
    }

    #[test]
    fn an_org_with_an_address_ends_an_offset_region_too() {
        let source = "\
    offset 0
field ds.w 1
    org $3000
here dc.b 1
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["here"].value, 0x3000);
        assert_eq!(program.symbols()["here"].kind, SymbolKind::Label);
    }

    #[test]
    fn a_section_ends_an_offset_region_as_an_org_does() {
        // The help says nothing about the two together; a `section` sets the
        // current address, so it ends the region, and refusing the line would
        // refuse a program EASy68K assembles (ADR 0001).
        let source = "\
    offset 0
field ds.w 1
    section 1
    org $3000
here dc.b 1
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["here"].value, 0x3000);
        assert_eq!(program.symbols()["here"].kind, SymbolKind::Label);
    }

    #[test]
    fn end_closes_an_offset_region() {
        let source = "\
start:
    nop
    offset 0
field ds.w 1
done end start
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["field"].kind, SymbolKind::Constant);
        assert_eq!(program.symbols()["done"].kind, SymbolKind::Label);
        assert_eq!(program.symbols()["done"].value, 0x1004);
    }

    #[test]
    fn a_line_that_would_produce_bytes_in_an_offset_region_says_so() {
        // "No machine code is generated by instructions or directives following
        // an OFFSET directive" (`Directives/offset.htm`), so a line that meant
        // to produce some is told rather than dropped in silence.
        let source = "\
    offset 0
field ds.w 1
    move.l #1,d0
    dc.b 1
    simhalt
";
        let (program, diagnostics) = assemble(source);
        let codes: Vec<&str> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect();
        assert_eq!(
            codes,
            vec![
                "no_bytes_in_an_offset_region",
                "no_bytes_in_an_offset_region",
                "no_bytes_in_an_offset_region"
            ],
            "`ds` is what the region is made of and says nothing"
        );
        assert!(program.instructions().is_empty());
        assert!(program.memory().is_empty());
    }

    #[test]
    fn an_offset_region_opens_whatever_its_expression_says() {
        // The rule `equ` follows for a value it cannot work out: the mistake is
        // reported once, and the region opens so that the table below it is not
        // laid out into memory and reported again line by line.
        assert_eq!(
            codes("    offset LATER\nfield ds.w 1\nLATER equ 4\n"),
            vec!["forward_reference_not_allowed"]
        );
        assert_eq!(
            codes("    offset\nfield ds.w 1\n"),
            vec!["wrong_operand_count"]
        );
        // A counter near the end of the 64 bits an Expression is computed in
        // saturates instead of wrapping round: the region is nonsense either
        // way, it has been reported once, and nothing overflows.
        assert_eq!(
            codes("    offset $7fffffffffffffff\nfield ds.w 1\n"),
            vec!["constant_above_32_bits"]
        );
    }

    #[test]
    fn a_negative_offset_is_the_stack_frame_of_the_help() {
        // `Directives/offset.htm`'s second example: three long words below a
        // frame pointer, and the offsets are negative.
        let source = "\
SIZE equ -3*4
    offset SIZE
num1 ds.l 1
num2 ds.l 1
num3 ds.l 1
    org *
    link a0,#SIZE
";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["num1"].value, -12);
        assert_eq!(program.symbols()["num2"].value, -8);
        assert_eq!(program.symbols()["num3"].value, -4);
        assert_eq!(addresses(source), vec![0x1000]);
    }

    #[test]
    fn a_word_in_an_offset_region_aligns_up_from_a_negative_offset() {
        // A word starts on an even address, and rounding *up* from `-11` is
        // `-10`: the remainder of the alignment is Euclidean for exactly this.
        let source = "    offset -11\nfield ds.w 1\nafter ds.b 1\n";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["field"].value, -10);
        assert_eq!(program.symbols()["after"].value, -8);
    }

    #[test]
    fn a_set_variable_may_be_redefined() {
        let source = "size set 38\n    move.l #size,d0\nsize set 32\n    move.l #size,d1\n";
        let (program, diagnostics) = assemble(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(program.symbols()["size"].kind, SymbolKind::Variable);
        let immediates: Vec<u32> = program
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction.instruction {
                Instruction::MOVE(EncodedOperand::Immediate(value), _, _) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(immediates, vec![38, 32], "each use sees the latest `set`");
    }

    #[test]
    fn an_instruction_carries_its_line_and_its_source() {
        let program = assemble("* a comment\n    move.l #1,d0\n").0;
        let instruction = &program.instructions()[0];
        assert_eq!(instruction.location.line, 1);
        assert_eq!(instruction.source, "    move.l #1,d0");
        assert_eq!(instruction.size, 4);
        assert_eq!(
            program.instruction_at(0x1000).map(|i| i.address),
            Some(0x1000)
        );
    }
}
