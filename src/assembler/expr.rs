//! Evaluation of an Expression, against the Symbols and the current address.
//!
//! An Expression is computed while assembling and never while running
//! (CONTEXT.md, "Expression"). This module is the only place that computes one,
//! and it has two callers:
//!
//! * the Layout and the Directives, which want the Diagnostics as well as the
//!   value — [`evaluate`] collects a [`Problem`] for every name it cannot
//!   answer, and [`diagnose`] turns those into Diagnostics once the caller has
//!   said which line they are on and whether a forward reference is allowed
//!   there;
//! * the analyzer, which wants the value alone — [`value`] is the same fold
//!   with the Problems dropped, because everything it could report has already
//!   been reported by the phase before it.
//!
//! # The arithmetic
//!
//! Values are computed in **64 bits** (the design record, "Symbols and
//! expressions") and range-checked against the operand size afterwards, by the
//! analyzer and by the Directives. The operators are EASy68K's
//! (`Directives/operators.htm`) and the precedence is
//! [`BinaryOperator::precedence`](super::ast::BinaryOperator::precedence).
//! Where the help is silent, this module chooses, and the choice is written
//! down here and in the design record's implementation notes:
//!
//! * `/` **truncates towards zero** and `\` is the remainder of that division,
//!   which is Rust's own `/` and `%` and the C behaviour EASy68K's own
//!   implementation inherits. `x / 0` and `x \ 0` are
//!   [`ProblemKind::DivisionByZero`] and have no value.
//! * `+ - *` **wrap** in 64 bits rather than overflowing: a Diagnostic about a
//!   number nobody can write is worth less than the value the rest of the line
//!   still gets, and the range checks that matter are the ones against the
//!   operand size.
//! * `<<` and `>>` work on the **low 32 bits**, which is the width EASy68K
//!   computes in and the only width that gives `>>` a meaning at all. The
//!   result is that pattern read back as a signed 32-bit number, so
//!   `-1 >> 0` is still `-1` and `-1 >> 1` is `$7fffffff`. The count is read as
//!   an **unsigned 32-bit** count, so a count of 32 or more — a negative count
//!   included, which is what `-1` is when it is read that way — shifts
//!   everything out and gives `0`.
//! * A character literal is its Latin-1 bytes, first byte highest (`'A'` is
//!   65, `'ab'` is `$6162`), which is how 1.4.2 read one and what the help's
//!   `'` operator means. A literal of more than eight characters keeps its last
//!   eight, which is all a 64-bit value holds.
//! * `*` is the current address, which is the address the next byte will be
//!   placed at (CONTEXT.md, "Current address"): the address of the line for an
//!   instruction or a `dc`, and the address `org` is about to leave for an
//!   `org`.

use super::ast::{BinaryOperator, Expr, UnaryOperator};
use super::diagnostics::{Diagnostic, DiagnosticKind};
use super::source::{Location, Span};

/// What a name stands for when an Expression asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A Symbol with a value.
    Value(i64),
    /// A `reg` Symbol, which stands for a register list and has no value in an
    /// Expression (`Directives/reg.htm`).
    RegisterList,
    /// A Symbol that is defined, but not here yet: further down the File, or —
    /// for a `set` Variable — below this line. This is what a forward reference
    /// looks like to the pass that refuses one.
    Later,
    /// A name that is defined nowhere.
    Unknown {
        /// The closest defined name, when there is one.
        suggestion: Option<String>,
    },
}

/// What an Expression may look a name up in.
///
/// The symbol table implements it
/// ([`SymbolsInScope`](super::symbols::SymbolsInScope)); [`NoSymbols`] is what
/// a caller with no program passes.
pub trait Symbols {
    /// What `name` stands for.
    fn resolve(&self, name: &str) -> Resolution;
}

/// A table that knows no name, for an Expression evaluated on its own.
pub struct NoSymbols;

impl Symbols for NoSymbols {
    fn resolve(&self, _name: &str) -> Resolution {
        Resolution::Unknown { suggestion: None }
    }
}

/// Something the evaluator could not do, and where in the line it happened.
///
/// It is not a Diagnostic yet: only the caller knows which line the Expression
/// is on and whether a forward reference is allowed there, which is what
/// [`diagnose`] adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// What went wrong.
    pub kind: ProblemKind,
    /// Where in the line, as a byte range.
    pub span: Span,
}

/// What the evaluator has to say about an Expression: the four things that stop
/// it from having a value, and the two EASy68K warns about and computes anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemKind {
    /// A name that is defined nowhere.
    UndefinedSymbol {
        /// The name that was written.
        name: String,
        /// The closest defined name, when there is one.
        suggestion: Option<String>,
    },
    /// A name that is defined further down, where the value is needed now.
    ForwardReference {
        /// The name that was written.
        name: String,
    },
    /// A `reg` Symbol inside an Expression.
    RegisterList {
        /// The name that was written.
        name: String,
    },
    /// A division or a modulus by zero.
    DivisionByZero,
    /// A character literal of more than four characters, which is EASy68K's
    /// "ASCII constant exceeds 4 characters" (`errors.htm`). A **warning**: the
    /// value is the literal's last four bytes and the line still assembles.
    CharacterLiteralTooLong {
        /// How many characters it holds.
        characters: usize,
    },
    /// A written number above `$ffffffff`, which is EASy68K's "Numeric constant
    /// exceeds 32 bits" (`errors.htm`). A **warning**: the value is kept in the
    /// 64 bits an Expression is computed in and checked against the size it is
    /// used at.
    ConstantAbove32Bits {
        /// The value that was written.
        value: i64,
    },
}

/// The value of `expression`, and every [`Problem`] met on the way.
///
/// Evaluation carries on after a Problem — the two halves of `count+size` are
/// both asked for even when neither answers — so that one line's mistakes are
/// reported together. The value is `None` as soon as anything is unknown.
pub fn evaluate(
    expression: &Expr,
    symbols: &dyn Symbols,
    current_address: i64,
    problems: &mut Vec<Problem>,
) -> Option<i64> {
    match expression {
        Expr::Number { value, span, .. } => {
            // EASy68K's "Numeric constant exceeds 32 bits" (`errors.htm`,
            // `docs/grammar.md` 1.8): the number is kept whole and the warning
            // says it is wider than the machine.
            if *value > u32::MAX as i64 {
                problems.push(Problem {
                    kind: ProblemKind::ConstantAbove32Bits { value: *value },
                    span: *span,
                });
            }
            Some(*value)
        }
        Expr::CharacterLiteral { bytes, span, .. } => {
            // EASy68K's "ASCII constant exceeds 4 characters": four characters
            // are a long, which is the widest value the 68000 holds.
            if bytes.len() > CHARACTERS_IN_A_LITERAL {
                problems.push(Problem {
                    kind: ProblemKind::CharacterLiteralTooLong {
                        characters: bytes.len(),
                    },
                    span: *span,
                });
            }
            Some(character_value(bytes))
        }
        Expr::CurrentAddress { .. } => Some(current_address),
        Expr::Symbol { name, span } => match symbols.resolve(name) {
            Resolution::Value(value) => Some(value),
            Resolution::RegisterList => {
                problems.push(Problem {
                    kind: ProblemKind::RegisterList { name: name.clone() },
                    span: *span,
                });
                None
            }
            Resolution::Later => {
                problems.push(Problem {
                    kind: ProblemKind::ForwardReference { name: name.clone() },
                    span: *span,
                });
                None
            }
            Resolution::Unknown { suggestion } => {
                problems.push(Problem {
                    kind: ProblemKind::UndefinedSymbol {
                        name: name.clone(),
                        suggestion,
                    },
                    span: *span,
                });
                None
            }
        },
        Expr::Unary {
            operator, operand, ..
        } => {
            let value = evaluate(operand, symbols, current_address, problems)?;
            Some(match operator {
                UnaryOperator::Minus => value.wrapping_neg(),
                UnaryOperator::Not => !value,
            })
        }
        Expr::Binary {
            operator,
            left,
            right,
            span,
        } => {
            // Both sides are evaluated even when the first has no value, so
            // that two undefined names in one Expression are two messages.
            let left = evaluate(left, symbols, current_address, problems);
            let right = evaluate(right, symbols, current_address, problems);
            let (left, right) = (left?, right?);
            match operator {
                BinaryOperator::Add => Some(left.wrapping_add(right)),
                BinaryOperator::Subtract => Some(left.wrapping_sub(right)),
                BinaryOperator::Multiply => Some(left.wrapping_mul(right)),
                BinaryOperator::Divide | BinaryOperator::Modulo if right == 0 => {
                    problems.push(Problem {
                        kind: ProblemKind::DivisionByZero,
                        span: *span,
                    });
                    None
                }
                // `wrapping_` covers the one pair `i64` cannot divide,
                // `i64::MIN / -1`, which has no 64-bit answer.
                BinaryOperator::Divide => Some(left.wrapping_div(right)),
                BinaryOperator::Modulo => Some(left.wrapping_rem(right)),
                BinaryOperator::And => Some(left & right),
                BinaryOperator::Or => Some(left | right),
                BinaryOperator::ExclusiveOr => Some(left ^ right),
                BinaryOperator::ShiftLeft => Some(shift(left, right, true)),
                BinaryOperator::ShiftRight => Some(shift(left, right, false)),
            }
        }
    }
}

/// The value of `expression`, with nothing to say about what it cannot work
/// out.
///
/// This is what the analyzer uses: an undefined Symbol, a division by zero and
/// a Register list have all been reported by the phase that laid the line out,
/// and reporting them again would double every message.
pub fn value(expression: &Expr, symbols: &dyn Symbols, current_address: i64) -> Option<i64> {
    evaluate(expression, symbols, current_address, &mut Vec::new())
}

/// A shift on the low 32 bits, the width EASy68K computes in.
///
/// The count is read as an unsigned 32-bit number, so a count of 32 or more —
/// which is what a negative count is when it is read that way — shifts
/// everything out. The result is the 32-bit pattern read back as a signed
/// number, so a shift by zero changes nothing.
fn shift(value: i64, count: i64, left: bool) -> i64 {
    let count = count as u32;
    if count >= 32 {
        return 0;
    }
    let bits = value as u32;
    let shifted = if left { bits << count } else { bits >> count };
    shifted as i32 as i64
}

/// How many characters a character literal holds: four, which is a long
/// (`docs/grammar.md` 1.9, EASy68K's "ASCII constant exceeds 4 characters").
pub const CHARACTERS_IN_A_LITERAL: usize = 4;

/// Whether `expression` holds a character literal of more than
/// [`CHARACTERS_IN_A_LITERAL`] characters.
///
/// The analyzer asks before it range-checks an immediate: the value of
/// `#'abcdefgh'` is a nineteen-digit number the student never wrote, and "that
/// does not fit in a long" is a worse sentence than the warning the evaluator
/// has already raised about the literal itself.
pub fn holds_a_long_character_literal(expression: &Expr) -> bool {
    match expression {
        Expr::CharacterLiteral { bytes, .. } => bytes.len() > CHARACTERS_IN_A_LITERAL,
        Expr::Unary { operand, .. } => holds_a_long_character_literal(operand),
        Expr::Binary { left, right, .. } => {
            holds_a_long_character_literal(left) || holds_a_long_character_literal(right)
        }
        _ => false,
    }
}

/// The value of a character literal: its Latin-1 bytes, first byte highest
/// (`'A'` is 65, `'ab'` is `$6162`).
///
/// A literal of more than eight characters keeps its last eight, which is all a
/// 64-bit value holds; more than four is
/// [`ProblemKind::CharacterLiteralTooLong`], a warning, and the value is
/// computed anyway.
pub fn character_value(bytes: &[u8]) -> i64 {
    let start = bytes.len().saturating_sub(8);
    bytes[start..]
        .iter()
        .fold(0i64, |value, byte| (value << 8) | *byte as i64)
}

/// Where an Expression was written, so that a [`Problem`] can become a
/// Diagnostic.
pub struct Site<'a> {
    /// The File the Expression is in.
    pub file: &'a str,
    /// The 0-based index of the Source line.
    pub line_index: usize,
    /// The text of that line, which is what turns a Span into columns.
    pub line_text: &'a str,
    /// The Directive whose value decides the Layout, when a forward reference
    /// is refused here (`org`, `ds`, `dcb`, `equ`, `set`); `None` in an
    /// instruction Operand and in `dc` data, where one is allowed and where a
    /// [`ProblemKind::ForwardReference`] therefore never arises.
    pub refused_by: Option<&'a str>,
}

/// The Diagnostics of the Problems `evaluate` collected.
pub fn diagnose(problems: &[Problem], site: &Site) -> Vec<Diagnostic> {
    problems
        .iter()
        .map(|problem| {
            let location =
                Location::from_span(site.file, site.line_index, site.line_text, problem.span);
            let kind = match &problem.kind {
                ProblemKind::UndefinedSymbol { name, suggestion } => {
                    DiagnosticKind::UndefinedSymbol {
                        name: name.clone(),
                        suggestion: suggestion.clone(),
                    }
                }
                ProblemKind::ForwardReference { name } => {
                    DiagnosticKind::ForwardReferenceNotAllowed {
                        name: name.clone(),
                        // A forward reference is only ever refused by a Directive
                        // that decides the Layout, and that Directive is what
                        // asked for the value.
                        directive: site.refused_by.unwrap_or("this line").to_string(),
                    }
                }
                ProblemKind::RegisterList { name } => {
                    DiagnosticKind::RegisterListInExpression { name: name.clone() }
                }
                ProblemKind::DivisionByZero => DiagnosticKind::DivisionByZero,
                ProblemKind::CharacterLiteralTooLong { characters } => {
                    DiagnosticKind::CharacterLiteralTooLong {
                        // The literal as it was written, quotes included: the
                        // value it packs to is a number nobody typed and is the
                        // one thing a message about it must not lead with.
                        text: problem.span.text(site.line_text).to_string(),
                        characters: *characters,
                    }
                }
                ProblemKind::ConstantAbove32Bits { value } => DiagnosticKind::ConstantAbove32Bits {
                    text: problem.span.text(site.line_text).to_string(),
                    value: *value,
                },
            };
            Diagnostic::new(kind, location)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::parser;

    /// A table of the two names the tests use, and nothing else.
    struct Table;

    impl Symbols for Table {
        fn resolve(&self, name: &str) -> Resolution {
            match name {
                "count" => Resolution::Value(12),
                "start" => Resolution::Value(0x1000),
                "later" => Resolution::Later,
                "AllRegs" => Resolution::RegisterList,
                _ => Resolution::Unknown {
                    suggestion: Some("count".to_string()),
                },
            }
        }
    }

    /// The value of the Expression of `dc.l <text>`, with the Problems it met.
    fn eval(text: &str, current_address: i64) -> (Option<i64>, Vec<Problem>) {
        let source = format!("    dc.l {text}");
        let (line, diagnostics) = parser::parse_line(&source, "main.m68k", 0);
        assert!(
            diagnostics.iter().all(|d| !d.is_error()),
            "`{text}` does not parse: {:?}",
            diagnostics.iter().map(|d| d.message()).collect::<Vec<_>>()
        );
        let operand = line
            .operation
            .as_ref()
            .and_then(|operation| operation.operands.first().cloned())
            .expect("one operand");
        let expression = match operand {
            crate::assembler::ast::Operand::Absolute { value, .. } => value,
            other => panic!("`{text}` parsed as {other:?}"),
        };
        let mut problems = Vec::new();
        let value = evaluate(&expression, &Table, current_address, &mut problems);
        (value, problems)
    }

    fn value_of(text: &str) -> Option<i64> {
        eval(text, 0x1000).0
    }

    #[test]
    fn a_number_a_symbol_and_the_current_address() {
        assert_eq!(value_of("12"), Some(12));
        assert_eq!(value_of("$ff"), Some(255));
        assert_eq!(value_of("%1010"), Some(10));
        assert_eq!(value_of("@17"), Some(15));
        assert_eq!(value_of("count"), Some(12));
        assert_eq!(value_of("*"), Some(0x1000));
        assert_eq!(eval("*+2", 0x2000).0, Some(0x2002));
    }

    #[test]
    fn a_character_literal_packs_its_bytes_first_byte_highest() {
        assert_eq!(value_of("'A'"), Some(65));
        assert_eq!(value_of("'ab'"), Some(0x6162));
        assert_eq!(value_of("'abcd'"), Some(0x6162_6364));
        assert_eq!(character_value(b"abcdefghij"), 0x6364_6566_6768_696a);
    }

    /// The two warnings EASy68K raises and computes through: a character
    /// literal of more than four characters and a written number above 32 bits.
    ///
    /// Both are warnings, so the value is still answered and the line still
    /// assembles, and both name what was *written* — the packed value of
    /// `'abcdefgh'` is a nineteen-digit number nobody typed.
    #[test]
    fn the_two_warnings_of_the_evaluator_name_what_was_written() {
        let (value, problems) = eval("'abcdefgh'", 0x1000);
        assert_eq!(value, Some(0x6162_6364_6566_6768));
        assert_eq!(
            problems
                .iter()
                .map(|problem| &problem.kind)
                .collect::<Vec<_>>(),
            vec![&ProblemKind::CharacterLiteralTooLong { characters: 8 }]
        );
        assert!(eval("'abcd'", 0x1000).1.is_empty(), "four is the limit");

        let (value, problems) = eval("$1234567890", 0x1000);
        assert_eq!(value, Some(0x0012_3456_7890));
        assert_eq!(
            problems
                .iter()
                .map(|problem| &problem.kind)
                .collect::<Vec<_>>(),
            vec![&ProblemKind::ConstantAbove32Bits {
                value: 0x0012_3456_7890
            }]
        );
        assert!(
            eval("$ffffffff", 0x1000).1.is_empty(),
            "`$ffffffff` is the widest a 32-bit machine holds"
        );

        // The messages quote the source and not the value, which is what
        // `diagnose` reads the line for.
        let source = "    dc.l 'abcdefgh'";
        let site = Site {
            file: "main.m68k",
            line_index: 0,
            line_text: source,
            refused_by: None,
        };
        let problems = eval("'abcdefgh'", 0x1000).1;
        let diagnostics = diagnose(&problems, &site);
        assert_eq!(
            diagnostics[0].message(),
            "`'abcdefgh'` is 8 characters, and a character literal holds at most four."
        );
        assert!(!diagnostics[0].is_error(), "EASy68K warns and assembles");
        assert!(holds_a_long_character_literal(&parsed_expression(
            "'abcdefgh'+1"
        )));
        assert!(!holds_a_long_character_literal(&parsed_expression(
            "'abcd'+1"
        )));
    }

    /// The Expression of `dc.l <text>`, for the tests that want the tree.
    fn parsed_expression(text: &str) -> Expr {
        let source = format!("    dc.l {text}");
        let (line, _) = parser::parse_line(&source, "main.m68k", 0);
        match line
            .operation
            .and_then(|operation| operation.operands.into_iter().next())
        {
            Some(crate::assembler::ast::Operand::Absolute { value, .. }) => value,
            other => panic!("`{text}` parsed as {other:?}"),
        }
    }

    #[test]
    fn the_four_arithmetic_operators_are_easy68ks() {
        assert_eq!(value_of("7+2"), Some(9));
        assert_eq!(value_of("7-2"), Some(5));
        assert_eq!(value_of("7*2"), Some(14));
        assert_eq!(value_of("7/2"), Some(3), "division truncates towards zero");
        assert_eq!(value_of("-7/2"), Some(-3), "and so does it for a negative");
        assert_eq!(value_of("7\\2"), Some(1), "`\\` is the modulus");
        assert_eq!(value_of("-7\\2"), Some(-1));
    }

    #[test]
    fn the_bitwise_operators_are_easy68ks() {
        assert_eq!(value_of("12&10"), Some(8));
        assert_eq!(value_of("12!10"), Some(14));
        assert_eq!(value_of("12|10"), Some(14), "`!` and `|` are one operator");
        assert_eq!(value_of("12^10"), Some(6));
        assert_eq!(value_of("~0"), Some(-1));
        assert_eq!(value_of("-5"), Some(-5));
        assert_eq!(value_of("~-5"), Some(4), "unary operators chain");
    }

    #[test]
    fn a_shift_works_on_the_low_32_bits() {
        assert_eq!(value_of("1<<4"), Some(16));
        assert_eq!(value_of("$100>>4"), Some(16));
        assert_eq!(value_of("1<<31"), Some(-0x8000_0000), "the sign bit");
        assert_eq!(value_of("1<<32"), Some(0), "shifted out of the 32 bits");
        assert_eq!(
            value_of("-1>>0"),
            Some(-1),
            "a shift by zero changes nothing"
        );
        assert_eq!(value_of("-1>>1"), Some(0x7fff_ffff), "and `>>` is logical");
        assert_eq!(value_of("1<<-1"), Some(0), "a negative count is a huge one");
    }

    #[test]
    fn precedence_is_the_helps_own_table() {
        // `>> <<` first, then `& ! | ^`, then `* / \`, then `+ -`.
        assert_eq!(value_of("1<<2+3"), Some(7), "(1<<2)+3");
        assert_eq!(value_of("1+2*3"), Some(7), "1+(2*3)");
        assert_eq!(value_of("2*3&1"), Some(2), "2*(3&1)");
        assert_eq!(value_of("(1+2)*3"), Some(9), "and parentheses win");
    }

    #[test]
    fn the_org_alignment_idiom_of_the_help_works() {
        // `Directives/org.htm`: ORG (*+1)&-2 forces word alignment.
        assert_eq!(eval("(*+1)&-2", 0x1001).0, Some(0x1002));
        assert_eq!(eval("(*+1)&-2", 0x1002).0, Some(0x1002));
        assert_eq!(eval("(*+3)&-4", 0x1001).0, Some(0x1004));
    }

    #[test]
    fn arithmetic_wraps_rather_than_overflowing() {
        assert_eq!(value_of("$7fffffffffffffff+1"), Some(i64::MIN));
        assert_eq!(value_of("$4000000000000000*2"), Some(i64::MIN));
        // The one pair `i64` cannot divide, which has no 64-bit answer.
        assert_eq!(value_of("(0-$7fffffffffffffff-1)/-1"), Some(i64::MIN));
    }

    #[test]
    fn a_division_by_zero_has_no_value_and_one_problem() {
        let (value, problems) = eval("1/0", 0x1000);
        assert_eq!(value, None);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ProblemKind::DivisionByZero);
        let (value, problems) = eval("1\\0", 0x1000);
        assert_eq!(value, None);
        assert_eq!(problems[0].kind, ProblemKind::DivisionByZero);
    }

    #[test]
    fn an_undefined_symbol_is_a_problem_with_the_closest_name() {
        let (value, problems) = eval("nowhere", 0x1000);
        assert_eq!(value, None);
        assert_eq!(
            problems[0].kind,
            ProblemKind::UndefinedSymbol {
                name: "nowhere".to_string(),
                suggestion: Some("count".to_string()),
            }
        );
    }

    #[test]
    fn one_expression_reports_both_of_its_undefined_names() {
        let (_, problems) = eval("nowhere+elsewhere", 0x1000);
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn a_forward_reference_and_a_register_list_are_their_own_problems() {
        let (_, problems) = eval("later", 0x1000);
        assert_eq!(
            problems[0].kind,
            ProblemKind::ForwardReference {
                name: "later".to_string()
            }
        );
        let (_, problems) = eval("AllRegs", 0x1000);
        assert_eq!(
            problems[0].kind,
            ProblemKind::RegisterList {
                name: "AllRegs".to_string()
            }
        );
    }

    #[test]
    fn a_problem_becomes_a_diagnostic_where_it_was_written() {
        let line = "    org  end_of_data";
        let (_, problems) = {
            let mut problems = Vec::new();
            let expression = Expr::Symbol {
                name: "end_of_data".to_string(),
                span: Span::new(9, 20),
            };
            let value = evaluate(&expression, &Table, 0, &mut problems);
            (value, problems)
        };
        let diagnostics = diagnose(
            &problems,
            &Site {
                file: "main.m68k",
                line_index: 4,
                line_text: line,
                refused_by: Some("org"),
            },
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), "undefined_symbol");
        assert_eq!(
            diagnostics[0].location,
            Location::new("main.m68k", 4, 9, 20)
        );
    }

    #[test]
    fn the_silent_evaluator_answers_the_same_value() {
        let expression = Expr::Symbol {
            name: "count".to_string(),
            span: Span::new(0, 5),
        };
        assert_eq!(value(&expression, &Table, 0), Some(12));
        assert_eq!(value(&expression, &NoSymbols, 0), None);
    }
}
