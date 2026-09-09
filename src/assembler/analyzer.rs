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
//! 2. every Operand is one this phase assembles — `usp` is the only one that is
//!    not, and it says so;
//! 3. the number of Operands picks the Form;
//! 4. the size suffix against that Form, and the byte rule for address
//!    registers;
//! 5. each Operand's mode against its position;
//! 6. the two Operands together, where the instruction has only one memory
//!    access;
//! 7. the values it can already work out: the count of a quick form, a shift
//!    count, a bit number, a displacement, how far a PC-relative Operand has to
//!    reach, the width an absolute address is forced to, an immediate against
//!    its size, and a bare number that is probably a missing `#`.
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
use super::instructions::table::{
    self, Combination, Family, Form, Implementation, InstructionSpec, Modes,
};
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

    fn instruction_address(&self) -> i64 {
        self.current_address
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
        let judged_as_a_pair = self.check_operand_pair(spec, operands);
        let mut fits = vec![true; operands.len()];
        for (index, operand) in operands.iter().enumerate() {
            if judged_as_a_pair || unimplemented[index] {
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
        lowering::lower_operation(family, size, operands, self.context)
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

    /// Whether the Operand is one this phase does not assemble, in which case
    /// it says so and answers `true`.
    ///
    /// One is left, and it is not an Addressing mode at all: `usp`. s68k runs
    /// one program in supervisor mode and has one stack pointer (the design
    /// record, "Scope"), so a student writing `move usp,a0` is told that and
    /// what to write instead, rather than that the mode is invalid, which would
    /// be a lie. There is no "yet" in the sentence, because there is nothing to
    /// wait for.
    ///
    /// `sr` and `ccr` were here until the first half of phase 3 and the
    /// PC-relative modes until its last: all four are ordinary [`Modes`] now
    /// and the instruction table says which position takes them.
    fn check_operand_is_implemented(&mut self, operand: &Operand) -> bool {
        let Operand::SpecialRegister {
            register: ast::SpecialRegister::Usp,
            ..
        } = operand
        else {
            return false;
        };
        self.raise(
            DiagnosticKind::UnimplementedAddressingMode {
                operand: self.text_of(operand.span()),
                description: "the user stack pointer".to_string(),
                advice: Some("s68k runs one program with one stack pointer, `a7`".to_string()),
            },
            operand.span(),
        );
        true
    }

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
            .filter(|form| agrees_about_the_status_register(form, operands))
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
        if spec.mnemonic == "movep" && found == Modes::INDIRECT {
            // `movep` reaches memory through a displacement and nothing else
            // (`Reference/68ks4g.htm`); the displacement of `(a1)` is 0 and
            // writing it is the whole fix.
            return Some("write the displacement, `0(a1)`".to_string());
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
    /// word, and of an indexed one, which it encodes in a byte; the distance a
    /// PC-relative Operand reaches; and the width an absolute address is
    /// forced to.
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
            Operand::PcDisplacement { displacement, .. } => {
                self.check_pc_distance(displacement, "d(PC)", -32768, 32767);
            }
            Operand::PcIndex {
                displacement,
                index,
                ..
            } => {
                if let Some(displacement) = displacement {
                    self.check_pc_distance(displacement, "d(PC,Xn)", -128, 127);
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
    /// The source writes the address it wants and the Assembler works out the
    /// distance from this instruction's extension word to it
    /// ([`lowering::pc_relative_offset`], the same arithmetic the lowering
    /// stores), so the number the range is about is one the student never
    /// wrote: the message says what it is and the hint says that a plain label
    /// has no such limit.
    fn check_pc_distance(&mut self, address: &ast::Expr, notation: &str, min: i64, max: i64) {
        let Some(distance) = lowering::pc_relative_offset(self.context, address) else {
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
    ///
    /// EASy68K warns that forcing the width *disables* this check and encodes
    /// the low word sign extended, so its `$8000.w` reads `$ff8000`. s68k has
    /// no encoding and reads the address as written, so an address the field
    /// cannot name would quietly mean a different place here: refusing it is
    /// the deviation ADR 0001 records.
    fn check_short_address(&mut self, value: &ast::Expr) {
        let Some(address) = self.context.evaluate(value) else {
            return;
        };
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

/// Whether a Form names `sr` or `ccr` in exactly the positions the Operands
/// wrote one, which is what [`Analyzer::choose_form`] narrows the candidates
/// by.
///
/// A position that takes a half of the status register takes nothing else, so
/// "the Form's modes at this position mention one" and "this Operand is one"
/// answer the same question from the two sides. An Operand that is neither —
/// an ordinary Addressing mode, and `usp`, whose [`Modes::of`] is `None` —
/// agrees with a position that names no half of the register, which is every
/// position of every Form but the four of `move` and the two each of `andi`,
/// `ori` and `eori`.
fn agrees_about_the_status_register(form: &Form, operands: &[Operand]) -> bool {
    operands.iter().enumerate().all(|(index, operand)| {
        let allowed = form.operands[index].intersection(Modes::STATUS);
        match Modes::of(operand).map(|mode| mode.intersection(Modes::STATUS)) {
            Some(written) if !written.is_empty() => allowed == written,
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
        assert_eq!(codes("    stop #$2700"), ["unimplemented_operation"]);
        assert_eq!(
            messages("    rte"),
            [
                "`rte` is not implemented: s68k runs every program in supervisor mode and keeps \
                 no exception frames. Write `rts` instead"
            ]
        );
        assert_eq!(codes("    trap #3"), ["unimplemented_operation"]);
        assert_eq!(codes("    trap #16"), ["value_out_of_range"]);
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

    /// `usp` is the one Operand left that the Assembler does not assemble:
    /// s68k runs one program with one stack pointer (the design record,
    /// "Scope"), so `move usp,a0` and `move a0,usp` say so where `sr`, `ccr`
    /// and the PC-relative modes are all assembled.
    #[test]
    fn the_user_stack_pointer_is_the_one_operand_that_is_not_assembled() {
        assert_eq!(
            messages("    move.l usp,a0"),
            [
                "`usp` is the user stack pointer, which s68k does not assemble. S68k runs one \
                 program with one stack pointer, `a7`"
            ]
        );
        assert_eq!(
            codes("    move.l a0,usp"),
            ["unimplemented_addressing_mode"]
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
            "    move.w ccr,d0",
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

    /// `movep` reaches memory through a displacement and nothing else, and the
    /// mistake that has a name is writing `(a1)` for `0(a1)`.
    #[test]
    fn movep_takes_a_displacement_and_says_so() {
        assert_eq!(codes("    movep.w d0,4(a1)"), Vec::<String>::new());
        assert_eq!(codes("    movep.l 4(a1),d0"), Vec::<String>::new());
        assert_eq!(
            messages("    movep.w d0,(a1)"),
            [
                "The second operand of `movep` cannot be an indirect operand. The operand \
                 should be d(An). Write the displacement, `0(a1)`"
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
