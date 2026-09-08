//! The two questions the parser asks about a name, and the only two.
//!
//! `docs/grammar.md` 2.4 is explicit about them:
//!
//! 1. [`is_operation_name`] answers `label_rule` (1.4): *is this word a
//!    Mnemonic or a Directive name?* A word in column 1 that is one is the
//!    Operation, and a word that is not is a Label.
//! 2. [`text_operation`] answers the Operand field's question (2.5): *does this
//!    Operation take its Operand field as text instead of as an
//!    `operand_list`?* The answer decides whether that text is tokenized at all,
//!    and no later phase can undo tokenizing — read as an `operand_list`,
//!    `include io.x68` takes `.x` for a size suffix and leaves `68` over.
//!
//! Nothing else about a name reaches the parser: the operand rules, the sizes
//! and the value ranges of the instruction table stay the analyzer's ([ADR
//! 0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md)).
//!
//! # Where each list lives
//!
//! The Mnemonics are [`super::instructions::table::TABLE`]'s and are not
//! written here: it is the single source of truth of ADR 0003, and
//! [`is_operation_name`] asks it. The Directive names are here, because the
//! Directives phase has no table of its own yet; question 2's list is a closed
//! one that `docs/grammar.md` writes out itself (`text_operation`, 2.6) and it
//! stays here for good, because it is grammar and not instruction data.

use super::instructions::table;

/// How an Operation named in `text_operation` (`docs/grammar.md` 2.6) reads its
/// Operand field.
///
/// All three are raw text: the difference is what the text means and where it
/// ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextOperandKind {
    /// `include` and `incbin`: one `file_specification`, quoted or bare, whose
    /// extent is `operand_field_extent` (1.5) exactly — rule 1 is what lets
    /// `include 'input output macros.x68'` keep its spaces.
    FileSpecification,
    /// `fail`: the `message_text`, the rest of the line up to a `;`. Commas,
    /// spaces and quotes are ordinary characters in it
    /// (`Directives/fail.htm`).
    MessageText,
    /// A refused Directive's `raw_operand_field`: the same extent as
    /// `message_text`, under the name it carries on an Operation s68k does not
    /// implement. It is what keeps one "not implemented" Diagnostic on
    /// `if.l d1 <hs> #NOON then.s` instead of five syntax errors about `<`.
    RawOperandField,
}

/// Every Directive name s68k reads, implemented, ignored or refused.
///
/// The buckets are the design record's ("Directives"); the refused ones are
/// [`REFUSED_OPERATIONS`] and are Directive names here as well, so that an
/// `endm` in column 1 is an Operation and not a Label (`docs/grammar.md` 2.6).
pub const DIRECTIVES: &[&str] = &[
    "dc", "dcb", "ds", "end", "equ", "fail", "incbin", "include", "list", "nolist", "offset",
    "opt", "org", "page", "reg", "section", "set", "simhalt",
];

/// The Operations s68k reads and refuses whole, `refused_operation` of
/// `docs/grammar.md` 2.6: `memory`, the macro and conditional-assembly
/// Directives, and the structured-control keywords.
///
/// Their Operand field is a [`TextOperandKind::RawOperandField`] and is never
/// tokenized. The words that appear only *inside* such a line — `then`, `do`,
/// `to`, `downto`, `by` — are deliberately not here: by the time they are
/// reached the line is raw text.
pub const REFUSED_OPERATIONS: &[&str] = &[
    // The design record's "refuse with a diagnostic that names the feature"
    "memory", // Macros (`Directives/macro.htm`)
    "macro", "endm", "mexit", // Conditional assembly (`Directives/conditional.htm`)
    "ifeq", "ifne", "iflt", "ifle", "ifgt", "ifge", "ifc", "ifnc", "ifarg", "endc",
    // Structured control (`StrucControl/Introduction.htm`)
    "if", "else", "endi", "while", "endw", "for", "endf", "repeat", "until", "dbloop", "unless",
];

/// Whether `name` is the name of a Mnemonic or of a Directive, case
/// insensitively — `label_rule`'s one question (`docs/grammar.md` 1.4).
///
/// A word in column 1 that answers `true` is the Operation of its line; one
/// that answers `false` is a Label.
pub fn is_operation_name(name: &str) -> bool {
    is_mnemonic(name) || is_directive(name)
}

/// Whether `name` is a Mnemonic, case insensitively — the instruction table's
/// answer and nothing else.
pub fn is_mnemonic(name: &str) -> bool {
    table::is_mnemonic(name)
}

/// Whether `name` is a Directive name, refused Directives included, case
/// insensitively.
pub fn is_directive(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let name = name.as_str();
    DIRECTIVES.contains(&name) || REFUSED_OPERATIONS.contains(&name)
}

/// How the Operation `name` reads its Operand field, or `None` when it reads it
/// as an ordinary `operand_list`.
///
/// Matched on the name alone, with any `size_suffix` set aside, because the
/// corpus writes `if.l` and `for.b` (`docs/grammar.md` 2.6).
pub fn text_operation(name: &str) -> Option<TextOperandKind> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "include" | "incbin" => Some(TextOperandKind::FileSpecification),
        "fail" => Some(TextOperandKind::MessageText),
        name if REFUSED_OPERATIONS.contains(&name) => Some(TextOperandKind::RawOperandField),
        _ => None,
    }
}

/// Whether `name` opens a Macro definition, the one construct that spans lines
/// (`docs/grammar.md` 1.3, `macro_definition` 2.6).
pub fn is_macro_start(name: &str) -> bool {
    name.eq_ignore_ascii_case("macro")
}

/// Whether `name` closes a Macro definition.
pub fn is_macro_end(name: &str) -> bool {
    name.eq_ignore_ascii_case("endm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_rule_asks_one_question_about_a_name() {
        assert!(is_operation_name("clr"));
        assert!(is_operation_name("CLR"), "mnemonics are case insensitive");
        assert!(is_operation_name("dc"));
        assert!(is_operation_name("end"));
        assert!(is_operation_name("endm"), "a refused directive is one too");
        assert!(!is_operation_name("loop"));
        assert!(!is_operation_name("start"));
        assert!(!is_operation_name("count"));
    }

    #[test]
    fn text_operation_is_a_closed_list() {
        assert_eq!(
            text_operation("include"),
            Some(TextOperandKind::FileSpecification)
        );
        assert_eq!(
            text_operation("INCBIN"),
            Some(TextOperandKind::FileSpecification)
        );
        assert_eq!(text_operation("fail"), Some(TextOperandKind::MessageText));
        assert_eq!(
            text_operation("macro"),
            Some(TextOperandKind::RawOperandField)
        );
        assert_eq!(text_operation("if"), Some(TextOperandKind::RawOperandField));
        assert_eq!(text_operation("move"), None);
        assert_eq!(text_operation("dc"), None);
        assert_eq!(
            text_operation("then"),
            None,
            "`then` only ever appears inside a line that is already raw text"
        );
    }

    #[test]
    fn every_name_is_lower_case_and_written_once() {
        let mut all: Vec<&str> = table::mnemonics()
            .chain(DIRECTIVES.iter().copied())
            .chain(REFUSED_OPERATIONS.iter().copied())
            .collect();
        for name in &all {
            assert_eq!(
                *name,
                name.to_ascii_lowercase(),
                "`{name}` is written in lower case, because the lookup lowers its argument"
            );
        }
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(count, all.len(), "a name is written once");
    }

    #[test]
    fn the_refused_operations_are_directive_names() {
        for name in REFUSED_OPERATIONS {
            assert!(is_directive(name), "`{name}` is a directive name");
            assert!(is_operation_name(name));
        }
    }
}
