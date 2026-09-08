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
//!    assemble gets the reason;
//! 2. every Operand is a mode this phase assembles — the PC-relative modes and
//!    the special registers are not, and say so;
//! 3. the number of Operands picks the Form;
//! 4. the size suffix against that Form, and the byte rule for address
//!    registers;
//! 5. each Operand's mode against its position;
//! 6. the two Operands together, where the instruction has only one memory
//!    access;
//! 7. the values it can already work out: the count of a quick form, a shift
//!    count, a bit number, a displacement, an immediate against its size, and a
//!    bare number that is probably a missing `#`.
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
use super::diagnostics::{Diagnostic, DiagnosticKind};
use super::expr;
use super::instructions::encoded::{Instruction, Size};
use super::instructions::lowering::{self, Values};
use super::instructions::table::{self, Combination, Form, Implementation, InstructionSpec, Modes};
use super::names;
use super::parser::MacroDefinition;
use super::source::{Location, Span};

/// What the analyzer knows about the Symbols of the program.
///
/// The symbol table implements it (`src/assembler/symbols.rs`, still to be
/// written); [`NoSymbols`] is what a caller with no program passes, and every
/// check that needs a value is then skipped rather than guessed at.
pub trait SymbolValues {
    /// The value of a Symbol, or `None` when it has none the analyzer may use.
    fn value_of(&self, name: &str) -> Option<i64>;
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
}

impl<'a> Context<'a> {
    /// A Context over `symbols`, with the default origin and a current address
    /// of `origin`.
    pub fn new(symbols: &'a dyn SymbolValues) -> Self {
        Self {
            symbols,
            current_address: DEFAULT_ORIGIN,
            origin: DEFAULT_ORIGIN,
            macros: &[],
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

impl Values for Context<'_> {
    fn value_of(&self, expression: &ast::Expr) -> Option<i64> {
        self.evaluate(expression)
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

        let unimplemented: Vec<bool> = operands
            .iter()
            .map(|operand| self.check_operand_is_implemented(operand))
            .collect();

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

        let size = self.check_size(spec, form, operation, operands);
        // An Operand that is not allowed where it stands is not asked about its
        // value as well: one mistake, one message.
        let mut fits = vec![true; operands.len()];
        for (index, operand) in operands.iter().enumerate() {
            if unimplemented[index] {
                fits[index] = false;
                continue;
            }
            fits[index] = self.check_mode(spec, form, index, operand);
            if fits[index] {
                self.check_operand_values(operand);
            }
        }
        self.check_combination(spec, form, operands);
        if fits.first() != Some(&false) {
            self.check_instruction_value(spec, operands);
        }
        self.check_immediate_sizes(size, operands, &fits);
        self.suggest_immediates(form, operands, &fits);

        // Warnings and suggestions leave a program that still builds; an
        // error means there is nothing honest to lower.
        if self.failed {
            return None;
        }
        let family = spec.family()?;
        let lowered: Vec<_> = operands
            .iter()
            .map(|operand| lowering::lower_operand(operand, self.context))
            .collect::<Option<Vec<_>>>()?;
        lowering::lower(family, size, &lowered)
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
                    reason: "it is a macro, and macros are not assembled yet".to_string(),
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
            table::mnemonics().chain(names::DIRECTIVES.iter().copied()),
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

    /// Whether the Operand is a mode this phase does not assemble, in which
    /// case it says so and answers `true`.
    ///
    /// The PC-relative modes and the three special registers are read by the
    /// parser and arrive with phase 3 (the design record, "Instructions"); a
    /// student writing one is told that and what to write meanwhile, rather
    /// than that the mode is invalid, which would be a lie.
    fn check_operand_is_implemented(&mut self, operand: &Operand) -> bool {
        let (description, advice) = match operand {
            Operand::PcDisplacement { .. } | Operand::PcIndex { .. } => (
                "a PC-relative operand",
                Some(
                    "write the label on its own: `label` reads the same place while the program \
                     stays where it is laid out",
                ),
            ),
            Operand::SpecialRegister { register, .. } => match register {
                ast::SpecialRegister::Sr => (
                    "the status register",
                    Some(
                        "the condition codes are set by the instructions themselves; `seq d0` \
                         puts one of them in a register",
                    ),
                ),
                ast::SpecialRegister::Ccr => (
                    "the condition code register",
                    Some(
                        "the condition codes are set by the instructions themselves; `seq d0` \
                         puts one of them in a register",
                    ),
                ),
                ast::SpecialRegister::Usp => (
                    "the user stack pointer",
                    Some("s68k runs one program with one stack pointer, `a7`"),
                ),
            },
            _ => return false,
        };
        self.raise(
            DiagnosticKind::UnimplementedAddressingMode {
                operand: self.text_of(operand.span()),
                description: description.to_string(),
                advice: advice.map(str::to_string),
            },
            operand.span(),
        );
        true
    }

    /// The Form the Operands were written in: the first one with their number
    /// that they all fit, and, when they fit none, the first one of that
    /// number.
    ///
    /// Only `cmp` and `movem` have two Forms of the same arity, and the
    /// fallback is what makes their messages the right ones: `cmp (a0)+,(a1)`
    /// fits neither, and is judged against the general Form ("there it takes
    /// Dn or An") rather than against the `cmpm` shape it half resembles.
    fn choose_form(
        &self,
        spec: &'static InstructionSpec,
        operands: &[Operand],
    ) -> Option<&'static Form> {
        let fits = |form: &&'static Form| {
            operands.iter().enumerate().all(|(index, operand)| {
                match Modes::of(operand) {
                    Some(mode) => form.operands[index].contains(mode),
                    // A mode this phase does not assemble has already been
                    // reported; it decides nothing here.
                    None => true,
                }
            })
        };
        let candidates: Vec<&'static Form> = spec
            .forms
            .iter()
            .filter(|form| form.arity() == operands.len())
            .collect();
        candidates
            .iter()
            .find(|form| fits(form))
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
            return Some(
                "an address register holds an address; move it into a data register first"
                    .to_string(),
            );
        }
        if found == Modes::IMMEDIATE && !allowed.contains(Modes::IMMEDIATE) && index > 0 {
            return Some("an immediate is a value, and nothing can be written to it".to_string());
        }
        if found == Modes::REGISTER_LIST && !allowed.contains(Modes::REGISTER_LIST) {
            return Some("only `movem` takes a register list".to_string());
        }
        None
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
                self.raise(
                    DiagnosticKind::InvalidSize {
                        mnemonic: spec.mnemonic.to_string(),
                        size: written.suffix().to_string(),
                        allowed: spec.sizes().iter().map(quoted_size).collect(),
                    },
                    span,
                );
                return form.sizes.default_size();
            }
        }
        if form.sizes == table::SizeRule::Branch {
            // A branch's suffix is the width of its displacement and not the
            // size it works at: `.b` and `.s` are the same one-byte offset
            // (`Reference/68ks9b.htm`), and none of the four reaches the
            // encoded instruction.
            return form.sizes.default_size();
        }
        let size = lowering::lower_size(operation.size, form.sizes.default_size());
        // Only where a byte was a *choice*: `Scc` writes a byte and `moveq`
        // writes a long, and neither is a size an author picked.
        if size == Some(Size::Byte) && form.sizes == table::SizeRule::Any {
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
    /// of its own: the count of a quick form, a shift count, a bit number, the
    /// vector of `trap`.
    fn check_instruction_value(&mut self, spec: &'static InstructionSpec, operands: &[Operand]) {
        let Some(rule) = spec.value_rule else { return };
        let Some(Operand::Immediate { value, span }) = operands.first() else {
            return;
        };
        let Some(number) = self.context.evaluate(value) else {
            return;
        };
        let max = match (rule.max_in_memory, operands.get(1).and_then(Modes::of)) {
            (Some(max_in_memory), Some(mode)) if Modes::MEMORY.contains(mode) => max_in_memory,
            _ => rule.max,
        };
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
            return;
        }
        self.check_the_only_trap(spec, number, *span);
    }

    /// `trap #15` is the input and output trap and the only one s68k answers;
    /// every other vector assembles on a 68000 and would stop the program here.
    fn check_the_only_trap(&mut self, spec: &'static InstructionSpec, vector: i64, span: Span) {
        if spec.mnemonic != "trap" || vector == 15 {
            return;
        }
        self.raise(
            DiagnosticKind::UnimplementedOperation {
                name: format!("trap #{vector}"),
                reason: "s68k simulates one trap, `#15`, which is its input and output".to_string(),
                alternative: Some("`trap #15`".to_string()),
            },
            span,
        );
    }

    /// The displacement of a displaced Operand, which the 68000 encodes in a
    /// word, and of an indexed one, which it encodes in a byte.
    fn check_operand_values(&mut self, operand: &Operand) {
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
            _ => {}
        }
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
    fn check_immediate_sizes(&mut self, size: Option<Size>, operands: &[Operand], fits: &[bool]) {
        let Some(size) = size else { return };
        let bits = size.to_bits() as u32;
        let min = -(1i64 << (bits - 1));
        let max = (1i64 << bits) - 1;
        for (index, operand) in operands.iter().enumerate() {
            let Operand::Immediate { value, span } = operand else {
                continue;
            };
            if !fits[index] {
                continue;
            }
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
    fn raise(&mut self, kind: DiagnosticKind, span: Span) {
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

    /// The messages one line raises, hint included, for reading them as a
    /// student would.
    fn messages(text: &str) -> Vec<String> {
        let (line, parser_diagnostics) = parser::parse_line(text, "main.m68k", 0);
        let symbols = Symbols(HashMap::new());
        let context = Context::new(&symbols);
        let mut analyzer = Analyzer::new("main.m68k", 0, text, &context);
        analyzer.analyze_line(&line, parser_diagnostics.iter().any(|d| d.is_error()));
        analyzer
            .finish()
            .iter()
            .map(|diagnostic| match diagnostic.hint() {
                Some(hint) => format!("{} — {hint}", diagnostic.message()),
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
            "    extb.l d0",
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
                "`frobnicate` is not an instruction or a directive — start it in column 1, \
                 or end it with a colon, if `frobnicate` is a label"
            ]
        );
        assert_eq!(
            messages("    mvoe d0,d1"),
            [
                "`mvoe` is not an instruction or a directive — did you mean `move`? start it \
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
        assert_eq!(codes("    movep.w d0,4(a0)"), ["unimplemented_operation"]);
        assert_eq!(
            messages("    rte"),
            [
                "`rte` is not implemented: s68k runs every program in supervisor mode and keeps \
                 no exception frames — write `rts` instead"
            ]
        );
        assert_eq!(codes("    trap #3"), ["unimplemented_operation"]);
        assert_eq!(codes("    trap #16"), ["value_out_of_range"]);
    }

    #[test]
    fn the_modes_this_phase_does_not_assemble_say_so() {
        assert_eq!(codes("    move.w sr,d0"), ["unimplemented_addressing_mode"]);
        assert_eq!(
            codes("    move.l label(pc),d0"),
            ["unimplemented_addressing_mode"]
        );
        assert_eq!(
            codes("    move.l usp,a0"),
            ["unimplemented_addressing_mode"]
        );
    }

    #[test]
    fn an_operand_is_judged_against_its_position() {
        assert_eq!(
            messages("    clr a0"),
            [
                "the first operand of `clr` cannot be an address register — `suba.l a0,a0` \
                 clears an address register; there it takes Dn, (An), (An)+, -(An), d(An), \
                 d(An,Xn) or Ea/<label>"
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
        assert_eq!(codes("    extb.w d0"), ["invalid_size"]);
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
                 one byte at a time — use `.w` or `.l`"
            ]
        );
        assert_eq!(
            messages("    cmp.b a0,d1"),
            [
                "`cmp.b` uses an address register, and an address register is never used \
                 one byte at a time — use `.w` or `.l`"
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
            "the operand field ended at the space before `d1`, and `d1` was read as a comment \
             — write `d0,d1`"
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
            ["the count of `addq` is 1 to 8, and `9` is outside it — `add #n,<ea>` has no such limit"]
        );
        assert_eq!(codes("    subq.w #0,d0"), ["value_out_of_range"]);
        assert_eq!(codes("    moveq #256,d0"), ["value_out_of_range"]);
        assert!(codes("    moveq #255,d0").is_empty(), "`$ff` is `-1`");
        assert!(codes("    moveq #-128,d0").is_empty());
        assert_eq!(codes("    asl.l #9,d0"), ["value_out_of_range"]);
        assert_eq!(codes("    btst #32,d0"), ["value_out_of_range"]);
        assert_eq!(
            codes("    btst #8,(a0)"),
            ["value_out_of_range"],
            "a byte in memory has eight bits"
        );
        assert!(codes("    btst #7,(a0)").is_empty());
    }

    #[test]
    fn an_immediate_is_checked_against_the_size_it_is_used_at() {
        assert_eq!(
            messages("    move.b #300,d0"),
            ["`#300` does not fit in a byte — a byte immediate holds -128 to 255"]
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
                "`5` here means the contents of address 5, not the number 5 — write `#5` for the \
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
            ["`move` takes two operands, and this line has one"]
        );
        assert_eq!(
            messages("    rts d0"),
            ["`rts` takes no operands, and this line has one"]
        );
        assert_eq!(
            messages("    asl.l #1,d0,d2"),
            ["`asl` takes one or two operands, and this line has three"]
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
            Some(Instruction::ADDI(..))
        ));
        assert!(matches!(
            analyze("    cmp.w #1,a0").1,
            Some(Instruction::CMPA(..))
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
