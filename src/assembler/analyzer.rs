//! The analyzer: every Operation judged against the instruction table.
//!
//! [ADR
//! 0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md)
//! makes this the only place that knows what an instruction takes. The parser
//! has read a well-formed Operand and said nothing about whether it belongs
//! where it stands; the analyzer compares it with
//! [`instructions::table`](super::instructions::table) and answers, in one
//! sentence, what was found, what is allowed there and what was probably meant.
//!
//! # What it checks, in order
//!
//! 1. the name is a Mnemonic — a near miss gets a "did you mean", an indented
//!    word gets the Label hint, a real 68000 instruction s68k does not
//!    assemble gets the reason, and one of a later processor that s68k's
//!    dialect does not have yet says so;
//! 2. the number of Operands, and the special registers `sr`, `ccr` and `usp`
//!    among them, pick the Form, which has to be one the Program's processor
//!    has: `extb.l` and `move ccr,<ea>` in a 68000 Program are refused with
//!    the 68000's way of doing the same and the `opt p=` line that would let
//!    them, as suggested fixes (ADR 0011);
//! 3. the size suffix against that Form, and the byte rule for address
//!    registers;
//! 4. each Operand's mode against its position;
//! 5. the two Operands together, where the instruction has only one memory
//!    access, and the two pairs the 68000 has no encoding for although each
//!    Operand fits its position: an address register added into memory and a
//!    static bit number on an immediate;
//! 6. the values it can already work out: the count of a quick form, a shift
//!    count, a bit number, a displacement, how far a PC-relative Operand has to
//!    reach from its own extension word, the displacement of `link`, the width
//!    an absolute address is forced to, an immediate against its size, how far
//!    a branch or a `DBcc` reaches in the displacement its size gives it, and a
//!    bare number that is probably a missing `#`.
//!
//! What survives is lowered ([`lowering`]), which is where the encoding
//! EASy68K would choose is chosen: the analyzer says whether a line is right,
//! and never which of the 68000's encodings it becomes.
//!
//! # Planning
//!
//! The Layout's first pass has to know how many bytes an instruction takes
//! before it can lay out the next line, and EASy68K decides that size from the
//! same two facts it encodes the line from: whether each Symbol in an Operand
//! is a back reference, and its value. So pass 1 runs the analyzer too, over a
//! [`Context`] that is [`planning`](Context::planning): it judges nothing,
//! raises nothing and lowers the line with whatever it knows, a Forward
//! reference standing for a placeholder value. A Forward reference is never
//! given a short form, whatever its value, so the placeholder changes no
//! size, and the instruction pass 2 lowers is the size pass 1 measured.
//!
//! # Where it sits
//!
//! **After the Layout**, so that every Symbol has a value and the range checks
//! are real rather than skipped: a Label is an address only once the program
//! has been laid out. Everything it cannot work out is `None` and is skipped
//! rather than guessed at — the undefined Symbol and the division by zero are
//! the evaluator's Diagnostics (`src/assembler/expr.rs`, still to be written)
//! and are not raised twice here.
//!
//! When the parser has already reported an error on the line, the Operands are
//! whatever survived recovery, so only the Operation's own name is judged: a
//! cascade of "move takes two operands" under a real mistake teaches nothing.

use super::ast::{self, Line, Operand, Operation, SizeSuffix};
use super::diagnostics::{Diagnostic, DiagnosticKind, SuggestedFix};
use super::expr;
use super::instructions::choice;
use super::instructions::encoded::{BranchSize, Instruction, Size, EXTENSION_WORD_OFFSET};
use super::instructions::lowering::{self, Values};
use super::instructions::table::{
    self, Combination, Family, Form, Implementation, InstructionSpec, Modes,
};
use super::names;
use super::parser::MacroDefinition;
use super::source::{Location, Span};
use crate::machine::Processor;

/// What the analyzer knows about the Symbols of the program.
///
/// The symbol table implements it (`src/assembler/symbols.rs`, still to be
/// written); [`NoSymbols`] is what a caller with no program passes, and every
/// check that needs a value is then skipped rather than guessed at.
pub trait SymbolValues {
    /// The value of a Symbol, or `None` when it has none the analyzer may use.
    fn value_of(&self, name: &str) -> Option<i64>;

    /// Whether a Symbol is defined above the line, or on it: whether a use of
    /// it here is EASy68K's **back reference**, which is what decides a
    /// branch's size, an address's width and the quick forms
    /// ([`instructions::choice`](super::instructions::choice)).
    ///
    /// A Label on the line itself counts, as EASy68K defines it before it
    /// reads the Operands. The default answers that a name with a value is
    /// one, which is right for a table that has no lines to be above; the
    /// Program's own table answers by Position (CONTEXT.md).
    fn is_defined_above(&self, name: &str) -> bool {
        self.value_of(name).is_some()
    }
}

/// A symbol table that knows nothing, for a line judged on its own.
///
/// It is the evaluator's ([`expr::NoSymbols`]), so that a caller with no
/// program has one name for it whichever of the two questions it is answering.
pub use super::expr::NoSymbols;

impl SymbolValues for NoSymbols {
    fn value_of(&self, _name: &str) -> Option<i64> {
        None
    }
}

/// A [`SymbolValues`] read as the evaluator's [`Symbols`](expr::Symbols).
///
/// The analyzer knows a value or knows nothing; the evaluator also tells a
/// forward reference and a Register list apart, and neither is a distinction
/// this phase can make or has to report.
struct AsSymbols<'a>(&'a dyn SymbolValues);

impl expr::Symbols for AsSymbols<'_> {
    fn resolve(&self, name: &str) -> expr::Resolution {
        match self.0.value_of(name) {
            Some(value) => expr::Resolution::Value(value),
            None => expr::Resolution::Unknown { suggestion: None },
        }
    }
}

/// The default origin of a program, `$1000` (the design record, "Layout").
pub const DEFAULT_ORIGIN: i64 = 0x1000;

/// What the phases before the analyzer have worked out: the Symbols, where this
/// line sits, and where the program starts.
pub struct Context<'a> {
    /// The Symbols and their values.
    pub symbols: &'a dyn SymbolValues,
    /// The address this line is laid out at, which is what `*` means in it.
    pub current_address: i64,
    /// The address the program starts at, which is what tells `move.l 5,d0`
    /// from `move.l label,d0`.
    pub origin: i64,
    /// The Macros the File defines, which the parser collected while it skipped
    /// their bodies. An Operation named after one of them is an invocation and
    /// not an unknown word, and is answered with the feature it needs.
    pub macros: &'a [MacroDefinition],
    /// Whether this is the Layout's first pass measuring the line rather than
    /// the second judging it (the module's "Planning"): nothing is raised, and
    /// a value that cannot be worked out yet is lowered as a placeholder.
    pub planning: bool,
    /// The processor the Program is written for, which `opt p=` chose: a Form
    /// of a later one is refused, and a branch written `.l` is long only on
    /// the 68020.
    pub processor: Processor,
    /// Whether the Program has the 68020's bit-field instructions, through
    /// `opt bit` or the 68020.
    pub bit_fields: bool,
    /// Where the source chooses its processor, or would, which a fix that
    /// changes it edits; `None` for a line judged with no Program around it,
    /// which is then refused with the hint and no fix.
    pub processor_line: Option<&'a ProcessorLine>,
}

/// Where the source chooses its processor, or would: what a suggested fix
/// that changes the processor edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessorLine {
    /// The value an `opt p=` line chose it with, `68010` in `opt p=68010`.
    Chosen(Location),
    /// No line chose one, and a line that does goes above the line at
    /// `above`, the first of the Entry file that is neither blank nor a
    /// Comment line, indented as `indentation` says.
    Missing {
        /// The line the new one goes above, its columns empty.
        above: Location,
        /// The whitespace the new line starts with.
        indentation: String,
    },
}

impl<'a> Context<'a> {
    /// A Context over `symbols`, with the default origin and a current address
    /// of `origin`, for a 68000 Program.
    pub fn new(symbols: &'a dyn SymbolValues) -> Self {
        Self {
            symbols,
            current_address: DEFAULT_ORIGIN,
            origin: DEFAULT_ORIGIN,
            macros: &[],
            planning: false,
            processor: Processor::M68000,
            bit_fields: false,
            processor_line: None,
        }
    }

    /// The value of an Expression, in 64 bits, or `None` when it cannot be
    /// worked out — an undefined Symbol, a division by zero, an overflow.
    ///
    /// This is a fold and not a diagnosis: whatever it cannot answer has
    /// already been reported by the evaluator, and reporting it again here
    /// would double every message. The arithmetic itself is
    /// [`expr::value`]'s, which is the same one the Layout and the Directives
    /// use.
    pub fn evaluate(&self, expression: &ast::Expr) -> Option<i64> {
        expr::value(expression, &AsSymbols(self.symbols), self.current_address)
    }
}

/// What a value that cannot be worked out yet is lowered as while planning: a
/// Forward reference, or an Expression that holds one.
///
/// Any number would do, since a Forward reference is never given a short form
/// and so its value decides no size; 0 is the one that reads as "nothing".
const PLACEHOLDER: i64 = 0;

impl Values for Context<'_> {
    fn value_of(&self, expression: &ast::Expr) -> Option<i64> {
        match self.evaluate(expression) {
            None if self.planning => Some(PLACEHOLDER),
            value => value,
        }
    }

    fn is_back_reference(&self, expression: &ast::Expr) -> bool {
        expr::every_symbol(expression, &|name| self.symbols.is_defined_above(name))
    }

    fn instruction_address(&self) -> i64 {
        self.current_address
    }

    fn processor(&self) -> Processor {
        self.processor
    }
}

/// Reads one line's Operation against the instruction table.
///
/// One Analyzer a Source line: it holds the line's text, so that a Diagnostic
/// can quote the Operand it is about and land on the right columns.
pub struct Analyzer<'a> {
    file: &'a str,
    line_index: usize,
    text: &'a str,
    context: &'a Context<'a>,
    diagnostics: Vec<Diagnostic>,
    /// Whether an error has been raised about this line's Operation, which is
    /// what decides whether anything is lowered and whether the
    /// `mnemonic_used_as_label` suggestion is worth making.
    failed: bool,
}

impl<'a> Analyzer<'a> {
    /// An Analyzer over the `line_index`th line of `file`, whose text is
    /// `text`, with everything the earlier phases worked out.
    pub fn new(
        file: &'a str,
        line_index: usize,
        text: &'a str,
        context: &'a Context<'a>,
    ) -> Analyzer<'a> {
        Analyzer {
            file,
            line_index,
            text,
            context,
            diagnostics: Vec::new(),
            failed: false,
        }
    }

    /// Judge one line and, when it is an instruction and nothing is wrong with
    /// it, build the [`Instruction`] the Interpreter runs.
    ///
    /// `parser_reported_an_error` says whether the parser has already found an
    /// error on this line: its Operands are then whatever survived recovery, so
    /// only the Operation's name is judged and nothing is lowered.
    ///
    /// A line with no Operation, or one whose Operation is a Directive, gives
    /// `None` and no Diagnostic: the Directives are phase 2's.
    pub fn analyze_line(
        &mut self,
        line: &Line,
        parser_reported_an_error: bool,
    ) -> Option<Instruction> {
        let operation = line.operation.as_ref()?;
        self.analyze_operation(line, operation, parser_reported_an_error)
    }

    /// Everything the analyzer found, in the order it found it.
    pub fn finish(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn analyze_operation(
        &mut self,
        line: &Line,
        operation: &Operation,
        parser_reported_an_error: bool,
    ) -> Option<Instruction> {
        let name = operation.lowercase_name();
        let spec = match table::lookup(&name) {
            Some(spec) => spec,
            None => {
                // A Directive is phase 2's to check; anything else is a word
                // this assembler has never heard of.
                if !names::is_directive(&name) {
                    self.unknown_mnemonic(operation);
                }
                return None;
            }
        };
        if let Implementation::NotImplemented {
            reason,
            alternative,
        } = spec.implementation
        {
            self.raise(
                DiagnosticKind::UnimplementedOperation {
                    name: name.clone(),
                    reason: reason.to_string(),
                    alternative: alternative.map(str::to_string),
                },
                operation.name_span,
            );
            self.suggest_a_label(line, operation);
            return None;
        }
        if let Implementation::NotImplementedYet {
            processor,
            bit_field,
        } = spec.implementation
        {
            self.raise(
                DiagnosticKind::UnimplementedInDialect {
                    name: name.clone(),
                    processor,
                    bit_field,
                },
                operation.name_span,
            );
            self.suggest_a_label(line, operation);
            return None;
        }

        let instruction = self.check(line, spec, operation, parser_reported_an_error);
        self.suggest_a_label(line, operation);
        instruction
    }

    /// The checks of the module's list, once the Mnemonic is known.
    fn check(
        &mut self,
        line: &Line,
        spec: &'static InstructionSpec,
        operation: &Operation,
        parser_reported_an_error: bool,
    ) -> Option<Instruction> {
        let operands = &operation.operands;
        if parser_reported_an_error {
            // The Operands are whatever recovery left behind; only the size,
            // which the parser does not judge, is still worth a word.
            self.check_size_alone(spec, operation);
            return None;
        }

        // EASy68K's comment marker read as the current address
        // (`docs/grammar.md` 3.4): `nop *` is an Operation that takes no
        // Operands with one `*` after it, and the `*` is dropped.
        let operands: &[Operand] = if self.star_is_a_comment(spec, operation) {
            &[]
        } else {
            operands
        };

        // A shape a later processor gave the Mnemonic, in a Program written
        // for that processor: it is still to come, and nothing else about the
        // line is worth saying. On an earlier processor the line is judged as
        // the 68000 judges it.
        if let Some(shape) = table::later_shape(spec.mnemonic, operation.size, operands) {
            if self.context.processor.has(shape.processor) {
                self.raise(
                    DiagnosticKind::UnimplementedInDialect {
                        name: self.text_of(operation.span),
                        processor: shape.processor,
                        bit_field: false,
                    },
                    operation.span,
                );
                return None;
            }
        }

        let form = match self.choose_form(spec, operands) {
            Some(form) => form,
            None => {
                let arities = spec.arities();
                self.raise(
                    DiagnosticKind::WrongOperandCount {
                        mnemonic: spec.mnemonic.to_string(),
                        found: operands.len(),
                        expected: arities.clone(),
                        at_least: false,
                    },
                    operation.span,
                );
                self.missing_comma(line, operands, &arities);
                self.check_size_alone(spec, operation);
                return None;
            }
        };

        if !self.context.processor.has(form.processor) {
            self.needs_a_later_processor(spec, form, operation, operands);
            return None;
        }

        let size = self.check_size(spec, form, operation, operands);
        // Where a PC-relative Operand's displacement sits, which is what the
        // distance it reaches is measured from.
        let extension_offset = spec
            .family()
            .map_or(EXTENSION_WORD_OFFSET as i64, |family| {
                lowering::extension_word_offset(family, operands)
            });
        // An Operand that is not allowed where it stands is not asked about its
        // value as well: one mistake, one message.
        let judged_as_a_pair = self.check_operand_pair(spec, operands);
        let mut fits = vec![true; operands.len()];
        for (index, operand) in operands.iter().enumerate() {
            if judged_as_a_pair {
                fits[index] = false;
                continue;
            }
            fits[index] = self.check_mode(spec, form, index, operand);
            if fits[index] {
                self.check_operand_values(operand, extension_offset);
            }
        }
        self.check_combination(spec, form, operands);
        if fits.iter().all(|fits| *fits) {
            self.check_encodable_pair(spec, operands);
        }
        if fits.first() != Some(&false) {
            self.check_instruction_value(spec, operands);
            self.check_bit_number(spec, operands);
        }
        self.check_link_displacement(spec, operands, &fits);
        self.check_branch_distance(spec, operation, operands, &fits);
        self.check_immediate_sizes(spec, size, operands, &fits);
        self.suggest_immediates(form, operands, &fits);

        // Warnings and suggestions leave a program that still builds; an
        // error means there is nothing honest to lower.
        if self.failed {
            return None;
        }
        let family = spec.family()?;
        lowering::lower_operation(family, operation.size, size, operands, self.context)
    }

    // -- the name ----------------------------------------------------------

    /// A word that is neither a Mnemonic nor a Directive name.
    ///
    /// It carries a "did you mean" from the table when something is close, and
    /// the Label hint when the word could have been meant as one — an indented
    /// word with no size suffix, `label_rule` row 8 (`docs/grammar.md` 1.4).
    fn unknown_mnemonic(&mut self, operation: &Operation) {
        let name = operation.name.clone();
        if let Some(definition) = self
            .context
            .macros
            .iter()
            .find(|definition| definition.name.eq_ignore_ascii_case(&name))
        {
            // An invocation of a Macro the File defines. Macros are out of this
            // phase (the design record, "Scope"), and the diagnostic a student
            // reads has to name that feature rather than offer to turn the call
            // into a label.
            let diagnostic = Diagnostic::new(
                DiagnosticKind::UnimplementedOperation {
                    name,
                    reason: "it is a macro, and macros are not assembled".to_string(),
                    alternative: Some("the lines of the macro here".to_string()),
                },
                self.location(operation.name_span),
            )
            .with_related(
                definition.location.clone(),
                format!("`{}` is defined here", definition.name),
            );
            self.failed = true;
            self.diagnostics.push(diagnostic);
            return;
        }
        let suggestion = table::closest_name(
            &name,
            table::suggestible_mnemonics().chain(names::DIRECTIVES.iter().copied()),
        )
        .map(str::to_string);
        let could_be_label = operation.name_span.start > 0 && operation.size.is_none();
        self.raise(
            DiagnosticKind::UnknownMnemonic {
                name,
                suggestion,
                could_be_label,
            },
            operation.name_span,
        );
    }

    /// The commonest way to lose an Operand: writing a space where the comma
    /// goes, `move.l  d0 d1`.
    ///
    /// The Operand field ends at whitespace not beside a comma
    /// (`docs/grammar.md` 1.5, `missing_comma` in 3.7), so the `d1` is EASy68K's
    /// bare Comment field and the line genuinely has one Operand. The count
    /// error above says the count is wrong; this says where the operand went.
    /// The `bare_comment` suggestion cannot, because it is raised once per File
    /// and is usually spent on a real comment long before.
    ///
    /// It is narrow on purpose, the way `expression_split_by_space` is: the
    /// Operand count has to be one no Form takes (which is why this is only
    /// reached from the `wrong_operand_count` above) *and* short of one that a
    /// Form does take, the Comment field has to be bare, and its first word has
    /// to read as an Operand *and* be the whole of the field —
    /// a word of prose after it and nothing is said, because then the field is
    /// a comment that happens to start with a register's name.
    fn missing_comma(&mut self, line: &Line, operands: &[Operand], arities: &[usize]) {
        if !arities.iter().any(|arity| *arity > operands.len()) {
            return;
        }
        if !line.bare_comment {
            return;
        }
        let Some(comment) = &line.comment else {
            return;
        };
        let Some((word, span)) = first_word(comment) else {
            return;
        };
        if !self.reads_as_an_operand(word) {
            return;
        }
        let previous = operands
            .last()
            .map(|operand| operand.span().text(self.text).to_string());
        self.raise(
            DiagnosticKind::MissingCommaBetweenOperands {
                operand: word.to_string(),
                previous,
            },
            span,
        );
    }

    /// Whether a word out of the Comment field would have been an Operand: a
    /// register, the start of an immediate or of an indirect operand, or a name
    /// the program defines.
    fn reads_as_an_operand(&self, word: &str) -> bool {
        word.starts_with('#')
            || word.starts_with('(')
            || word.starts_with("-(")
            || ast::is_reserved_name(word)
            || self.context.symbols.value_of(word).is_some()
    }

    /// `label_rule` row 5: a Mnemonic in column 1 with something wrong with the
    /// line is usually a Label that has lost its colon.
    ///
    /// A suggestion and never an error, because the error is already there —
    /// the line has one, which is the condition for saying this at all.
    fn suggest_a_label(&mut self, line: &Line, operation: &Operation) {
        if !self.failed || line.label.is_some() || operation.name_span.start != 0 {
            return;
        }
        self.raise(
            DiagnosticKind::MnemonicUsedAsLabel {
                name: operation.name.clone(),
            },
            operation.name_span,
        );
    }

    // -- the operands ------------------------------------------------------

    /// The Form the Operands were written in: the first one with their number
    /// that they all fit, and, when they fit none, the first one of that
    /// number.
    ///
    /// Several Mnemonics have more than one Form of the same arity, and the
    /// fallback is what makes their messages the right ones: `cmp (a0)+,(a1)`
    /// fits neither of `cmp`'s, and is judged against the general Form ("there
    /// it takes Dn or An") rather than against the `cmpm` shape it half
    /// resembles.
    ///
    /// **A written `sr` or `ccr` narrows the candidates first.** `move` has
    /// five Forms of two Operands and four of them name a half of the status
    /// register, so the fallback alone would answer `move a0,sr` with "the
    /// second operand of `move` cannot be the status register", which is the
    /// wrong half of the line: the mistake is that `move <ea>,sr` takes no
    /// address register. Only the Forms that agree with every `sr` and `ccr`
    /// that was written are candidates, and a Form that names one in a position
    /// names nothing else there (a test in `table.rs` holds it to that), so the
    /// agreement is exact. When nothing agrees — `move sr,ccr` — every Form of
    /// that arity is a candidate again and the fallback answers as it always
    /// did.
    fn choose_form(
        &self,
        spec: &'static InstructionSpec,
        operands: &[Operand],
    ) -> Option<&'static Form> {
        let of_that_arity: Vec<&'static Form> = spec
            .forms
            .iter()
            .filter(|form| form.arity() == operands.len())
            .collect();
        let agreeing: Vec<&'static Form> = of_that_arity
            .iter()
            .filter(|form| agrees_about_the_special_registers(form, operands))
            .copied()
            .collect();
        let candidates = if agreeing.is_empty() {
            &of_that_arity
        } else {
            &agreeing
        };
        candidates
            .iter()
            .find(|form| form.fits(operands))
            .or(candidates.first())
            .copied()
    }

    /// One Operand against the rule of its position; `false` when it does not
    /// fit, in which case its value is not asked about as well.
    fn check_mode(
        &mut self,
        spec: &'static InstructionSpec,
        form: &'static Form,
        index: usize,
        operand: &Operand,
    ) -> bool {
        let allowed = form.operands[index];
        let Some(mode) = Modes::of(operand) else {
            return true;
        };
        if allowed.contains(mode) {
            return true;
        }
        let suggestion = self.suggestion_for(spec, index, mode, allowed);
        self.raise(
            DiagnosticKind::InvalidAddressingMode {
                mnemonic: spec.mnemonic.to_string(),
                position: index + 1,
                found: operand.description().to_string(),
                allowed: allowed.names(),
                suggestion,
            },
            operand.span(),
        );
        false
    }

    /// What was probably meant, for the mistakes that have a name.
    fn suggestion_for(
        &self,
        spec: &'static InstructionSpec,
        index: usize,
        found: Modes,
        allowed: Modes,
    ) -> Option<String> {
        if found == Modes::AN && !allowed.contains(Modes::AN) {
            if spec.mnemonic == "clr" {
                // EASy68K's own advice, and the 68000's: `clr` has no address
                // register form at all.
                return Some("`suba.l a0,a0` clears an address register".to_string());
            }
            if allowed.contains(Modes::DN) {
                return Some(
                    "an address register holds an address; move it into a data register first"
                        .to_string(),
                );
            }
            return None;
        }
        if found == Modes::IMMEDIATE && !allowed.contains(Modes::IMMEDIATE) && index > 0 {
            return Some("an immediate is a value, and nothing can be written to it".to_string());
        }
        if found == Modes::REGISTER_LIST && !allowed.contains(Modes::REGISTER_LIST) {
            return Some("only `movem` takes a register list".to_string());
        }
        if found == Modes::USP && !["move", "movec"].contains(&spec.mnemonic) {
            return Some(
                "only `move` reaches the user stack pointer, to or from an address register, \
                 and the 68010's `movec`"
                    .to_string(),
            );
        }
        if found == Modes::CONTROL_REGISTER && spec.mnemonic != "movec" {
            return Some(
                "only `movec` reaches the 68010's control registers, to or from a general \
                 register"
                    .to_string(),
            );
        }
        if found.intersects(Modes::STATUS) && !takes_the_status_register(spec) {
            // Only for a Mnemonic that reaches the status register nowhere:
            // `move ccr,ccr` is a `move` written wrong and is told what `move`
            // takes there, not that `move` is one of the four.
            return Some(
                "only `move`, `andi`, `ori` and `eori` reach the status register".to_string(),
            );
        }
        if found.intersects(Modes::PC_RELATIVE)
            && !allowed.intersects(Modes::PC_RELATIVE)
            && allowed.intersects(Modes::MEMORY)
        {
            // The position does reach memory, so the mistake is not the place
            // but the direction: nothing is written through the program
            // counter (`Reference/68ks1e.htm`, and the manual's "alterable"
            // group, which is the one PC-relative is not in). Said only where
            // the position reaches memory at all, so `lea label(pc),a0` is
            // told that its second operand takes An and nothing else.
            return Some(
                "a PC-relative operand is read and never written; write the label on its own"
                    .to_string(),
            );
        }
        None
    }

    /// The two Operands of `addx`, `subx`, `abcd` and `sbcd` against both of
    /// the instruction's shapes at once, which is the only way they can be
    /// judged.
    ///
    /// These four take two data registers *or* two predecrement Operands
    /// ("ADDRESS METHODS: Dn, -(An)", `Reference/68ks5e.htm` and its three
    /// neighbours), so `addx d0,-(a1)` is wrong in neither Operand on its own
    /// and `addx (a0),(a1)` is wrong in both. A per-position message would say
    /// "the second operand of `addx` cannot be a predecrement operand" of a
    /// line whose fix is to make the *first* one a predecrement too, which is
    /// the wrong sentence twice over. One Diagnostic over the pair says what
    /// was found, what the two shapes are and what was probably meant, which is
    /// ADR 0003 read literally.
    ///
    /// Answers `true` when it has said something, in which case the Operands
    /// are not judged one by one as well.
    fn check_operand_pair(&mut self, spec: &'static InstructionSpec, operands: &[Operand]) -> bool {
        let Some(advice) = the_advice_of_a_pair(spec) else {
            return false;
        };
        let [first, second] = operands else {
            // The count is already wrong, and `wrong_operand_count` has said
            // so.
            return false;
        };
        if spec.has_a_form_that_fits(operands) {
            return false;
        }
        self.raise(
            DiagnosticKind::InvalidOperandPair {
                mnemonic: spec.mnemonic.to_string(),
                found: format!("{} and {}", first.description(), second.description()),
                advice: Some(advice.to_string()),
            },
            first.span().join(second.span()),
        );
        true
    }

    /// The two pairs of Operands each of which every position allows on its
    /// own and the 68000 has no encoding for.
    ///
    /// * **An address register added into memory**, `add a0,(a1)`. `add` and
    ///   `sub` reach memory through their `dn,<ea>` form only, so the source
    ///   is then a data register, or an immediate, which is `addi`; an address
    ///   register is a source only into a register (`addfl` in EASy68K's
    ///   `INSTTABL.CPP`, which refuses the line too).
    /// * **A static bit number on an immediate**, `btst #1,#5`. The 68000 tests
    ///   an immediate with a bit number from a register only; its static form
    ///   reads no immediate destination (the Programmer's Reference Manual's
    ///   `BTST`, and the decoder's row for it), so `btst d1,#5` is the line
    ///   that exists.
    ///
    /// Each is said about the Operand to change, in the words of
    /// `invalid_addressing_mode`, with what the position takes beside the other
    /// Operand.
    fn check_encodable_pair(&mut self, spec: &'static InstructionSpec, operands: &[Operand]) {
        let [first, second] = operands else { return };
        let (Some(first_mode), Some(second_mode)) = (Modes::of(first), Modes::of(second)) else {
            return;
        };
        let (position, operand, allowed, suggestion) = match spec.family() {
            Some(Family::AddSub { .. })
                if first_mode == Modes::AN && Modes::MEMORY.contains(second_mode) =>
            {
                (
                    1,
                    first,
                    Modes::DN | Modes::IMMEDIATE,
                    format!(
                        "into memory, `{}` takes a data register or an immediate; move the \
                         address into a data register first",
                        spec.mnemonic
                    ),
                )
            }
            Some(Family::Bit(_))
                if first_mode == Modes::IMMEDIATE && second_mode == Modes::IMMEDIATE =>
            {
                (
                    2,
                    second,
                    Modes::DATA.difference(Modes::IMMEDIATE),
                    format!(
                        "a bit number written `#n` is tested in a register or in memory; \
                         `{} d1,{}` takes the bit number from a register",
                        spec.mnemonic,
                        self.text_of(second.span())
                    ),
                )
            }
            _ => return,
        };
        self.raise(
            DiagnosticKind::InvalidAddressingMode {
                mnemonic: spec.mnemonic.to_string(),
                position,
                found: operand.description().to_string(),
                allowed: allowed.names(),
                suggestion: Some(suggestion),
            },
            operand.span(),
        );
    }

    /// The rule no position can state: `add`, `sub`, `and` and `or` reach
    /// memory once.
    fn check_combination(
        &mut self,
        spec: &'static InstructionSpec,
        form: &'static Form,
        operands: &[Operand],
    ) {
        if form.combination != Combination::AtMostOneMemoryOperand {
            return;
        }
        let in_memory =
            |operand: &Operand| Modes::of(operand).is_some_and(|mode| Modes::MEMORY.contains(mode));
        match operands {
            [first, second] if in_memory(first) && in_memory(second) => self.raise(
                DiagnosticKind::BothOperandsInMemory {
                    mnemonic: spec.mnemonic.to_string(),
                },
                first.span().join(second.span()),
            ),
            _ => {}
        }
    }

    // -- the size ----------------------------------------------------------

    /// The size the instruction works at, the Form's default when the source
    /// wrote none, with the two size rules checked on the way.
    fn check_size(
        &mut self,
        spec: &'static InstructionSpec,
        form: &'static Form,
        operation: &Operation,
        operands: &[Operand],
    ) -> Option<Size> {
        if let (Some(written), Some(span)) = (operation.size, operation.size_span) {
            if !form.sizes.accepts(written) {
                // The sizes named are the **chosen Form's** and not every size
                // the Mnemonic has: `move.b d0,ccr` is answered with "`move`
                // takes `.w`", because that is the shape being judged, and the
                // union over the Forms would have offered the `.b` it has just
                // refused.
                self.raise(
                    DiagnosticKind::InvalidSize {
                        mnemonic: spec.mnemonic.to_string(),
                        size: written.suffix().to_string(),
                        allowed: form.sizes.allowed().iter().map(quoted_size).collect(),
                    },
                    span,
                );
                return form.sizes.default_size();
            }
        }
        if form.sizes == table::SizeRule::Branch {
            // A branch's suffix is the width of its displacement and not the
            // size it works at: `.b` and `.s` are the same one-byte offset
            // (`Reference/68ks9b.htm`). The lowering reads it as written to
            // choose the branch's size; it has no size to work at.
            return form.sizes.default_size();
        }
        let size = lowering::lower_size(operation.size, form.sizes.default_size());
        // Only where a byte was a *choice*: `Scc` writes a byte and `moveq`
        // writes a long, and neither is a size an author picked. `moves` is
        // the one instruction whose byte reaches an address register: the low
        // byte of one is written, and a byte read is sign extended into one.
        if size == Some(Size::Byte)
            && form.sizes == table::SizeRule::Any
            && spec.family() != Some(Family::Moves)
        {
            if let Some(operand) = operands
                .iter()
                .find(|operand| matches!(operand, Operand::AddressRegisterDirect { .. }))
            {
                self.raise(
                    DiagnosticKind::AddressRegisterByteSize {
                        mnemonic: spec.mnemonic.to_string(),
                    },
                    operand.span(),
                );
            }
        }
        size
    }

    /// The size check on its own, for a line whose Operands the analyzer will
    /// not look at: a wrong number of them, or a parser error above.
    fn check_size_alone(&mut self, spec: &'static InstructionSpec, operation: &Operation) {
        let (Some(written), Some(span)) = (operation.size, operation.size_span) else {
            return;
        };
        if spec.forms.iter().any(|form| form.sizes.accepts(written)) {
            return;
        }
        self.raise(
            DiagnosticKind::InvalidSize {
                mnemonic: spec.mnemonic.to_string(),
                size: written.suffix().to_string(),
                allowed: spec
                    .sizes()
                    .iter()
                    .map(|size| size.suffix().to_string())
                    .collect(),
            },
            span,
        );
    }

    // -- the values --------------------------------------------------------

    /// The field the instruction encodes the first Operand in, when it has one
    /// of its own: the count of a quick form, a shift count, the vector of
    /// `trap`.
    fn check_instruction_value(&mut self, spec: &'static InstructionSpec, operands: &[Operand]) {
        let Some(rule) = spec.value_rule else { return };
        let Some(Operand::Immediate { value, span, .. }) = operands.first() else {
            return;
        };
        let Some(number) = self.context.evaluate(value) else {
            return;
        };
        let max = rule.max;
        if number < rule.min || number > max {
            self.raise(
                DiagnosticKind::ValueOutOfRange {
                    subject: rule.subject.to_string(),
                    value: number,
                    min: rule.min,
                    max,
                    advice: rule.hint.map(str::to_string),
                },
                *span,
            );
        }
    }

    /// A static bit number past the destination's width: EASy68K encodes the
    /// low byte of any number (`staticBit` in its `BUILD.CPP`), and the 68000
    /// takes it modulo 32 in a data register and modulo 8 in a byte of
    /// memory, so `btst #9,(a0)` tests bit 1. It assembles as it does there,
    /// with a warning that names the bit it reaches, as it is almost always a
    /// mistake.
    fn check_bit_number(&mut self, spec: &'static InstructionSpec, operands: &[Operand]) {
        if !matches!(spec.family(), Some(Family::Bit(_))) {
            return;
        }
        let Some(Operand::Immediate { value, span, .. }) = operands.first() else {
            return;
        };
        let Some(number) = self.context.evaluate(value) else {
            return;
        };
        let width = match operands.get(1).and_then(Modes::of) {
            Some(Modes::DN) => 32,
            _ => 8,
        };
        if (0..width).contains(&number) {
            return;
        }
        self.raise(
            DiagnosticKind::BitNumberWraps {
                number,
                width: width as u8,
                bit: (number as u8 % width as u8),
            },
            *span,
        );
    }

    /// The displacement of `link`, which the 68000 encodes in a word and sign
    /// extends: EASy68K's "Invalid 16 bit data" (`link` in its `BUILD.CPP`).
    /// A frame takes a negative one, so `#$FFFC` is refused as EASy68K
    /// refuses it, and `#-4` is the way to write it.
    fn check_link_displacement(
        &mut self,
        spec: &'static InstructionSpec,
        operands: &[Operand],
        fits: &[bool],
    ) {
        if spec.family() != Some(Family::Link) || fits.get(1) != Some(&true) {
            return;
        }
        let Some(Operand::Immediate { value, .. }) = operands.get(1) else {
            return;
        };
        let Some(number) = self.context.evaluate(value) else {
            return;
        };
        if (-32768..=32767).contains(&number) {
            return;
        }
        self.raise(
            DiagnosticKind::ValueOutOfRange {
                subject: "the displacement of `link`".to_string(),
                value: number,
                min: -32768,
                max: 32767,
                advice: Some(
                    "a frame larger than 32 KB is reserved with `suba.l #n,a7` after the `link`"
                        .to_string(),
                ),
            },
            value.span(),
        );
    }

    /// How far a `bra`, `bsr`, `Bcc` or `DBcc` reaches, against the
    /// displacement its size gives it: EASy68K's "Branch instruction
    /// displacement is out of range or invalid" (`branch` and `dbcc` in its
    /// `BUILD.CPP`).
    ///
    /// The size is the one the lowering chooses ([`choice::branch_size`]): a
    /// branch written `.s` or `.b` is short whatever its distance, and one with
    /// no size is short only when its target is a back reference a byte
    /// reaches, so an unsized branch can fail only as a word. A short branch
    /// holds −128 to 127 and not 0, the byte that marks a word branch, so a
    /// short branch to the next instruction has its own sentence. A word holds
    /// −32768 to 32767, and so does the displacement of every `DBcc`. Both are
    /// measured from the word after the operation word, which is where the
    /// 68000's program counter is when it adds them. On the 68020 a short
    /// branch does not hold −1 either, the byte `$FF` that marks a long branch
    /// there, and has a sentence of its own too.
    fn check_branch_distance(
        &mut self,
        spec: &'static InstructionSpec,
        operation: &Operation,
        operands: &[Operand],
        fits: &[bool],
    ) {
        let (position, decrements) = match spec.family() {
            Some(Family::Bra | Family::Bsr | Family::Bcc(_)) => (0, false),
            Some(Family::DBcc(_)) => (1, true),
            _ => return,
        };
        if fits.get(position) != Some(&true) {
            return;
        }
        let Some(Operand::Absolute { value, .. }) = operands.get(position) else {
            return;
        };
        let Some(target) = self.context.evaluate(value) else {
            return;
        };
        let displacement =
            choice::branch_displacement(self.context.current_address as u32, target as u32);
        let size = match decrements {
            true => BranchSize::Word,
            false => choice::branch_size(
                operation.size,
                self.context.is_back_reference(value),
                displacement,
                self.context.processor,
            ),
        };
        let written = match operation.size {
            Some(size) => format!("{}.{}", operation.lowercase_name(), size.letter()),
            None => operation.lowercase_name(),
        };
        match size {
            BranchSize::Short if displacement == 0 => self.raise(
                DiagnosticKind::ShortBranchToNextInstruction {
                    branch: written,
                    mnemonic: operation.lowercase_name(),
                },
                value.span(),
            ),
            BranchSize::Short
                if displacement == -1 && choice::marks_a_long_branch(self.context.processor) =>
            {
                self.raise(
                    DiagnosticKind::ShortBranchMarksALongBranch {
                        branch: written,
                        mnemonic: operation.lowercase_name(),
                    },
                    value.span(),
                )
            }
            BranchSize::Short
                if !choice::fits_a_short_branch(displacement, self.context.processor) =>
            {
                self.raise(
                    DiagnosticKind::ValueOutOfRange {
                        subject: format!("the distance `{written}` reaches"),
                        value: displacement,
                        min: -128,
                        max: 127,
                        advice: Some(format!(
                            "write `{name}` with no size, or `{name}.w`: a word branch reaches \
                             32 KB either way",
                            name = operation.lowercase_name()
                        )),
                    },
                    value.span(),
                )
            }
            BranchSize::Word if !(-32768..=32767).contains(&displacement) => self.raise(
                DiagnosticKind::ValueOutOfRange {
                    subject: format!("the distance `{written}` reaches"),
                    value: displacement,
                    min: -32768,
                    max: 32767,
                    advice: Some(far_branch_advice(
                        spec,
                        &operation.lowercase_name(),
                        self.context.processor,
                    )),
                },
                value.span(),
            ),
            _ => {}
        }
    }

    /// The displacement of a displaced Operand, which the 68000 encodes in a
    /// word, and of an indexed one, which it encodes in a byte; the distance a
    /// PC-relative Operand reaches; and the width an absolute address is
    /// forced to.
    ///
    /// `extension_offset` is how far past the instruction a PC-relative
    /// Operand's extension word is ([`lowering::extension_word_offset`]).
    fn check_operand_values(&mut self, operand: &Operand, extension_offset: i64) {
        match operand {
            Operand::Displacement { displacement, .. } => {
                self.check_range(displacement, "the displacement of `d(An)`", -32768, 32767);
            }
            Operand::Index {
                displacement,
                index,
                ..
            } => {
                if let Some(displacement) = displacement {
                    self.check_range(displacement, "the displacement of `d(An,Xn)`", -128, 127);
                }
                self.check_index_size(index);
            }
            Operand::PcDisplacement { displacement, .. } => {
                self.check_pc_distance(displacement, extension_offset, "d(PC)", -32768, 32767);
            }
            Operand::PcIndex {
                displacement,
                index,
                ..
            } => {
                if let Some(displacement) = displacement {
                    self.check_pc_distance(displacement, extension_offset, "d(PC,Xn)", -128, 127);
                }
                self.check_index_size(index);
            }
            Operand::Absolute { value, size, .. } => self.check_address_width(value, *size),
            _ => {}
        }
    }

    /// How far a PC-relative Operand has to reach, against the field the 68000
    /// encodes that distance in.
    ///
    /// The source writes the address it wants, and the Program keeps that
    /// address; what has to fit the field is the distance from this
    /// instruction's extension word to it ([`lowering::pc_relative_offset`]),
    /// which is the displacement the 68000 encodes. So the number the range is
    /// about is one the student never wrote: the message says what it is and
    /// the hint says that a plain label has no such limit.
    fn check_pc_distance(
        &mut self,
        address: &ast::Expr,
        extension_offset: i64,
        notation: &str,
        min: i64,
        max: i64,
    ) {
        let Some(distance) = lowering::pc_relative_offset(self.context, address, extension_offset)
        else {
            return;
        };
        if distance >= min && distance <= max {
            return;
        }
        let text = self.text_of(address.span());
        self.raise(
            DiagnosticKind::ValueOutOfRange {
                subject: format!("the distance a `{notation}` operand reaches"),
                value: distance,
                min,
                max,
                advice: Some(format!(
                    "write `{text}` on its own: an absolute address reaches anywhere in memory"
                )),
            },
            address.span(),
        );
    }

    /// The width an absolute address is forced to: `label.w` and `label.l`,
    /// and nothing else (`Reference/68ks1e.htm`, "Forcing Absolute Short
    /// Addressing").
    ///
    /// The two name the same address here — s68k stores addresses and encodes
    /// no words — so `.l` says nothing and is accepted in silence, while `.w`
    /// is a claim about the address that can be false and is checked.
    fn check_address_width(&mut self, value: &ast::Expr, size: Option<SizeSuffix>) {
        match size {
            None | Some(SizeSuffix::Long) => {}
            Some(SizeSuffix::Word) => self.check_short_address(value),
            Some(size) => self.raise(
                DiagnosticKind::InvalidAddressWidth {
                    address: self.text_of(value.span()),
                    size: size.suffix().to_string(),
                },
                value.span(),
            ),
        }
    }

    /// An address forced to `.w` against the sixteen bits an absolute short
    /// reference holds, which is EASy68K's own range: "Absolute short
    /// addressing must be in the range -32768 through 32767" (`errors.htm`).
    /// The address is read in 32 bits, as EASy68K reads it and as an address
    /// is everywhere else, so `$FFFF8000.w` is −32768 and fits, which is the
    /// address the word `$8000` sign extends to.
    ///
    /// EASy68K warns that forcing the width *disables* this check and encodes
    /// the low word sign extended, so its `$8000.w` reads `$ff8000`. The
    /// Program keeps the address as written, so an address the field cannot
    /// name would quietly mean a different place: refusing it is the
    /// deviation ADR 0001 records.
    fn check_short_address(&mut self, value: &ast::Expr) {
        let Some(address) = self.context.evaluate(value) else {
            return;
        };
        let address = address as u32 as i32 as i64;
        if (-32768..=32767).contains(&address) {
            return;
        }
        let text = self.text_of(value.span());
        self.raise(
            DiagnosticKind::ValueOutOfRange {
                subject: "an address forced to `.w`".to_string(),
                value: address,
                min: -32768,
                max: 32767,
                advice: Some(format!(
                    "write `{text}.l`, or `{text}` on its own: both reach the same address here"
                )),
            },
            value.span(),
        );
    }

    /// An index register is read as a word or as a long, never as a byte.
    fn check_index_size(&mut self, index: &ast::IndexRegister) {
        let (Some(size), true) = (index.size, true) else {
            return;
        };
        if matches!(size, SizeSuffix::Word | SizeSuffix::Long) {
            return;
        }
        self.raise(
            DiagnosticKind::InvalidSize {
                mnemonic: "d(An,Xn)".to_string(),
                size: size.suffix().to_string(),
                allowed: vec!["`.w`".to_string(), "`.l`".to_string()],
            },
            index.span,
        );
    }

    /// One Expression against a range, when its value is already known.
    fn check_range(&mut self, expression: &ast::Expr, subject: &str, min: i64, max: i64) {
        let Some(value) = self.context.evaluate(expression) else {
            return;
        };
        if value >= min && value <= max {
            return;
        }
        self.raise(
            DiagnosticKind::ValueOutOfRange {
                subject: subject.to_string(),
                value,
                min,
                max,
                advice: None,
            },
            expression.span(),
        );
    }

    /// Every immediate against the size the instruction works at.
    ///
    /// Both readings fit: `move.b #-1,d0` and `move.b #$ff,d0` are the same
    /// byte, so the range runs from the lowest signed value to the highest
    /// unsigned one.
    ///
    /// The one immediate read at another size than the instruction's is the
    /// destination of a bit instruction, `btst d1,#5`, which is a byte whatever
    /// the instruction is written as; `stop`, which has no size, takes a word.
    fn check_immediate_sizes(
        &mut self,
        spec: &'static InstructionSpec,
        size: Option<Size>,
        operands: &[Operand],
        fits: &[bool],
    ) {
        for (index, operand) in operands.iter().enumerate() {
            let Operand::Immediate { value, span, .. } = operand else {
                continue;
            };
            if !fits[index] {
                continue;
            }
            let size = match (spec.family(), index) {
                // a static bit number is any number, of which the low byte is
                // encoded ([`Self::check_bit_number`])
                (Some(Family::Bit(_)), 0) => continue,
                (Some(Family::Bit(_)), 1) => Size::Byte,
                // the word `stop` loads the status register with
                (Some(Family::Stop), 0) => Size::Word,
                _ => match size {
                    Some(size) => size,
                    None => continue,
                },
            };
            let bits = size.to_bits() as u32;
            let min = -(1i64 << (bits - 1));
            let max = (1i64 << bits) - 1;
            let Some(number) = self.context.evaluate(value) else {
                continue;
            };
            if number >= min && number <= max {
                continue;
            }
            if expr::holds_a_long_character_literal(value) {
                // `#'abcdefgh'` is worth a nineteen-digit number the student
                // never wrote, and the evaluator has already said what is wrong
                // with it (`character_literal_too_long`). EASy68K warns and
                // assembles the same line, so this one does too (ADR 0001).
                continue;
            }
            self.raise(
                DiagnosticKind::ImmediateOutOfRange {
                    value: number,
                    size: size_name(size).to_string(),
                    min,
                    max,
                },
                *span,
            );
        }
    }

    /// A bare number below the program's origin where an immediate would also
    /// have been allowed: `move.l 5,d0` reads the long at address 5, which is
    /// almost never what was meant (ADR 0003, and it is a suggestion because
    /// the line is legal).
    fn suggest_immediates(&mut self, form: &'static Form, operands: &[Operand], fits: &[bool]) {
        for (index, operand) in operands.iter().enumerate() {
            let Operand::Absolute { value, span, .. } = operand else {
                continue;
            };
            if !fits[index] || !form.operands[index].contains(Modes::IMMEDIATE) {
                continue;
            }
            let ast::Expr::Number { value, .. } = value else {
                continue;
            };
            if *value < 0 || *value >= self.context.origin {
                continue;
            }
            self.raise(DiagnosticKind::BareNumberAsAddress { value: *value }, *span);
        }
    }

    /// Whether the one Operand of an Operation that takes none is a lone `*`,
    /// which is EASy68K's comment marker (`docs/grammar.md` 3.4).
    ///
    /// It is a warning and not an error: `nop  * do nothing` is an EASy68K
    /// program that has to keep assembling (ADR 0001), and the `*` is dropped.
    fn star_is_a_comment(&mut self, spec: &'static InstructionSpec, operation: &Operation) -> bool {
        if !spec.forms.iter().all(|form| form.arity() == 0) {
            return false;
        }
        let [Operand::Absolute {
            value: ast::Expr::CurrentAddress { .. },
            span,
            ..
        }] = operation.operands.as_slice()
        else {
            return false;
        };
        self.raise(
            DiagnosticKind::StarIsTheCurrentAddress {
                mnemonic: spec.mnemonic.to_string(),
            },
            *span,
        );
        true
    }

    // -- odds and ends -----------------------------------------------------

    /// Raise a Diagnostic about `span` of this line.
    ///
    /// While planning nothing is raised and nothing fails: the line is only
    /// being measured, and pass 2 judges it.
    /// A Form of a later processor than the Program's: `extb.l` or
    /// `move ccr,<ea>` in a 68000 Program.
    ///
    /// The hint has two halves, the 68000's way of doing the same and the
    /// `opt p=` line that writes the Program for the later processor, and the
    /// Diagnostic carries both as fixes, the `opt p=` line first: an editor
    /// applies one and the program assembles (the asm-editor's
    /// `docs/design/m68k-machine.md`, decision 3).
    fn needs_a_later_processor(
        &mut self,
        spec: &'static InstructionSpec,
        form: &'static Form,
        operation: &Operation,
        operands: &[Operand],
    ) {
        if self.context.planning {
            self.failed = true;
            return;
        }
        let (program, needed) = (self.context.processor, form.processor);
        let uppercase = operation.name.chars().any(|c| c.is_ascii_uppercase())
            && !operation.name.chars().any(|c| c.is_ascii_lowercase());
        let cased = |text: &str| match uppercase {
            true => text.to_ascii_uppercase(),
            false => text.to_string(),
        };
        let (name, equivalent) = match (spec.family(), operands) {
            (Some(Family::ExtByteToLong), [register]) => {
                let register = self.text_of(register.span());
                // The second line starts under the first one's Operation,
                // whatever stands before it on the line.
                let indentation: String = self.text[..operation.span.start]
                    .chars()
                    .map(|c| if c == '\t' { '\t' } else { ' ' })
                    .collect();
                let text = format!(
                    "{} {register}\n{indentation}{} {register}",
                    cased("ext.w"),
                    cased("ext.l")
                );
                let fix = SuggestedFix {
                    title: format!("Write `ext.w {register}` and `ext.l {register}`"),
                    location: self.location(operation.span),
                    text,
                };
                (
                    "extb.l".to_string(),
                    Some((
                        format!(
                            "`ext.w {register}` and then `ext.l {register}`, which do the same"
                        ),
                        fix,
                    )),
                )
            }
            (Some(Family::Move), [ccr, _]) => {
                let whole = operation.span;
                let ccr_span = ccr.span();
                let sr = match self
                    .text_of(ccr_span)
                    .chars()
                    .any(|c| c.is_ascii_uppercase())
                {
                    true => "SR",
                    false => "sr",
                };
                let rewritten = format!(
                    "{}{sr}{}",
                    &self.text[whole.start..ccr_span.start],
                    &self.text[ccr_span.end..whole.end]
                );
                let fix = SuggestedFix {
                    title: format!("Write `{rewritten}`"),
                    location: self.location(ccr_span),
                    text: sr.to_string(),
                };
                (
                    "move ccr,<ea>".to_string(),
                    Some((format!("`{rewritten}`, whose low byte is the CCR"), fix)),
                )
            }
            _ => (spec.mnemonic.to_string(), None),
        };
        let dialect_fix = self.context.processor_line.map(|line| match line {
            ProcessorLine::Chosen(location) => SuggestedFix {
                title: format!(
                    "Change `opt p={}` to `opt p={}`",
                    program.name(),
                    needed.name()
                ),
                location: location.clone(),
                text: needed.name().to_string(),
            },
            ProcessorLine::Missing { above, indentation } => SuggestedFix {
                title: format!("Add `opt p={}` at the top of the program", needed.name()),
                location: Location::new(above.file.clone(), above.line, 0, 0),
                text: format!("{indentation}opt p={}\n", needed.name()),
            },
        });
        let chosen = matches!(self.context.processor_line, Some(ProcessorLine::Chosen(_)));
        let (equivalent, equivalent_fix) = match equivalent {
            Some((text, fix)) => (Some(text), Some(fix)),
            None => (None, None),
        };
        let mut diagnostic = Diagnostic::new(
            DiagnosticKind::NeedsALaterProcessor {
                name,
                processor: needed,
                program,
                equivalent,
                chosen,
            },
            self.location(operation.span),
        );
        for fix in dialect_fix.into_iter().chain(equivalent_fix) {
            diagnostic = diagnostic.with_fix(fix);
        }
        self.failed = true;
        self.diagnostics.push(diagnostic);
    }

    fn raise(&mut self, kind: DiagnosticKind, span: Span) {
        if self.context.planning {
            return;
        }
        let diagnostic = Diagnostic::new(kind, self.location(span));
        if diagnostic.is_error() {
            self.failed = true;
        }
        self.diagnostics.push(diagnostic);
    }

    /// The Location of `span` on this line.
    fn location(&self, span: Span) -> Location {
        Location::from_span(self.file, self.line_index, self.text, span)
    }

    /// The text a span covers, which is how a message quotes what was written.
    fn text_of(&self, span: Span) -> String {
        span.text(self.text).to_string()
    }
}

/// What to write instead of a branch whose target is more than a word away,
/// `mnemonic` as it was written: the jump that reaches anywhere in memory, for
/// each kind of branch.
fn far_branch_advice(
    spec: &'static InstructionSpec,
    mnemonic: &str,
    processor: Processor,
) -> String {
    match spec.family() {
        Some(Family::Bra | Family::Bsr | Family::Bcc(_)) if processor.has(Processor::M68020) => {
            format!("write `{mnemonic}.l`: a long branch reaches anywhere in memory")
        }
        Some(Family::Bsr) => "`jsr` reaches anywhere in memory".to_string(),
        Some(Family::Bcc(_)) => "branch on the opposite condition over a `jmp`, which reaches \
                                 anywhere in memory"
            .to_string(),
        Some(Family::DBcc(_)) => format!(
            "`{mnemonic}` has no longer form: bring the loop within 32 KB, or count with `subq` \
             and a branch over a `jmp`"
        ),
        _ => "`jmp` reaches anywhere in memory".to_string(),
    }
}

/// What to write instead of a badly shaped `addx`, `subx`, `abcd` or `sbcd`,
/// and `None` for every other Mnemonic.
///
/// It is what tells the four apart from the rest of the table — an instruction
/// whose two Forms are two whole shapes — as well as being the last clause of
/// the message.
fn the_advice_of_a_pair(spec: &'static InstructionSpec) -> Option<&'static str> {
    match spec.family()? {
        Family::AddSubExtended { subtract: false } => Some("`add` takes every addressing mode"),
        Family::AddSubExtended { subtract: true } => Some("`sub` takes every addressing mode"),
        // `abcd` and `sbcd` have no counterpart that reaches memory: decimal
        // arithmetic is these two and `nbcd`, so the advice is how to get the
        // byte where they can see it.
        Family::AddSubDecimal { .. } => Some("move the byte into a data register first"),
        _ => None,
    }
}

/// Whether any Form of the instruction names `sr` or `ccr` anywhere, which is
/// what tells "this Mnemonic never reaches the status register" from "it does,
/// but not like that".
fn takes_the_status_register(spec: &'static InstructionSpec) -> bool {
    spec.forms.iter().any(|form| {
        form.operands
            .iter()
            .any(|modes| modes.intersects(Modes::STATUS))
    })
}

/// Whether a Form names `sr`, `ccr` or `usp` in exactly the positions the
/// Operands wrote one, which is what [`Analyzer::choose_form`] narrows the
/// candidates by.
///
/// A position that takes a special register takes nothing else, so "the
/// Form's modes at this position mention one" and "this Operand is one"
/// answer the same question from the two sides. An Operand that is none of
/// them, an ordinary Addressing mode, agrees with a position that names none,
/// which is every position of every Form but the six of `move`, the two each
/// of `andi`, `ori` and `eori`, and the control register of `movec`, which
/// takes `usp` or any of the others.
fn agrees_about_the_special_registers(form: &Form, operands: &[Operand]) -> bool {
    operands.iter().enumerate().all(|(index, operand)| {
        let allowed = form.operands[index].intersection(Modes::SPECIAL);
        match Modes::of(operand).map(|mode| mode.intersection(Modes::SPECIAL)) {
            Some(written) if !written.is_empty() => allowed.contains(written),
            _ => allowed.is_empty(),
        }
    })
}

/// The first word of a Comment field, and where it was written, when that word
/// is the whole field.
///
/// "The whole field" allows an explicit Comment after it — `move.l d0 d1 ; copy`
/// is the same mistake with a comment beside it — and nothing else, so a
/// sentence of prose that opens with a register's name says nothing.
fn first_word(comment: &ast::Comment) -> Option<(&str, Span)> {
    let text = &comment.text;
    let leading = text.len() - text.trim_start().len();
    let rest = text.trim_start();
    let word = rest.split_whitespace().next()?;
    let after = rest[word.len()..].trim_start();
    if !after.is_empty() && !after.starts_with(';') && !after.starts_with('*') {
        return None;
    }
    let start = comment.span.start + leading;
    Some((word, Span::new(start, start + word.len())))
}

/// A size as a hint writes it, backticks included: `` `.w` ``.
fn quoted_size(size: &SizeSuffix) -> String {
    format!("`{}`", size.suffix())
}

/// The name a message gives a size ("byte").
fn size_name(size: Size) -> &'static str {
    match size {
        Size::Byte => "byte",
        Size::Word => "word",
        Size::Long => "long",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::parser;
    use std::collections::HashMap;

    /// A symbol table written out, for the checks that need a value.
    struct Symbols(HashMap<String, i64>);

    impl SymbolValues for Symbols {
        fn value_of(&self, name: &str) -> Option<i64> {
            self.0.get(name).copied()
        }
    }

    /// Parse and analyze one line with no Symbols, and give back the codes it
    /// raised and the instruction it built.
    fn analyze(text: &str) -> (Vec<String>, Option<Instruction>) {
        analyze_with(text, Symbols(HashMap::new()))
    }

    fn analyze_with(text: &str, symbols: Symbols) -> (Vec<String>, Option<Instruction>) {
        let (line, parser_diagnostics) = parser::parse_line(text, "main.m68k", 0);
        let context = Context::new(&symbols);
        let mut analyzer = Analyzer::new("main.m68k", 0, text, &context);
        let instruction =
            analyzer.analyze_line(&line, parser_diagnostics.iter().any(|d| d.is_error()));
        let codes = analyzer
            .finish()
            .iter()
            .map(|diagnostic| diagnostic.code().to_string())
            .collect();
        (codes, instruction)
    }

    /// The codes one line raises.
    fn codes(text: &str) -> Vec<String> {
        analyze(text).0
    }

    /// The codes one line raises against a symbol table.
    fn codes_with(text: &str, symbols: Symbols) -> Vec<String> {
        analyze_with(text, symbols).0
    }

    /// A symbol table of name and value pairs.
    fn symbols<const N: usize>(values: [(&str, i64); N]) -> Symbols {
        Symbols(
            values
                .iter()
                .map(|(name, value)| ((*name).to_string(), *value))
                .collect(),
        )
    }

    /// The messages one line raises, hint included, for reading them as a
    /// student would.
    fn messages(text: &str) -> Vec<String> {
        messages_with(text, Symbols(HashMap::new()))
    }

    fn messages_with(text: &str, symbols: Symbols) -> Vec<String> {
        let (line, parser_diagnostics) = parser::parse_line(text, "main.m68k", 0);
        let context = Context::new(&symbols);
        let mut analyzer = Analyzer::new("main.m68k", 0, text, &context);
        analyzer.analyze_line(&line, parser_diagnostics.iter().any(|d| d.is_error()));
        analyzer
            .finish()
            .iter()
            .map(|diagnostic| match diagnostic.hint() {
                Some(hint) => format!("{} {hint}", diagnostic.message()),
                None => diagnostic.message(),
            })
            .collect()
    }

    /// `.b` on a branch is `.s`, which is what the reference does.
    ///
    /// "EASy68K will accept .B or .S to force 1-byte offsets and .W or .L to
    /// force 2-byte offsets" (`Reference/68ks9b.htm`, and the same sentence on
    /// `BRA`), so an EASy68K program that writes `BRA.B` assembles here
    /// unchanged (ADR 0001). None of the four suffixes reaches the encoded
    /// instruction, because a branch carries no operand size.
    #[test]
    fn branch_takes_b_as_it_takes_s() {
        for (with_b, with_s) in [
            ("    bra.b $1000", "    bra.s $1000"),
            ("    bsr.b $1000", "    bsr.s $1000"),
            ("    beq.b skip", "    beq.s skip"),
        ] {
            let (codes, byte) = analyze(with_b);
            assert!(codes.is_empty(), "{with_b} raised {codes:?}");
            assert_eq!(
                format!("{byte:?}"),
                format!("{:?}", analyze(with_s).1),
                "{with_b} and {with_s}"
            );
        }
    }

    #[test]
    fn a_well_formed_instruction_raises_nothing_and_is_lowered() {
        for text in [
            "    move.l #10,d0",
            "    move.b (a0)+,(a1)+",
            "    movem.l d0-d2/a0,-(a7)",
            "    movem.l (a7)+,d0-d2/a0",
            "    lea 4(a6,d1.w),a0",
            "    asl (a0)",
            "    asl.l #3,d0",
            "    asr.w (a0)",
            "    dbra d0,$1000",
            "    bra.s $1000",
            "    bra.b $1000",
            "    bsr.b $1000",
            "    beq.b $1000",
            "    trap #15",
            "    btst #24,d1",
            "    ext.l d0",
            "    moveq.l #1,d0",
            "    lea.l (a0),a1",
            "    btst.l #24,d1",
            "    swap.w d0",
            "    seq.b d0",
            "    cmp.l (a0)+,(a1)+",
            "    nop",
            "    rts",
        ] {
            let (codes, instruction) = analyze(text);
            assert!(codes.is_empty(), "{text} raised {codes:?}");
            assert!(instruction.is_some(), "{text} was not lowered");
        }
    }

    #[test]
    fn an_unknown_mnemonic_offers_the_closest_name_and_the_label_hint() {
        assert_eq!(codes("    mvoe.l d0,d1"), ["unknown_mnemonic"]);
        assert_eq!(
            messages("    frobnicate d0"),
            [
                "`frobnicate` is not an instruction or a directive. Start it in column 1, \
                 or end it with a colon, if `frobnicate` is a label"
            ]
        );
        assert_eq!(
            messages("    mvoe d0,d1"),
            [
                "`mvoe` is not an instruction or a directive. Did you mean `move`? start it \
                 in column 1, or end it with a colon, if `mvoe` is a label"
            ]
        );
    }

    #[test]
    fn a_mnemonic_in_column_one_is_offered_a_colon_when_the_line_is_wrong() {
        assert_eq!(
            codes("clr"),
            ["wrong_operand_count", "mnemonic_used_as_label"]
        );
        assert_eq!(
            codes("clr d0"),
            Vec::<String>::new(),
            "a correct instruction in column 1 is an instruction"
        );
        assert_eq!(
            codes("    clr"),
            ["wrong_operand_count"],
            "an indented one was never a label"
        );
    }

    #[test]
    fn an_instruction_that_is_not_assembled_says_why() {
        assert_eq!(codes("    callm #0,(a0)"), ["unimplemented_operation"]);
        assert_eq!(codes("    rtm d0"), ["unimplemented_operation"]);
        assert_eq!(codes("    trap #16"), ["value_out_of_range"]);
    }

    /// `stop` and `reset` are assembled since interrupts are taken (the
    /// plan's milestone C8); `stop`'s immediate is the word it loads the
    /// status register with, a word immediate's range.
    #[test]
    fn stop_and_reset_assemble() {
        assert!(matches!(analyze("    stop #$2700"), (codes, Some(Instruction::STOP(0x2700))) if codes.is_empty()));
        assert!(matches!(analyze("    stop #-1"), (codes, Some(Instruction::STOP(0xFFFF))) if codes.is_empty()));
        assert!(matches!(analyze("    reset"), (codes, Some(Instruction::RESET)) if codes.is_empty()));
        assert_eq!(codes("    stop #$12345"), ["immediate_out_of_range"]);
        assert_eq!(codes("    stop d0"), ["invalid_addressing_mode"]);
        assert_eq!(codes("    stop"), ["wrong_operand_count"]);
        assert_eq!(codes("    reset #1"), ["wrong_operand_count"]);
    }

    /// `rte` and every `trap` are assembled since exceptions are taken (the
    /// plan's milestone C6).
    #[test]
    fn rte_and_every_trap_assemble() {
        assert!(matches!(analyze("    rte"), (codes, Some(Instruction::RTE)) if codes.is_empty()));
        for vector in 0..=15 {
            let line = format!("    trap #{vector}");
            assert!(
                matches!(analyze(&line), (codes, Some(Instruction::TRAP(n))) if codes.is_empty() && n == vector),
                "{line}"
            );
        }
    }

    /// The extend-flag and binary-coded-decimal group: two shapes, judged
    /// together, and the sizes of the reference.
    #[test]
    fn the_extend_flag_group_is_judged_against_both_of_its_shapes() {
        assert!(codes("    addx.l d0,d1").is_empty());
        assert!(codes("    addx.b -(a0),-(a1)").is_empty());
        assert!(codes("    subx.w d0,d1").is_empty());
        assert!(codes("    abcd d0,d1").is_empty());
        assert!(codes("    sbcd -(a0),-(a1)").is_empty());
        assert!(codes("    negx.l (a0)").is_empty());
        assert!(codes("    nbcd d0").is_empty());
        assert!(codes("    roxl.l #3,d0").is_empty());
        assert!(codes("    roxr d1,d0").is_empty());
        assert!(codes("    roxl (a0)").is_empty());
        // One message about the pair, and never one about a position: the fix
        // for `addx d0,-(a1)` is in the operand the position message would not
        // have named.
        assert_eq!(
            messages("    addx.l #1,d0"),
            [
                "`addx` takes two data registers or two predecrement operands, and this line has \
                 an immediate and a data register. Write `addx d0,d1` or `addx -(a0),-(a1)`; \
                 `add` takes every addressing mode"
            ]
        );
        assert_eq!(
            messages("    addx.l d0,-(a1)"),
            [
                "`addx` takes two data registers or two predecrement operands, and this line has \
                 a data register and a predecrement operand. Write `addx d0,d1` or \
                 `addx -(a0),-(a1)`; `add` takes every addressing mode"
            ]
        );
        assert_eq!(
            codes("    addx.l (a0),(a1)"),
            ["invalid_operand_pair"],
            "two wrong operands are still one mistake"
        );
        assert_eq!(
            messages("    abcd (a0),d1"),
            [
                "`abcd` takes two data registers or two predecrement operands, and this line has \
                 an indirect operand and a data register. Write `abcd d0,d1` or \
                 `abcd -(a0),-(a1)`; move the byte into a data register first"
            ]
        );
        // The three decimal instructions are a byte and nothing else, and the
        // memory form of a rotate is a word, as every other shift's is.
        assert_eq!(
            messages("    abcd.w d0,d1"),
            ["`.w` is not a size for `abcd`. `abcd` takes `.b`"]
        );
        assert_eq!(
            messages("    nbcd.l d0"),
            ["`.l` is not a size for `nbcd`. `nbcd` takes `.b`"]
        );
        assert_eq!(
            messages("    roxl.b (a0)"),
            ["`.b` is not a size for `roxl`. `roxl` takes `.w`"]
        );
        // `negx` and `nbcd` write one data alterable operand, so an address
        // register is answered the way `clr`'s and `neg`'s is.
        assert_eq!(
            codes("    negx.l a0"),
            ["invalid_addressing_mode"],
            "`negx` is judged position by position, as `neg` is"
        );
        assert_eq!(
            codes("    roxl.l #9,d0"),
            ["value_out_of_range"],
            "a written count is 1 to 8, as it is for every other shift"
        );
    }

    /// `usp` is `move`'s, to and from an address register, a long, and
    /// nothing else's (`Reference/68ks4f.htm`).
    #[test]
    fn the_user_stack_pointer_moves_to_and_from_an_address_register() {
        for line in ["    move.l usp,a0", "    move a0,usp", "    move.l sp,usp"] {
            let (codes, instruction) = analyze(line);
            assert!(codes.is_empty(), "{line} raised {codes:?}");
            assert!(
                matches!(instruction, Some(Instruction::MOVEUSP { .. })),
                "{line} is `move usp`"
            );
        }
        assert_eq!(codes("    move.l d0,usp"), ["invalid_addressing_mode"]);
        assert_eq!(codes("    move.w a0,usp"), ["invalid_size"]);
        assert_eq!(
            messages("    add.l usp,d0"),
            [
                "The first operand of `add` cannot be the user stack pointer. The operand \
                 should be Dn, An, (An), (An)+, -(An), d(An), d(An,Xn), Ea/<label>, d(PC), \
                 d(PC,Xn) or Im. Only `move` reaches the user stack pointer, to or from an \
                 address register, and the 68010's `movec`"
            ]
        );
    }

    /// The PC-relative modes, which the reference puts in data, memory and
    /// control and in no group anything is written to
    /// (`Reference/68ks1e.htm`).
    #[test]
    fn a_pc_relative_operand_is_read_where_the_reference_allows_one() {
        for line in [
            "    move.l data(pc),d0",
            "    move.l (data,pc),d0",
            "    add.w data(pc),d1",
            "    cmp.l data(pc,d1.w),d2",
            "    lea data(pc),a0",
            "    pea data(pc)",
            "    jmp data(pc)",
            "    jsr data(pc,a1.l)",
            "    movem.l data(pc),d0-d2",
            "    btst #3,data(pc)",
            "    chk.w data(pc),d0",
            "    move.w (pc,d1.w),d0",
        ] {
            assert_eq!(
                analyze_with(line, symbols([("data", 0x1010)])).0,
                Vec::<String>::new(),
                "{line}"
            );
        }
        // Nothing is written through the program counter, and the sentence
        // says which half of the line is the mistake.
        assert_eq!(
            messages_with("    move.l d0,data(pc)", symbols([("data", 0x1010)])),
            [
                "The second operand of `move` cannot be a PC-relative operand. The operand \
                 should be Dn, An, (An), (An)+, -(An), d(An), d(An,Xn) or Ea/<label>. A \
                 PC-relative operand is read and never written; write the label on its own"
            ]
        );
        for line in [
            "    clr.l data(pc)",
            "    asl.w data(pc)",
            "    movem.l d0-d2,data(pc)",
            "    tst.b data(pc)",
        ] {
            assert_eq!(
                analyze_with(line, symbols([("data", 0x1010)])).0,
                ["invalid_addressing_mode"],
                "{line}"
            );
        }
        // A position that takes no place in memory is told what it takes and
        // not lectured about the program counter.
        assert_eq!(
            messages_with("    lea (a0),data(pc)", symbols([("data", 0x1010)])),
            ["The second operand of `lea` cannot be a PC-relative operand. The operand should be An"]
        );
    }

    /// The Operand is written as the address it reaches and stored as the
    /// distance to it, so the range is about a number the student never wrote
    /// and the message says what it is.
    #[test]
    fn a_pc_relative_operand_reaches_as_far_as_its_field() {
        // The line is laid out at the default origin, `$1000`, so its
        // extension word is at `$1002`: `$9001` is 32767 bytes away and fits,
        // and one byte further does not.
        assert!(
            analyze_with("    move.l far(pc),d0", symbols([("far", 0x9001)]))
                .0
                .is_empty()
        );
        assert_eq!(
            messages_with("    move.l far(pc),d0", symbols([("far", 0x9002)])),
            [
                "The distance a `d(PC)` operand reaches is -32768 to 32767, and `32768` is \
                 outside it. Write `far` on its own: an absolute address reaches anywhere in \
                 memory"
            ]
        );
        // The indexed form holds a byte, and the same address is far too far
        // for it.
        assert!(
            analyze_with("    move.l near(pc,d1.w),d0", symbols([("near", 0x1080)]))
                .0
                .is_empty()
        );
        assert_eq!(
            messages_with("    move.l far(pc,d1.w),d0", symbols([("far", 0x2000)])),
            [
                "The distance a `d(PC,Xn)` operand reaches is -128 to 127, and `4094` is outside \
                 it. Write `far` on its own: an absolute address reaches anywhere in memory"
            ]
        );
    }

    /// A branch reaches as far as the displacement its size gives it,
    /// measured from the word after the operation word: the line is at
    /// `$1000`, so that word is at `$1002`.
    #[test]
    fn a_branch_reaches_as_far_as_its_size() {
        // Written `.s`, a byte: 127 forwards is the most.
        assert!(analyze_with("    bra.s far", symbols([("far", 0x1081)])).0.is_empty());
        assert_eq!(
            messages_with("    bra.s far", symbols([("far", 0x1082)])),
            [
                "The distance `bra.s` reaches is -128 to 127, and `128` is outside it. Write \
                 `bra` with no size, or `bra.w`: a word branch reaches 32 KB either way"
            ]
        );
        assert_eq!(
            codes_with("    bsr.b back", symbols([("back", 0x0F81)])),
            ["value_out_of_range"],
            "`.b` is `.s`, and -129 is one too far back"
        );
        // A displacement of 0 is the mark of a word branch.
        assert_eq!(
            codes_with("    beq.s next", symbols([("next", 0x1002)])),
            ["short_branch_to_next_instruction"]
        );
        // With no size the branch takes a word wherever a byte cannot reach,
        // and a word reaches 32767 bytes.
        assert!(analyze_with("    bra far", symbols([("far", 0x9001)])).0.is_empty());
        assert!(analyze_with("    bra next", symbols([("next", 0x1002)])).0.is_empty());
        assert_eq!(
            messages_with("    bsr far", symbols([("far", 0x9002)])),
            [
                "The distance `bsr` reaches is -32768 to 32767, and `32768` is outside it. \
                 `jsr` reaches anywhere in memory"
            ]
        );
        // `dbra` has a word and nothing else.
        assert!(
            analyze_with("    dbra d0,back", symbols([("back", 0x1002 - 0x8000)]))
                .0
                .is_empty()
        );
        assert_eq!(
            codes_with("    dbra d0,back", symbols([("back", 0x1001 - 0x8000)])),
            ["value_out_of_range"]
        );
    }

    /// The width an absolute address is forced to (`Reference/68ks1e.htm`,
    /// "Forcing Absolute Short Addressing").
    #[test]
    fn an_address_is_forced_to_a_width_that_can_name_it() {
        assert!(codes("    move.l $1000.l,d0").is_empty());
        assert!(codes("    move.l $1000.w,d0").is_empty());
        assert!(
            codes("    move.l $18000.l,d0").is_empty(),
            "`.l` says nothing here"
        );
        assert_eq!(
            messages("    move.l $18000.w,d0"),
            [
                "An address forced to `.w` is -32768 to 32767, and `98304` is outside it. Write \
                 `$18000.l`, or `$18000` on its own: both reach the same address here"
            ]
        );
        assert_eq!(
            messages("    move.l table.b,d0"),
            [
                "`.b` after `table` forces the width of the address, and an address is forced to \
                 `.w` or `.l`. Write `table.w` or `table.l`, or `table` on its own; the size the \
                 instruction works at goes after the mnemonic"
            ]
        );
        assert_eq!(
            codes("    bra done.s"),
            ["invalid_address_width"],
            "a branch's own size goes on the mnemonic, `bra.s done`"
        );
    }

    /// The status register and the condition codes, in every shape the table
    /// takes them in.
    #[test]
    fn the_status_register_and_the_condition_codes_are_assembled() {
        for line in [
            "    move.w d0,ccr",
            "    move.w #$1f,ccr",
            "    move.w (a0),ccr",
            "    move.w d0,sr",
            "    move.w #$2700,sr",
            "    move.w sr,d0",
            "    move.w sr,(a0)",
            "    move d0,ccr",
            "    andi.b #$1f,ccr",
            "    ori.b #$1,ccr",
            "    eori.b #$4,ccr",
            "    andi.w #$00,sr",
            "    ori.w #$700,sr",
            "    eori.w #$2000,sr",
        ] {
            assert_eq!(codes(line), Vec::<String>::new(), "{line}");
        }
    }

    /// A `move` that names a half of the status register is judged against the
    /// Form that names the same half, so the message is about the operand that
    /// is wrong and not about the one that is right.
    #[test]
    fn a_status_register_operand_chooses_the_form_that_names_it() {
        assert_eq!(
            messages("    move.w a0,sr"),
            [
                "The first operand of `move` cannot be an address register. The operand \
                 should be Dn, (An), (An)+, -(An), d(An), d(An,Xn), Ea/<label>, d(PC), d(PC,Xn) \
                 or Im. An address register holds an address; move it into a data register first"
            ]
        );
        assert_eq!(
            messages("    move.w sr,#5"),
            [
                "The second operand of `move` cannot be an immediate. The operand should be \
                 Dn, (An), (An)+, -(An), d(An), d(An,Xn) or Ea/<label>. An immediate is a value, \
                 and nothing can be written to it"
            ]
        );
        assert_eq!(
            messages("    move.b d0,ccr"),
            ["`.b` is not a size for `move`. `move` takes `.w`"],
            "the sizes named are the chosen form's, not every size `move` has"
        );
        assert_eq!(
            codes("    andi.w #$1f,ccr"),
            ["invalid_size"],
            "`andi` to the condition codes is a byte"
        );
        assert_eq!(
            codes("    andi.b #$1f,sr"),
            ["invalid_size"],
            "`andi` to the status register is a word"
        );
        assert_eq!(
            codes("    addi.w #1,sr"),
            ["invalid_addressing_mode"],
            "only `andi`, `ori` and `eori` reach the status register"
        );
        assert_eq!(
            codes("    move.w ccr,ccr"),
            ["invalid_addressing_mode", "invalid_addressing_mode"],
            "a `move` that fits no form of its own is judged against the general one,              operand by operand"
        );
        assert!(
            !messages("    move.w ccr,ccr")[0].contains("only `move`"),
            "and is not told that `move` reaches the status register, which it does"
        );
        assert_eq!(
            messages("    tst.w ccr"),
            [
                "The first operand of `tst` cannot be the condition codes. The operand should \
                 be Dn, (An), (An)+, -(An), d(An), d(An,Xn) or Ea/<label>. Only `move`, `andi`, \
                 `ori` and `eori` reach the status register"
            ]
        );
    }

    /// `movep` reaches memory through a displacement, and through `(a1)`,
    /// which EASy68K takes as `0(a1)`; nothing else.
    #[test]
    fn movep_takes_a_displacement_and_says_so() {
        assert_eq!(codes("    movep.w d0,4(a1)"), Vec::<String>::new());
        assert_eq!(codes("    movep.l 4(a1),d0"), Vec::<String>::new());
        assert_eq!(codes("    movep.w d0,(a1)"), Vec::<String>::new());
        assert_eq!(codes("    movep.l (a1),d0"), Vec::<String>::new());
        assert_eq!(
            messages("    movep.w d0,(a1)+"),
            [
                "The second operand of `movep` cannot be a postincrement operand. The operand \
                 should be (An) or d(An)"
            ]
        );
        assert_eq!(
            codes("    movep.b d0,4(a1)"),
            ["invalid_size"],
            "`movep` moves a word or a long"
        );
    }

    #[test]
    fn an_operand_is_judged_against_its_position() {
        assert_eq!(
            messages("    clr a0"),
            [
                "The first operand of `clr` cannot be an address register. The operand \
                 should be Dn, (An), (An)+, -(An), d(An), d(An,Xn) or Ea/<label>. `suba.l a0,a0` \
                 clears an address register"
            ]
        );
        assert_eq!(
            messages("    addi a0,d0"),
            [
                "The first operand of `addi` cannot be an address register. The operand \
                 should be Im"
            ]
        );
        assert_eq!(codes("    move.l d0,#5"), ["invalid_addressing_mode"]);
        assert_eq!(codes("    divu a0,d0"), ["invalid_addressing_mode"]);
        assert_eq!(codes("    jmp (a0)+"), ["invalid_addressing_mode"]);
        assert_eq!(codes("    eor.l (a0),d0"), ["invalid_addressing_mode"]);
        assert_eq!(codes("    lea d0,a0"), ["invalid_addressing_mode"]);
    }

    #[test]
    fn movem_takes_a_register_list_on_one_side_and_memory_on_the_other() {
        assert!(codes("    movem.l d0-d2,-(a7)").is_empty());
        assert!(codes("    movem.w (a7)+,d0-d2").is_empty());
        assert_eq!(codes("    movem.l d0-d2,d3"), ["invalid_addressing_mode"]);
        assert_eq!(
            codes("    movem.l d0-d2,(a7)+"),
            ["invalid_addressing_mode"],
            "registers go out through -(An) and come back through (An)+"
        );
        assert_eq!(codes("    movem.b d0-d2,-(a7)"), ["invalid_size"]);
    }

    #[test]
    fn a_size_is_judged_against_the_form_that_matched() {
        assert_eq!(codes("    movea.b d0,a0"), ["invalid_size"]);
        assert_eq!(codes("    lea.b (a0),a1"), ["invalid_size"]);
        assert_eq!(codes("    asl.l (a0)"), ["invalid_size"]);
        assert!(
            codes("    asl.l #1,d0").is_empty(),
            "the register form takes any size"
        );
        assert_eq!(
            analyze_for("    extb.w d0", Processor::M68020, None).0,
            ["invalid_size"]
        );
    }

    /// One line judged for a Program written for `processor`, with the line
    /// that chooses it or would, and every Diagnostic it raised.
    fn analyze_for(
        text: &str,
        processor: Processor,
        processor_line: Option<&ProcessorLine>,
    ) -> (Vec<String>, Option<Instruction>, Vec<Diagnostic>) {
        let (line, parser_diagnostics) = parser::parse_line(text, "main.m68k", 0);
        let symbols = Symbols(HashMap::new());
        let mut context = Context::new(&symbols);
        context.processor = processor;
        context.processor_line = processor_line;
        let mut analyzer = Analyzer::new("main.m68k", 0, text, &context);
        let instruction =
            analyzer.analyze_line(&line, parser_diagnostics.iter().any(|d| d.is_error()));
        let diagnostics = analyzer.finish();
        let codes = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code().to_string())
            .collect();
        (codes, instruction, diagnostics)
    }

    /// The first members of the dialects assemble in a Program written for
    /// their processor or a later one, and nowhere else (ADR 0011).
    #[test]
    fn a_form_assembles_for_its_processor_and_the_later_ones() {
        for (text, first) in [
            ("    move.w ccr,d0", Processor::M68010),
            ("    move ccr,(a0)+", Processor::M68010),
            ("    extb.l d0", Processor::M68020),
            ("    extb d7", Processor::M68020),
        ] {
            for processor in Processor::ALL {
                let (codes, instruction, _) = analyze_for(text, processor, None);
                match processor.has(first) {
                    true => {
                        assert!(codes.is_empty(), "{text} for the {processor:?}: {codes:?}");
                        assert!(instruction.is_some(), "{text} is lowered");
                    }
                    false => {
                        assert_eq!(codes, ["needs_a_later_processor"], "{text}, {processor:?}");
                        assert!(instruction.is_none());
                    }
                }
            }
        }
    }

    /// The refusal says what the 68000 writes instead and which `opt p=` line
    /// writes the Program for the later processor, and carries both as fixes,
    /// the `opt p=` one first.
    #[test]
    fn a_refused_form_carries_its_two_fixes() {
        let missing = ProcessorLine::Missing {
            above: Location::new("main.m68k", 2, 0, 0),
            indentation: "\t".to_string(),
        };
        let (_, _, found) = analyze_for(
            "loop:  extb.l d3 ; widen",
            Processor::M68000,
            Some(&missing),
        );
        let diagnostic = &found[0];
        assert_eq!(
            diagnostic.message(),
            "`extb.l` is a 68020 instruction, and this program is written for the 68000."
        );
        assert_eq!(
            diagnostic.hint().unwrap(),
            "On the 68000, write `ext.w d3` and then `ext.l d3`, which do the same; or add \
             `opt p=68020` at the top of the program to write it for the 68020"
        );
        let fixes: Vec<(&str, &Location, &str)> = diagnostic
            .fixes
            .iter()
            .map(|fix| (fix.title.as_str(), &fix.location, fix.text.as_str()))
            .collect();
        assert_eq!(
            fixes,
            [
                (
                    "Add `opt p=68020` at the top of the program",
                    &Location::new("main.m68k", 2, 0, 0),
                    "\topt p=68020\n"
                ),
                (
                    "Write `ext.w d3` and `ext.l d3`",
                    &Location::new("main.m68k", 0, 7, 16),
                    "ext.w d3\n       ext.l d3"
                ),
            ]
        );

        let chosen = ProcessorLine::Chosen(Location::new("main.m68k", 0, 10, 15));
        let (_, _, found) = analyze_for("    MOVE.W CCR,-(A7)", Processor::M68000, Some(&chosen));
        let diagnostic = &found[0];
        assert_eq!(
            diagnostic.message(),
            "`move ccr,<ea>` is a 68010 instruction, and this program is written for the 68000."
        );
        assert_eq!(
            diagnostic.hint().unwrap(),
            "On the 68000, write `MOVE.W SR,-(A7)`, whose low byte is the CCR; or change \
             `opt p=68000` to `opt p=68010`"
        );
        assert_eq!(
            diagnostic.fixes[0].title,
            "Change `opt p=68000` to `opt p=68010`"
        );
        assert_eq!(
            diagnostic.fixes[0].location,
            Location::new("main.m68k", 0, 10, 15)
        );
        assert_eq!(diagnostic.fixes[0].text, "68010");
        assert_eq!(diagnostic.fixes[1].title, "Write `MOVE.W SR,-(A7)`");
        assert_eq!(
            diagnostic.fixes[1].location,
            Location::new("main.m68k", 0, 11, 14)
        );
        assert_eq!(diagnostic.fixes[1].text, "SR");

        // An upper-case line is mended in upper case, and a line judged with
        // no Program around it has no `opt p=` line to edit: the hint still
        // says to add one, and the only fix is the 68000's.
        let (_, _, found) = analyze_for("    EXTB.L D0", Processor::M68010, Some(&missing));
        assert_eq!(found[0].fixes[1].text, "EXT.W D0\n    EXT.L D0");
        let (_, _, found) = analyze_for("    extb.l d0", Processor::M68000, None);
        assert_eq!(found[0].fixes.len(), 1);
        assert_eq!(found[0].fixes[0].title, "Write `ext.w d0` and `ext.l d0`");
        assert!(found[0].hint().unwrap().contains("add `opt p=68020`"));
    }

    /// The other instructions of the 68020, and the shapes it gives 68000
    /// Mnemonics, are still to come, and say so. The 68010's are all in.
    #[test]
    fn an_instruction_still_to_come_says_so() {
        for (text, processor) in [
            ("    cas.l d0,d1,(a0)", Processor::M68000),
            ("    cas.l d0,d1,(a0)", Processor::M68020),
            ("    bfextu d0{4:8},d1", Processor::M68000),
            ("    muls.l d0,d1", Processor::M68020),
            ("    link.l a6,#-8", Processor::M68020),
            ("    tst.l a0", Processor::M68020),
        ] {
            assert_eq!(
                analyze_for(text, processor, None).0,
                ["unimplemented_in_dialect"],
                "{text}, {processor:?}"
            );
        }
        // On an earlier processor a later shape is judged as the 68000
        // judges it.
        assert_eq!(
            analyze_for("    muls.l d0,d1", Processor::M68000, None).0,
            ["invalid_size"]
        );
        assert_eq!(
            analyze_for("    muls.l d0,d1", Processor::M68010, None).0,
            ["invalid_size"]
        );
        assert_eq!(
            analyze_for("    tst.l a0", Processor::M68000, None).0,
            ["invalid_addressing_mode"]
        );
        for (text, processor) in [
            ("    rtd #4", Processor::M68000),
            ("    movec vbr,d0", Processor::M68000),
        ] {
            assert_eq!(
                analyze_for(text, processor, None).0,
                ["needs_a_later_processor"],
                "{text}, {processor:?}"
            );
        }
        for (text, processor) in [
            ("    rtd #4", Processor::M68010),
            ("    movec vbr,d0", Processor::M68020),
        ] {
            assert!(
                analyze_for(text, processor, None).0.is_empty(),
                "{text}, {processor:?}"
            );
        }
        let (_, _, found) = analyze_for("    muls.l d0,d1", Processor::M68020, None);
        assert_eq!(
            found[0].message(),
            "`muls.l d0,d1` is a 68020 instruction, and is not implemented yet in s68k's 68020 \
             dialect."
        );
        let (_, _, found) = analyze_for("    bftst (a0){0:8}", Processor::M68000, None);
        assert_eq!(
            found[0].message(),
            "`bftst` is a 68020 bit-field instruction, and bit fields are not implemented yet."
        );
    }

    /// `.l` on a branch is a word where there is no long branch, as in
    /// EASy68K, and a long one on the 68020.
    #[test]
    fn a_long_branch_is_the_68020s() {
        for (processor, expected) in [
            (Processor::M68000, BranchSize::Word),
            (Processor::M68010, BranchSize::Word),
            (Processor::M68020, BranchSize::Long),
        ] {
            for text in ["    bra.l $2000", "    bsr.l $2000", "    beq.l $2000"] {
                let (codes, instruction, _) = analyze_for(text, processor, None);
                assert!(codes.is_empty(), "{text}: {codes:?}");
                let size = match instruction {
                    Some(
                        Instruction::BRA(_, size)
                        | Instruction::BSR(_, size)
                        | Instruction::Bcc(_, _, size),
                    ) => size,
                    other => panic!("{text} lowered to {other:?}"),
                };
                assert_eq!(size, expected, "{text}, {processor:?}");
            }
        }
        // A branch too far for a word is told to write `.l` on the 68020.
        let (_, _, found) = analyze_for("    bra.w $20000", Processor::M68020, None);
        assert!(
            found[0].hint().unwrap().contains("`bra.l`"),
            "{:?}",
            found[0].hint()
        );
        let (_, _, found) = analyze_for("    bra.w $20000", Processor::M68000, None);
        assert!(found[0].hint().unwrap().contains("`jmp`"));
    }

    #[test]
    fn a_byte_never_reaches_an_address_register() {
        assert_eq!(codes("    move.b d0,a0"), ["address_register_byte_size"]);
        assert_eq!(codes("    addq.b #1,a0"), ["address_register_byte_size"]);
        // The register is read here and written above, and the sentence has to
        // hold for both: nothing is written to `a0` in `cmp.b a0,d1`.
        assert_eq!(codes("    cmp.b a0,d1"), ["address_register_byte_size"]);
        assert_eq!(
            messages("    move.b d0,a0"),
            [
                "`move.b` uses an address register, and an address register is never used \
                 one byte at a time. Use `.w` or `.l`"
            ]
        );
        assert_eq!(
            messages("    cmp.b a0,d1"),
            [
                "`cmp.b` uses an address register, and an address register is never used \
                 one byte at a time. Use `.w` or `.l`"
            ]
        );
    }

    /// A space where the comma goes is not silent any more.
    ///
    /// `move.l d0 d1` is one Operand and a bare Comment field, which is the
    /// commonest first-year typo and the one the rewrite exists to stop
    /// misreading; the count error says the count is wrong and this says where
    /// the second operand went.
    #[test]
    fn missing_comma_between_operands_is_named() {
        assert_eq!(
            codes("    move.l  d0 d1"),
            ["wrong_operand_count", "missing_comma_between_operands"]
        );
        assert_eq!(
            messages("    move.l  d0 d1")[1],
            "The operand field ended at the space before `d1`, and `d1` was read as a comment. \
             Write `d0,d1`"
        );
        // Every shape an Operand can start with, and a name the program defines.
        for text in [
            "    move.l  d0 (a1)",
            "    add.w   d0 #4",
            "    move.l  d0 -(a7)",
        ] {
            assert_eq!(
                codes(text),
                ["wrong_operand_count", "missing_comma_between_operands"],
                "{text}"
            );
        }
        assert_eq!(
            analyze_with(
                "    move.l  d0 count",
                Symbols(HashMap::from([("count".to_string(), 4)]))
            )
            .0,
            ["wrong_operand_count", "missing_comma_between_operands"]
        );
        // And the narrowness: prose after the word, a word that is no operand,
        // and a line whose operands are all there.
        assert_eq!(
            codes("    move.l  d0 d1 is the target"),
            ["wrong_operand_count"]
        );
        assert_eq!(codes("    move.l  d0 copy"), ["wrong_operand_count"]);
        assert!(codes("    move.l  d0,d1 copy it").is_empty());
    }

    #[test]
    fn one_memory_operand_to_an_add() {
        assert_eq!(codes("    add.l (a0),(a1)"), ["both_operands_in_memory"]);
        assert_eq!(codes("    or.w $2000,$2004"), ["both_operands_in_memory"]);
        assert!(codes("    add.l (a0),d1").is_empty());
        assert!(
            codes("    move.l (a0),(a1)").is_empty(),
            "`move` is the one that reaches memory twice"
        );
    }

    #[test]
    fn a_quick_form_says_what_it_holds() {
        assert_eq!(
            messages("    addq.l #9,d0"),
            ["The count of `addq` is 1 to 8, and `9` is outside it. `add #n,<ea>` has no such limit"]
        );
        assert_eq!(codes("    subq.w #0,d0"), ["value_out_of_range"]);
        assert_eq!(codes("    moveq #256,d0"), ["value_out_of_range"]);
        assert!(codes("    moveq #255,d0").is_empty(), "`$ff` is `-1`");
        assert!(codes("    moveq #-128,d0").is_empty());
        assert_eq!(codes("    asl.l #9,d0"), ["value_out_of_range"]);
        // A static bit number past the width assembles, as in EASy68K, and is
        // warned of: the 68000 takes it modulo the width.
        assert_eq!(codes("    btst #32,d0"), ["bit_number_wraps"]);
        assert_eq!(
            messages("    btst #9,(a0)"),
            [
                "Bit 9 is bit 1: a byte in memory has 8 bits, and the 68000 takes the bit \
                 number modulo 8. Write `#1` if bit 1 is the one meant; bits are numbered from 0"
            ],
        );
        assert!(codes("    btst #7,(a0)").is_empty());
        assert!(codes("    btst #31,d0").is_empty());
    }

    #[test]
    fn an_immediate_is_checked_against_the_size_it_is_used_at() {
        assert_eq!(
            messages("    move.b #300,d0"),
            ["`#300` does not fit in a byte. A byte immediate holds -128 to 255"]
        );
        assert!(codes("    move.b #-1,d0").is_empty());
        assert!(codes("    move.b #255,d0").is_empty());
        assert_eq!(codes("    move.w #70000,d0"), ["immediate_out_of_range"]);
        assert!(codes("    move.l #70000,d0").is_empty());
    }

    #[test]
    fn a_displacement_is_checked_against_the_field_it_is_encoded_in() {
        assert_eq!(codes("    move.l 40000(a0),d0"), ["value_out_of_range"]);
        assert!(codes("    move.l 30000(a0),d0").is_empty());
        assert_eq!(codes("    move.l 200(a0,d1.w),d0"), ["value_out_of_range"]);
        assert_eq!(codes("    move.l 4(a0,d1.b),d0"), ["invalid_size"]);
    }

    #[test]
    fn a_bare_number_below_the_origin_is_probably_a_missing_hash() {
        assert_eq!(
            messages("    move.l 5,d0"),
            [
                "`5` here means the contents of address 5, not the number 5. Write `#5` for the \
                 number itself"
            ]
        );
        assert!(
            codes("    move.l $2000,d0").is_empty(),
            "an address above the origin is an address"
        );
        assert!(
            codes("    bra $10").is_empty(),
            "a branch takes no immediate, so a low address is not a missing `#`"
        );
        let mut symbols = HashMap::new();
        symbols.insert("count".to_string(), 5);
        assert!(
            analyze_with("    move.l count,d0", Symbols(symbols))
                .0
                .is_empty(),
            "a symbol is not a bare number"
        );
    }

    #[test]
    fn a_star_where_no_operand_belongs_is_the_current_address() {
        let (found, instruction) = analyze("    nop * do nothing");
        assert_eq!(found, ["star_is_the_current_address"]);
        assert!(
            matches!(instruction, Some(Instruction::NOP)),
            "the line still assembles, as an EASy68K program has to (ADR 0001)"
        );
        assert_eq!(
            codes("    nop *+2"),
            ["wrong_operand_count"],
            "only a bare `*` is the comment marker"
        );
    }

    #[test]
    fn the_wrong_number_of_operands_says_how_many_it_takes() {
        assert_eq!(
            messages("    move.l d0"),
            ["`move` takes two operands, and this line has one."]
        );
        assert_eq!(
            messages("    rts d0"),
            ["`rts` takes no operands, and this line has one."]
        );
        assert_eq!(
            messages("    asl.l #1,d0,d2"),
            ["`asl` takes one or two operands, and this line has three."]
        );
    }

    #[test]
    fn a_line_the_parser_has_already_failed_on_gets_no_second_message() {
        assert_eq!(
            codes("    move.l (d0),d1"),
            Vec::<String>::new(),
            "the parser has said what is wrong; a count of the survivors would not help"
        );
        assert_eq!(
            codes("    movea.b (d0),d1"),
            ["invalid_size"],
            "the size is still the analyzer's to judge"
        );
    }

    #[test]
    fn a_directive_is_left_to_its_own_phase() {
        for text in ["    dc.b 1,2,3", "    org $1000", "count equ 5", "    end"] {
            assert!(codes(text).is_empty(), "{text} is phase 2's");
        }
    }

    #[test]
    fn the_normalisations_of_the_printer_survive_the_analyzer() {
        assert!(matches!(
            analyze("    add.w #1,a0").1,
            Some(Instruction::ADDQ(1, _, Size::Word))
        ));
        assert!(matches!(
            analyze("    add.w #100,a0").1,
            Some(Instruction::ADDA(..))
        ));
        assert!(matches!(
            analyze("    add.w #100,d0").1,
            Some(Instruction::ADDI(100, _, Size::Word))
        ));
        assert!(matches!(
            analyze("    cmp.w #1,a0").1,
            Some(Instruction::CMPA(..))
        ));
        assert!(matches!(
            analyze("    cmp.w #1,d0").1,
            Some(Instruction::CMP(..))
        ));
        assert!(matches!(
            analyze("    cmp.w #1,(a0)").1,
            Some(Instruction::CMPI(..))
        ));
        assert!(matches!(
            analyze("    and.w #1,(a0)").1,
            Some(Instruction::ANDI(..))
        ));
        assert!(matches!(
            analyze("    eor.w #1,d0").1,
            Some(Instruction::EORI(..))
        ));
        assert!(matches!(
            analyze("    move.l d0,a0").1,
            Some(Instruction::MOVEA(..))
        ));
        assert!(matches!(
            analyze("    cmp.b (a0)+,(a1)+").1,
            Some(Instruction::CMPM(..))
        ));
        assert!(matches!(
            analyze("    sub.l a0,a1").1,
            Some(Instruction::SUBA(..))
        ));
    }

    #[test]
    fn an_expression_is_folded_the_way_easy68k_reads_it() {
        let symbols = Symbols(HashMap::new());
        let context = Context::new(&symbols);
        let value = |text: &str| {
            let line = crate::assembler::parse_line(text);
            let operand = line.operation.expect("an operation").operands.remove(0);
            match operand {
                Operand::Immediate { value, .. } => context.evaluate(&value),
                Operand::Absolute { value, .. } => context.evaluate(&value),
                other => panic!("expected a value operand, got {other:?}"),
            }
        };
        assert_eq!(value("    move.l #1<<2+3,d0"), Some(7));
        assert_eq!(value("    move.l #'A',d0"), Some(65));
        assert_eq!(value("    move.l #'ab',d0"), Some(0x6162));
        assert_eq!(value("    move.l #-5,d0"), Some(-5));
        assert_eq!(value("    move.l #~0,d0"), Some(-1));
        assert_eq!(value("    move.l *,d0"), Some(DEFAULT_ORIGIN));
        assert_eq!(value("    move.l #7\\4,d0"), Some(3));
        assert_eq!(
            value("    move.l #1/0,d0"),
            None,
            "the evaluator's to report"
        );
        assert_eq!(value("    move.l #undefined,d0"), None);
    }
}
