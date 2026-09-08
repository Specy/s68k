//! The Symbol table: what every name in the program stands for.
//!
//! Four kinds of Symbol (CONTEXT.md, "Symbols and expressions"), each with the
//! Location it was defined at:
//!
//! * a **Label**, the address of the line it is on;
//! * a **Constant**, `equ`, defined once;
//! * a **Variable**, `set`, which may be redefined, each use seeing the latest
//!   definition above it;
//! * a **Register list**, `reg`, stored as the `movem` mask it stands for and
//!   refused inside an Expression.
//!
//! # Local labels
//!
//! A Local label is written `.name` and is visible between the Global label
//! above it and the next one. Its full name is EASy68K's own: "the assembler
//! creates a unique name for local labels by appending the local label name to
//! the preceding global label and replacing the dot with a colon"
//! (`quickStart.htm`, "Label Field"), so `.loop` under `start` is
//! `start:loop`. That is what [`qualify`] builds and what
//! [`SymbolTable::iter`] and the Program's symbol map show. A look-up of
//! `.loop` resolves inside the scope it is made in; a look-up of `start:loop`
//! finds the same Symbol from anywhere, which is what a diagnostic that quotes
//! a name back needs.
//!
//! A Local label written before any Global label is in the File's own nameless
//! scope, and its full name begins with the `:` alone.
//!
//! EASy68K keeps only the first 32 characters of a name significant; s68k keeps
//! all of them, which is the lenient direction of ADR 0001 and costs nothing.
//!
//! # Position
//!
//! Every look-up carries the position it is made at (`at`), the index of the
//! Source line the Expression is on. Only a Variable reads it — it is what
//! "the latest definition above it" means — and every other kind answers the
//! same value wherever it is asked. Phase 4 makes `include` textual, so the
//! line index has to become a position in the *assembled* order then; the one
//! place that compares two of them is [`Symbol::value_at`].

use std::collections::BTreeMap;

use serde::Serialize;

use super::diagnostics::{Diagnostic, DiagnosticKind};
use super::expr::{Resolution, Symbols};
use super::instructions::table;
use super::source::Location;

/// Which of the four kinds of Symbol a name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// A name for the address of the line it is on.
    Label,
    /// `equ`: a name for a value, defined once.
    Constant,
    /// `set`: a name for a value that may be redefined.
    Variable,
    /// `reg`: a name for a `movem` register list.
    RegisterList,
}

impl SymbolKind {
    /// How a message names the kind ("a constant").
    pub fn description(&self) -> &'static str {
        match self {
            SymbolKind::Label => "a label",
            SymbolKind::Constant => "a constant",
            SymbolKind::Variable => "a variable",
            SymbolKind::RegisterList => "a register list",
        }
    }

    /// The name the serialised Program uses.
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Label => "label",
            SymbolKind::Constant => "constant",
            SymbolKind::Variable => "variable",
            SymbolKind::RegisterList => "register_list",
        }
    }
}

/// What a Symbol stands for.
///
/// A Register list is not a number: it may not appear in an Expression, and
/// the mask is what `movem` reads (`Directives/reg.htm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolValue {
    /// A value, in the 64 bits Expressions are computed in.
    Number(i64),
    /// A `movem` register mask, `d0` the lowest bit and `a7` the highest.
    RegisterList(u16),
}

impl SymbolValue {
    /// The value as a number, or `None` for a Register list.
    pub fn number(&self) -> Option<i64> {
        match self {
            SymbolValue::Number(value) => Some(*value),
            SymbolValue::RegisterList(_) => None,
        }
    }
}

/// One value a Variable took, and where it took it.
#[derive(Debug, Clone)]
struct Redefinition {
    /// The position the `set` is at.
    at: usize,
    /// The value from there down to the next `set` of the same name.
    value: i64,
    /// Where it was written.
    location: Location,
}

/// One Symbol: a name, what it stands for, and where it was defined.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// The full name, `global:local` for a Local label ([`qualify`]).
    pub name: String,
    /// Which of the four kinds it is.
    pub kind: SymbolKind,
    /// What it stands for. For a Variable this is the value of its *last*
    /// definition; [`value_at`](Symbol::value_at) answers for a given place.
    pub value: SymbolValue,
    /// Where it was defined — the first definition, and for a Variable the
    /// last.
    pub location: Location,
    /// Every `set` of a Variable, in source order. Empty for the other kinds.
    redefinitions: Vec<Redefinition>,
}

impl Symbol {
    /// The value the Symbol has at `at`, the Source line a look-up is made on.
    ///
    /// A Label, a Constant and a Register list have one value wherever they are
    /// read. A Variable has the value of the latest `set` at or above `at`, and
    /// none at all above its first one, which is what "each use sees the latest
    /// definition above it" (CONTEXT.md, "Variable") means read literally.
    pub fn value_at(&self, at: usize) -> Option<SymbolValue> {
        match self.kind {
            SymbolKind::Variable => self.number_at(at).map(SymbolValue::Number),
            _ => Some(self.value),
        }
    }

    /// The number a Variable stands for at `at`, or `None` above its first
    /// `set`.
    fn number_at(&self, at: usize) -> Option<i64> {
        self.redefinitions
            .iter()
            .rev()
            .find(|redefinition| redefinition.at <= at)
            .map(|redefinition| redefinition.value)
    }

    /// Where the definition a look-up at `at` sees was written.
    pub fn location_at(&self, at: usize) -> &Location {
        match self.kind {
            SymbolKind::Variable => self
                .redefinitions
                .iter()
                .rev()
                .find(|redefinition| redefinition.at <= at)
                .map(|redefinition| &redefinition.location)
                .unwrap_or(&self.location),
            _ => &self.location,
        }
    }
}

/// Every Symbol of one assembly, by full name.
///
/// It is a `BTreeMap`, so [`iter`](SymbolTable::iter) is sorted by name in byte
/// order — the order `tests/corpus/README.md` asks a fixture's maps for — and
/// a "did you mean" over the names is reproducible.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    symbols: BTreeMap<String, Symbol>,
}

impl SymbolTable {
    /// A table with no Symbol in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define `name` in `scope`, or say where it was already defined.
    ///
    /// `at` is the position of the definition, which only a Variable reads.
    /// A `set` of a Variable that is already one is a redefinition and is
    /// allowed; everything else that is defined twice is the
    /// `symbol_already_defined` error, and the Diagnostic it gives back already
    /// carries the first definition as a related Location, as ADR 0003 asks.
    /// That Diagnostic is boxed because it is much the larger half of the
    /// answer and the common answer is `Ok`.
    pub fn define(
        &mut self,
        name: &str,
        scope: Option<&str>,
        kind: SymbolKind,
        value: SymbolValue,
        location: Location,
        at: usize,
    ) -> Result<(), Box<Diagnostic>> {
        let full_name = qualify(name, scope);
        match self.symbols.get_mut(&full_name) {
            Some(existing)
                if existing.kind == SymbolKind::Variable && kind == SymbolKind::Variable =>
            {
                existing.value = value;
                existing.location = location.clone();
                existing.redefinitions.push(Redefinition {
                    at,
                    value: value.number().unwrap_or(0),
                    location,
                });
                Ok(())
            }
            Some(existing) => Err(Box::new(
                Diagnostic::new(
                    DiagnosticKind::SymbolAlreadyDefined {
                        name: full_name.clone(),
                    },
                    location,
                )
                .with_related(
                    existing.location.clone(),
                    format!(
                        "`{full_name}` is {} defined here",
                        existing.kind.description()
                    ),
                ),
            )),
            None => {
                let redefinitions = match kind {
                    SymbolKind::Variable => vec![Redefinition {
                        at,
                        value: value.number().unwrap_or(0),
                        location: location.clone(),
                    }],
                    _ => Vec::new(),
                };
                self.symbols.insert(
                    full_name.clone(),
                    Symbol {
                        name: full_name,
                        kind,
                        value,
                        location,
                        redefinitions,
                    },
                );
                Ok(())
            }
        }
    }

    /// The Symbol `name` stands for, read from `scope`.
    ///
    /// A name that starts with a `.` is a Local label and is looked up inside
    /// `scope`; every other name is looked up as it is written, so that a full
    /// name (`start:.loop`) is found from anywhere.
    pub fn resolve(&self, name: &str, scope: Option<&str>) -> Option<&Symbol> {
        self.symbols.get(&qualify(name, scope))
    }

    /// The Symbol with that full name, whatever scope it belongs to.
    pub fn get(&self, full_name: &str) -> Option<&Symbol> {
        self.symbols.get(full_name)
    }

    /// Whether a Symbol with that full name is defined.
    pub fn contains(&self, full_name: &str) -> bool {
        self.symbols.contains_key(full_name)
    }

    /// Every Symbol, sorted by full name in byte order.
    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values()
    }

    /// How many Symbols are defined.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Whether nothing is defined at all.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// The defined name closest to `name`, for the "did you mean" of
    /// `undefined_symbol`.
    ///
    /// The name is compared with the full names, so a Local label is offered
    /// under the name a message can quote back.
    pub fn closest_name(&self, name: &str) -> Option<String> {
        table::closest_name(name, self.symbols.keys().map(String::as_str)).map(str::to_string)
    }

    /// The table as one line reads it: a scope for the Local labels, and the
    /// position that decides which `set` a Variable is at.
    ///
    /// `declared_below` is what pass 1 passes and pass 2 leaves empty: the full
    /// names the File defines further down, which is what tells a forward
    /// reference from a name that is nowhere at all.
    pub fn in_scope<'a>(
        &'a self,
        scope: Option<&'a str>,
        at: usize,
        declared_below: Option<&'a dyn Fn(&str) -> bool>,
    ) -> SymbolsInScope<'a> {
        SymbolsInScope {
            table: self,
            scope,
            at,
            declared_below,
        }
    }
}

/// The full name of `name` written in `scope`.
///
/// A Global name is its own full name. A Local name is the scope with the local
/// name appended and its dot replaced by a `:`, which is EASy68K's own rule
/// (`quickStart.htm`): `.loop` under `start` is `start:loop`, and a Local name
/// written before any Global label has the empty scope and comes out as
/// `:loop`. A name that holds no leading dot is a Global one and is left
/// alone, which is what lets a diagnostic's full name be looked up again.
pub fn qualify(name: &str, scope: Option<&str>) -> String {
    match is_local(name) {
        true => format!("{}:{}", scope.unwrap_or(""), &name[1..]),
        false => name.to_string(),
    }
}

/// Whether `name` is a Local label's, which is to say it starts with a `.`
/// (`docs/grammar.md` 1.7).
pub fn is_local(name: &str) -> bool {
    name.starts_with('.')
}

/// The Symbols as one line reads them, which is what an Expression is evaluated
/// against.
///
/// It answers [`Symbols`], the evaluator's question, and
/// [`SymbolValues`](super::analyzer::SymbolValues), the analyzer's.
pub struct SymbolsInScope<'a> {
    table: &'a SymbolTable,
    scope: Option<&'a str>,
    at: usize,
    declared_below: Option<&'a dyn Fn(&str) -> bool>,
}

impl Symbols for SymbolsInScope<'_> {
    fn resolve(&self, name: &str) -> Resolution {
        match self.table.resolve(name, self.scope) {
            Some(symbol) => match symbol.value_at(self.at) {
                Some(SymbolValue::Number(value)) => Resolution::Value(value),
                Some(SymbolValue::RegisterList(_)) => Resolution::RegisterList,
                // A `set` read above its first definition: the name exists and
                // has no value here.
                None => Resolution::Later,
            },
            None => {
                let full_name = qualify(name, self.scope);
                match self.declared_below {
                    Some(declared) if declared(&full_name) => Resolution::Later,
                    _ => Resolution::Unknown {
                        suggestion: self.table.closest_name(&full_name),
                    },
                }
            }
        }
    }
}

impl super::analyzer::SymbolValues for SymbolsInScope<'_> {
    fn value_of(&self, name: &str) -> Option<i64> {
        match self.resolve(name) {
            Resolution::Value(value) => Some(value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(line: usize) -> Location {
        Location::new("main.m68k", line, 0, 4)
    }

    fn table_with_a_label() -> SymbolTable {
        let mut table = SymbolTable::new();
        table
            .define(
                "start",
                None,
                SymbolKind::Label,
                SymbolValue::Number(0x1000),
                location(3),
                3,
            )
            .expect("a name defined once");
        table
    }

    #[test]
    fn a_label_carries_its_address_and_its_definition() {
        let table = table_with_a_label();
        let symbol = table.resolve("start", None).expect("the label");
        assert_eq!(symbol.kind, SymbolKind::Label);
        assert_eq!(symbol.value, SymbolValue::Number(0x1000));
        assert_eq!(symbol.location, location(3));
        assert_eq!(symbol.value_at(0), Some(SymbolValue::Number(0x1000)));
    }

    #[test]
    fn a_name_defined_twice_points_at_both_definitions() {
        let mut table = table_with_a_label();
        let error = table
            .define(
                "start",
                None,
                SymbolKind::Constant,
                SymbolValue::Number(1),
                location(9),
                9,
            )
            .expect_err("the second definition");
        assert_eq!(error.code(), "symbol_already_defined");
        assert_eq!(error.location, location(9));
        assert_eq!(error.related.len(), 1);
        assert_eq!(error.related[0].0, location(3));
        assert_eq!(error.related[0].1, "`start` is a label defined here");
    }

    #[test]
    fn a_variable_may_be_redefined_and_a_use_sees_the_one_above_it() {
        let mut table = SymbolTable::new();
        for (line, value) in [(2usize, 38i64), (7, 32)] {
            table
                .define(
                    "size",
                    None,
                    SymbolKind::Variable,
                    SymbolValue::Number(value),
                    location(line),
                    line,
                )
                .expect("`set` redefines");
        }
        let symbol = table.resolve("size", None).expect("the variable");
        assert_eq!(symbol.value_at(0), None, "above the first `set`");
        assert_eq!(symbol.value_at(2), Some(SymbolValue::Number(38)));
        assert_eq!(symbol.value_at(6), Some(SymbolValue::Number(38)));
        assert_eq!(symbol.value_at(7), Some(SymbolValue::Number(32)));
        assert_eq!(symbol.value_at(99), Some(SymbolValue::Number(32)));
        assert_eq!(symbol.location_at(2), &location(2));
        assert_eq!(symbol.location_at(99), &location(7));
    }

    #[test]
    fn a_variable_and_a_constant_are_still_two_definitions() {
        let mut table = SymbolTable::new();
        table
            .define(
                "size",
                None,
                SymbolKind::Variable,
                SymbolValue::Number(1),
                location(0),
                0,
            )
            .expect("the first");
        assert!(
            table
                .define(
                    "size",
                    None,
                    SymbolKind::Constant,
                    SymbolValue::Number(2),
                    location(1),
                    1,
                )
                .is_err(),
            "`equ` does not redefine a `set` variable"
        );
    }

    #[test]
    fn a_local_label_is_scoped_by_the_global_label_above_it() {
        let mut table = SymbolTable::new();
        for (scope, address, line) in [("first", 0x1000, 1), ("second", 0x2000, 5)] {
            table
                .define(
                    ".loop",
                    Some(scope),
                    SymbolKind::Label,
                    SymbolValue::Number(address),
                    location(line),
                    line,
                )
                .expect("the same local name under two scopes");
        }
        assert_eq!(qualify(".loop", Some("first")), "first:loop");
        assert_eq!(
            table.resolve(".loop", Some("first")).map(|s| s.value),
            Some(SymbolValue::Number(0x1000))
        );
        assert_eq!(
            table.resolve(".loop", Some("second")).map(|s| s.value),
            Some(SymbolValue::Number(0x2000))
        );
        assert!(table.resolve(".loop", Some("third")).is_none());
        // The full name reaches the same Symbol from any scope.
        assert!(table.resolve("first:loop", Some("second")).is_some());
    }

    #[test]
    fn a_local_label_before_any_global_one_is_in_the_files_own_scope() {
        assert_eq!(qualify(".loop", None), ":loop");
        assert_eq!(qualify("start", None), "start");
    }

    #[test]
    fn a_register_list_is_a_mask_and_not_a_number() {
        let mut table = SymbolTable::new();
        table
            .define(
                "AllRegs",
                None,
                SymbolKind::RegisterList,
                SymbolValue::RegisterList(0b0000_0001_0000_0011),
                location(0),
                0,
            )
            .expect("the definition");
        let symbol = table.resolve("AllRegs", None).expect("the register list");
        assert_eq!(symbol.value.number(), None);
        assert!(matches!(
            symbol.value,
            SymbolValue::RegisterList(0b0000_0001_0000_0011)
        ));
    }

    #[test]
    fn a_lookup_answers_the_evaluator_and_the_analyzer_alike() {
        use super::super::analyzer::SymbolValues;
        let table = table_with_a_label();
        let symbols = table.in_scope(None, 0, None);
        assert!(matches!(
            symbols.resolve("start"),
            Resolution::Value(0x1000)
        ));
        assert_eq!(symbols.value_of("start"), Some(0x1000));
        assert_eq!(symbols.value_of("finish"), None);
        assert!(matches!(
            symbols.resolve("finish"),
            Resolution::Unknown { .. }
        ));
    }

    #[test]
    fn a_name_defined_further_down_is_a_forward_reference_and_not_an_unknown() {
        let table = table_with_a_label();
        let declared = |name: &str| name == "finish";
        let symbols = table.in_scope(None, 0, Some(&declared));
        assert!(matches!(symbols.resolve("finish"), Resolution::Later));
        assert!(matches!(
            symbols.resolve("nowhere"),
            Resolution::Unknown { .. }
        ));
    }

    #[test]
    fn an_unknown_name_is_offered_the_closest_one() {
        let table = table_with_a_label();
        let symbols = table.in_scope(None, 0, None);
        match symbols.resolve("stat") {
            Resolution::Unknown { suggestion } => assert_eq!(suggestion.as_deref(), Some("start")),
            other => panic!("expected an unknown symbol, found {other:?}"),
        }
    }
}
