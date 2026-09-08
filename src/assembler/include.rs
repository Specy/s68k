//! The Project as one sequence of lines: `include` expanded, `incbin` left to
//! the Layout.
//!
//! `include` is **textual** (CONTEXT.md, "Include"): the included File's lines
//! are assembled as if they had been pasted at the `include` line, in the same
//! section, at the same current address, in one Symbol namespace, with the
//! Local label scopes running across the boundary. So the Assembler does not
//! assemble a File: it assembles the **assembled sequence**, the Entry file's
//! lines with every `include` line followed by the lines of the File it names,
//! recursively. [`expand`] builds that sequence and [`Expansion`] is what the
//! [Layout](super::layout) walks.
//!
//! # A position, and why it is not a line index
//!
//! An index into the sequence is a **position**. It is what "above" and "below"
//! mean once a File may appear twice: a `set` Variable sees the latest
//! definition above it in the assembled sequence, and a `reg` list has to be
//! defined above the `movem` that reads it. Both were Source line indexes
//! through phases 1 to 3, which the implementation notes said phase 4 would
//! have to change together; this module is why. A [`Position`] carries the
//! File, the line of it, and the Include chain it was reached through.
//!
//! # The Include chain
//!
//! The chain is the sequence of `include` lines that led to a position, from
//! the Entry file down (CONTEXT.md, "Include chain"). It is stored once per
//! *followed* `include`, as a link with a parent, so a File included twice
//! costs two links and not two copies of its lines' provenance;
//! [`Expansion::chain`] walks the links back, innermost first, which is the
//! order a Diagnostic lists them in.
//!
//! # What this module refuses
//!
//! * a File the Project has not got, or one that holds bytes where source
//!   belongs — [`DiagnosticKind::UnreadableFile`], the same kind the Entry file
//!   itself raises;
//! * a File that would be included inside itself —
//!   [`DiagnosticKind::IncludeCycle`], with the chain of paths written out;
//! * a nest deeper than [`MAX_INCLUDE_DEPTH`], and an expansion longer than
//!   [`MAX_ASSEMBLED_LINES`] — [`DiagnosticKind::IncludeTooDeep`], the two
//!   backstops. Neither can be reached by a program anybody meant to write.

use std::collections::HashMap;

use super::ast::Line;
use super::diagnostics::{Diagnostic, DiagnosticKind};
use super::instructions::table;
use super::parser::{self, MacroDefinition, ParsedFile};
use super::source::{FileContent, Files, Location, SourceFile, Span};

/// How many Files an `include` may nest, the Entry file not counted.
///
/// A cycle is caught exactly, by path, so nesting is bounded by the number of
/// Files a Project holds and this limit is a backstop against a Project of
/// hundreds of chained Files rather than a rule a program can trip by accident:
/// eight Files one inside the next is already a chain nobody can read. It also
/// keeps the recursion of [`expand`] shallow, which is what matters on wasm's
/// 1 MiB stack — the same reason the Expression parser has `nesting_too_deep`.
pub const MAX_INCLUDE_DEPTH: usize = 8;

/// How many lines one assembly may take in.
///
/// Including a File twice is allowed, so a File that includes two others which
/// each include two more multiplies rather than adds; this is the bound that
/// keeps a Project nobody meant to write from hanging the editor instead of
/// answering it. It is thirty times the longest program the asm-editor ships
/// (`tests/corpus/editor/bad-apple.x68`, 6645 lines).
pub const MAX_ASSEMBLED_LINES: usize = 200_000;

/// One line of the assembled sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Which File it is in, as an index into the Expansion's Files.
    pub file: usize,
    /// Which line of that File, 0-based.
    pub line: usize,
    /// The `include` line that pulled the File in, as an index into the
    /// Expansion's links; `None` for the Entry file.
    pub chain: Option<usize>,
}

/// One link of an Include chain: an `include` line that was followed.
#[derive(Debug, Clone)]
struct Link {
    /// The file name of the `include` line, which is what a related Location
    /// points at.
    site: Location,
    /// The link the including File was itself reached through.
    parent: Option<usize>,
}

/// One File of the Project that the assembly reads, parsed once.
struct ReadFile<'a> {
    source: SourceFile<'a>,
    parsed: ParsedFile,
}

/// What a written `include` or `incbin` file name turned out to name.
///
/// The same three answers serve `include`, which needs text, and `incbin`,
/// which takes either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved<'a> {
    /// A text File of the Project.
    Text {
        /// Its root-relative path, as the Project spells it.
        path: &'a str,
        /// Its text.
        text: &'a str,
    },
    /// A binary File of the Project.
    Bytes {
        /// Its root-relative path, as the Project spells it.
        path: &'a str,
        /// Its bytes.
        bytes: &'a [u8],
    },
    /// No File of the Project is at any of the paths the name resolves to.
    Missing {
        /// The path as the source wrote it, normalised, which is what a message
        /// quotes back.
        path: String,
    },
}

/// The Project expanded: every line the Assembler assembles, in order.
pub struct Expansion<'a> {
    files: &'a Files,
    entry: String,
    read: Vec<ReadFile<'a>>,
    lines: Vec<Position>,
    chains: Vec<Link>,
    macros: Vec<MacroDefinition>,
}

impl<'a> Expansion<'a> {
    /// How many lines the assembled sequence has.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the Entry file is empty and there is nothing to assemble.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The path of the Entry file.
    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// The root-relative path of the File the position is in.
    pub fn path(&self, at: usize) -> &'a str {
        self.read[self.lines[at].file].source.path()
    }

    /// The 0-based Source line the position is at.
    pub fn line_index(&self, at: usize) -> usize {
        self.lines[at].line
    }

    /// The parsed line at the position.
    pub fn line(&self, at: usize) -> &Line {
        let position = self.lines[at];
        &self.read[position.file].parsed.lines[position.line]
    }

    /// The text of the Source line at the position.
    pub fn text(&self, at: usize) -> &'a str {
        let position = self.lines[at];
        self.read[position.file]
            .source
            .line(position.line)
            .unwrap_or("")
    }

    /// The Location of `span` on the line at the position.
    pub fn location(&self, at: usize, span: Span) -> Location {
        Location::from_span(self.path(at), self.line_index(at), self.text(at), span)
    }

    /// The Location of the whole line at the position.
    pub fn whole_line(&self, at: usize) -> Location {
        Location::whole_line(self.path(at), self.line_index(at), self.text(at))
    }

    /// The Include chain of the position, innermost first.
    ///
    /// Empty for a line of the Entry file. Every Diagnostic raised at the
    /// position carries it as related Locations, and so does every instruction
    /// assembled there (the design record, "Files, `include`, `incbin`").
    pub fn chain(&self, at: usize) -> Vec<Location> {
        let mut chain = Vec::new();
        let mut link = self.lines[at].chain;
        while let Some(index) = link {
            chain.push(self.chains[index].site.clone());
            link = self.chains[index].parent;
        }
        chain
    }

    /// Whether the position is in the Entry file, which is where `end` belongs.
    pub fn is_in_the_entry_file(&self, at: usize) -> bool {
        self.lines[at].chain.is_none()
    }

    /// The two `include` lines that brought one File in twice, when two
    /// positions of one File were reached by two different chains.
    ///
    /// It answers the **outermost** pair that differs, which is the `include`
    /// that is really written twice: a File included once by a File that is
    /// itself included twice is the outer line's doing, and pointing at the
    /// inner one would name the same line twice. `None` when the two positions
    /// are in different Files, or in one copy of one File, where the
    /// duplication is an ordinary one and needs nothing said about `include`.
    pub fn included_twice(&self, first: usize, second: usize) -> Option<(Location, Location)> {
        if self.lines[first].file != self.lines[second].file {
            return None;
        }
        let mut earlier = self.chain(first);
        let mut later = self.chain(second);
        earlier.reverse();
        later.reverse();
        earlier
            .into_iter()
            .zip(later)
            .find(|(outer, inner)| outer != inner)
    }

    /// Every Macro definition of every File the assembly reads.
    ///
    /// `include` is textual, so a Macro defined in an included File is a name
    /// the analyzer knows about wherever it is invoked.
    pub fn macros(&self) -> &[MacroDefinition] {
        &self.macros
    }

    /// Whether the parser reported an error on the line at the position.
    ///
    /// The Layout asks it before judging a line's Operands: what recovery left
    /// behind is not the mistake.
    pub fn parser_found_an_error(&self, at: usize) -> bool {
        let position = self.lines[at];
        self.read[position.file]
            .parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.location.line == position.line && diagnostic.is_error())
    }

    /// What a `file_specification` written on the line at `at` names.
    ///
    /// The path is resolved relative to the including File's directory first
    /// and to the project root second, `\` read as a separator and `.` and `..`
    /// segments resolved on the way (the design record, "Files, `include`,
    /// `incbin`"). This is what `incbin` calls; `include` resolves the same way
    /// while the sequence is being built.
    pub fn resolve(&self, at: usize, written: &str) -> Resolved<'a> {
        resolve_in(self.files, written, self.path(at))
    }

    /// The Diagnostic for a file name that resolves to no File of the Project.
    ///
    /// `directive` is `include` or `incbin`, and the sentence names it; the
    /// closest existing paths are the hint, and a Project of one File says so
    /// instead.
    pub fn miss(&self, path: &str, directive: &str) -> DiagnosticKind {
        missing_file(self.files, path, Some(directive))
    }
}

/// Read the Entry file and expand every `include` under it.
///
/// The Diagnostics come back with the position they were found at, which is
/// what puts them in the order a student reads and what carries the Include
/// chain. The parser's own Diagnostics are among them: a File is parsed once,
/// however many times it is included, and its parser Diagnostics are reported
/// once, at the positions of the first inclusion. A mistake in the *text* of a
/// File is one mistake however often the File is pasted in, and the
/// once-a-File suggestions (`bare_comment`, `double_quoted_string`) are defined
/// that way.
///
/// `Err` is the one failure that leaves nothing to assemble: an Entry file the
/// Project has not got, or one that holds bytes. It is boxed because it is much
/// the larger half of the answer and the common answer is `Ok`, which is the
/// arrangement `SymbolTable::define` already uses.
#[allow(clippy::type_complexity)]
pub fn expand<'a>(
    files: &'a Files,
    entry: &str,
) -> Result<(Expansion<'a>, Vec<(usize, Diagnostic)>), Box<Diagnostic>> {
    let wanted = join("", entry);
    let (path, text) = match files.entry(&wanted) {
        Some((path, FileContent::Text(text))) => (path, text.as_str()),
        content => {
            let kind = match content {
                Some(_) => DiagnosticKind::UnreadableFile {
                    path: wanted.clone(),
                    directive: None,
                    suggestions: Vec::new(),
                    binary: true,
                    alone: files.len() <= 1,
                },
                None => missing_file(files, &wanted, None),
            };
            return Err(Box::new(Diagnostic::new(
                kind,
                Location::new(wanted, 0, 0, 0),
            )));
        }
    };
    let mut expander = Expander {
        files,
        read: Vec::new(),
        expanded: Vec::new(),
        by_path: HashMap::new(),
        lines: Vec::new(),
        chains: Vec::new(),
        macros: Vec::new(),
        diagnostics: Vec::new(),
        budget_reported: false,
    };
    let entry_file = expander.read_file(path, text);
    expander.expand_file(entry_file, None, 0, &mut Vec::new());
    let Expander {
        read,
        lines,
        chains,
        macros,
        diagnostics,
        ..
    } = expander;
    Ok((
        Expansion {
            files,
            entry: path.to_string(),
            read,
            lines,
            chains,
            macros,
        },
        diagnostics,
    ))
}

/// The walk that builds the sequence.
struct Expander<'a> {
    files: &'a Files,
    read: Vec<ReadFile<'a>>,
    /// Whether each File's parser Diagnostics have been reported, which is once
    /// however many times the File is included.
    expanded: Vec<bool>,
    by_path: HashMap<&'a str, usize>,
    lines: Vec<Position>,
    chains: Vec<Link>,
    macros: Vec<MacroDefinition>,
    diagnostics: Vec<(usize, Diagnostic)>,
    budget_reported: bool,
}

impl<'a> Expander<'a> {
    /// Parse a File, or answer the index it was already parsed into.
    fn read_file(&mut self, path: &'a str, text: &'a str) -> usize {
        if let Some(index) = self.by_path.get(path) {
            return *index;
        }
        let parsed = parser::parse_file(path, text);
        self.macros.extend(parsed.macros.iter().cloned());
        let index = self.read.len();
        self.read.push(ReadFile {
            source: SourceFile::new(path, text),
            parsed,
        });
        self.expanded.push(false);
        self.by_path.insert(path, index);
        index
    }

    /// Push every line of one File, following the `include` lines among them.
    ///
    /// `stack` is the Files being expanded, which is what makes a cycle a
    /// finding rather than a hang.
    fn expand_file(
        &mut self,
        file: usize,
        chain: Option<usize>,
        depth: usize,
        stack: &mut Vec<usize>,
    ) {
        stack.push(file);
        let first_time = !self.expanded[file];
        self.expanded[file] = true;
        let count = self.read[file].source.line_count();
        let mut positions = Vec::with_capacity(count);
        for line in 0..count {
            let at = self.lines.len();
            self.lines.push(Position { file, line, chain });
            positions.push(at);
            let included = self.read[file].parsed.lines.get(line).and_then(include_of);
            if let Some((written, span)) = included {
                self.follow(at, file, &written, span, chain, depth, stack);
            }
        }
        if first_time {
            // The parser's Diagnostics belong to the File's text, so they are
            // reported once, at the positions of this first expansion of it.
            let found: Vec<Diagnostic> = self.read[file].parsed.diagnostics.clone();
            for diagnostic in found {
                if let Some(at) = positions.get(diagnostic.location.line) {
                    self.diagnostics.push((*at, diagnostic));
                }
            }
        }
        stack.pop();
    }

    /// Follow one `include` line, or say why it was not followed.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &mut self,
        at: usize,
        file: usize,
        written: &str,
        span: Span,
        chain: Option<usize>,
        depth: usize,
        stack: &mut Vec<usize>,
    ) {
        let from = self.read[file].source.path();
        let location = self.location(at, span);
        let (path, text) = match resolve_in(self.files, written, from) {
            Resolved::Text { path, text } => (path, text),
            Resolved::Bytes { path, .. } => {
                return self.raise(
                    at,
                    location,
                    DiagnosticKind::UnreadableFile {
                        path: path.to_string(),
                        directive: Some("include".to_string()),
                        suggestions: Vec::new(),
                        binary: true,
                        alone: false,
                    },
                );
            }
            Resolved::Missing { path } => {
                let kind = missing_file(self.files, &path, Some("include"));
                return self.raise(at, location, kind);
            }
        };
        let target = self.read_file(path, text);
        if stack.contains(&target) {
            let mut walked: Vec<String> = stack
                .iter()
                .map(|file| self.read[*file].source.path().to_string())
                .collect();
            walked.push(path.to_string());
            return self.raise(
                at,
                location,
                DiagnosticKind::IncludeCycle {
                    path: path.to_string(),
                    chain: walked,
                },
            );
        }
        if depth + 1 > MAX_INCLUDE_DEPTH {
            return self.raise(
                at,
                location,
                DiagnosticKind::IncludeTooDeep {
                    path: path.to_string(),
                    limit: MAX_INCLUDE_DEPTH,
                    nesting: true,
                },
            );
        }
        if self.lines.len() + self.read[target].source.line_count() > MAX_ASSEMBLED_LINES {
            // One sentence is enough for a Project that has run away: every
            // `include` below this point would say the same thing.
            if !self.budget_reported {
                self.budget_reported = true;
                self.raise(
                    at,
                    location,
                    DiagnosticKind::IncludeTooDeep {
                        path: path.to_string(),
                        limit: MAX_ASSEMBLED_LINES,
                        nesting: false,
                    },
                );
            }
            return;
        }
        self.chains.push(Link {
            site: location,
            parent: chain,
        });
        let link = self.chains.len() - 1;
        self.expand_file(target, Some(link), depth + 1, stack);
    }

    fn location(&self, at: usize, span: Span) -> Location {
        let position = self.lines[at];
        let source = &self.read[position.file].source;
        Location::from_span(
            source.path(),
            position.line,
            source.line(position.line).unwrap_or(""),
            span,
        )
    }

    fn raise(&mut self, at: usize, location: Location, kind: DiagnosticKind) {
        self.diagnostics.push((at, Diagnostic::new(kind, location)));
    }
}

/// The file name an `include` line writes, with its quotes taken off, and where
/// it was written.
///
/// `None` for every other line, and for an `include` that names no File at all
/// — which the Layout answers with `wrong_operand_count`, since the expansion
/// has nothing to say about a line with no file name in it.
fn include_of(line: &Line) -> Option<(String, Span)> {
    let operation = line.operation.as_ref()?;
    if !operation.name.eq_ignore_ascii_case("include") {
        return None;
    }
    let field = operation.text.as_ref()?;
    let written = written_path(&field.text);
    match written.is_empty() {
        true => None,
        false => Some((written, field.span)),
    }
}

/// A `file_specification` with its quotes taken off.
///
/// Either quote will do and neither is part of the name
/// (`Directives/include.htm`: quotes are needed only "if any part of the file
/// path or name includes spaces"). A doubled quote inside a quoted name is one
/// quote, which is the rule the parser's `file_specification_extent` already
/// reads the field by.
pub fn written_path(field: &str) -> String {
    let trimmed = field.trim();
    let mut characters = trimmed.chars();
    match (characters.next(), characters.next_back()) {
        (Some(quote @ ('\'' | '"')), Some(last)) if last == quote => {
            let inner = &trimmed[quote.len_utf8()..trimmed.len() - quote.len_utf8()];
            let doubled = format!("{quote}{quote}");
            inner.replace(&doubled, quote.encode_utf8(&mut [0; 4]))
        }
        _ => trimmed.to_string(),
    }
}

/// What `written`, read on a line of `from`, names in `files`.
fn resolve_in<'a>(files: &'a Files, written: &str, from: &str) -> Resolved<'a> {
    for candidate in candidates(written, from) {
        match files.entry(&candidate) {
            Some((path, FileContent::Text(text))) => return Resolved::Text { path, text },
            Some((path, FileContent::Bytes(bytes))) => return Resolved::Bytes { path, bytes },
            None => {}
        }
    }
    Resolved::Missing {
        path: join("", written),
    }
}

/// The paths a written file name is looked for at, in order: beside the File
/// that wrote it, then at the project root.
fn candidates(written: &str, from: &str) -> Vec<String> {
    let directory = match from.rfind('/') {
        Some(at) => &from[..at],
        None => "",
    };
    let beside = join(directory, written);
    let at_root = join("", written);
    match beside == at_root {
        true => vec![beside],
        false => vec![beside, at_root],
    }
}

/// `base` and `written` joined into one root-relative path.
///
/// `\` is a separator like `/` (`Directives/include.htm`'s own example is
/// `"C:\EASy68K\macros\input output macros.x68"`), empty and `.` segments are
/// dropped, a `..` climbs one segment, and a `..` that would climb above the
/// project root is dropped, because a Project has no above-the-root
/// (CONTEXT.md, "File").
///
/// It is public because a caller outside the Assembler may have to resolve a
/// written file name exactly as the Assembler does — the same `\`, the same
/// `..`, the same root — and there is no second implementation of that rule.
pub fn join(base: &str, written: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in base.split('/').chain(written.split(['/', '\\'])) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// The Diagnostic for a file name that names no File of the Project.
///
/// The hint is the closest existing paths, which is what turns "there is no
/// file `io.m68k`" into something a student can act on; a Project that holds no
/// other File says that instead, because there is nothing to have meant.
fn missing_file(files: &Files, path: &str, directive: Option<&str>) -> DiagnosticKind {
    DiagnosticKind::UnreadableFile {
        path: path.to_string(),
        directive: directive.map(str::to_string),
        suggestions: closest_paths(path, files),
        binary: false,
        alone: files.len() <= 1,
    }
}

/// How many paths a miss offers at most.
const SUGGESTION_COUNT: usize = 3;

/// The existing paths closest to `path`, sorted as the Project sorts them.
///
/// Two rules, and the first is the one that matters: a File whose **name** is
/// the name that was written is what `include io.m68k` meant when the File is
/// `lib/io.m68k`, and no edit distance over whole paths would find it — the
/// name written with no extension counts too, since EASy68K requires the
/// extension and a student who leaves it off has still named the File. The
/// second rule is the ordinary did-you-mean over the whole path, for a
/// misspelling (`lib/io.m86k`), and it is asked only when the first finds
/// nothing.
pub fn closest_paths(path: &str, files: &Files) -> Vec<String> {
    let name = file_name(path).to_ascii_lowercase();
    let by_name: Vec<String> = files
        .paths()
        .filter(|candidate| {
            let candidate = file_name(candidate).to_ascii_lowercase();
            candidate == name || stem(&candidate) == name
        })
        .take(SUGGESTION_COUNT)
        .map(str::to_string)
        .collect();
    if !by_name.is_empty() {
        return by_name;
    }
    let written = path.to_ascii_lowercase();
    let limit = if written.len() <= 3 { 1 } else { 2 };
    let mut best = usize::MAX;
    let mut found: Vec<String> = Vec::new();
    for candidate in files.paths() {
        let distance = table::edit_distance(&written, candidate);
        if distance > limit || distance > best {
            continue;
        }
        if distance < best {
            best = distance;
            found.clear();
        }
        found.push(candidate.to_string());
    }
    found.truncate(SUGGESTION_COUNT);
    found
}

/// The last segment of a path.
fn file_name(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((_, name)) => name,
        None => path,
    }
}

/// A file name without its extension.
fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) => stem,
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> Files {
        let mut project = Files::new();
        for (path, text) in files {
            project.insert_text(path, *text);
        }
        project
    }

    /// The sequence as `path:line` for each position, which is what "the
    /// assembled sequence" means read out loud.
    fn sequence(files: &Files, entry: &str) -> Vec<String> {
        let (unit, _) = expand(files, entry).expect("an entry file");
        (0..unit.len())
            .map(|at| format!("{}:{}", unit.path(at), unit.line_index(at)))
            .collect()
    }

    fn codes(files: &Files, entry: &str) -> Vec<&'static str> {
        let (_, diagnostics) = expand(files, entry).expect("an entry file");
        diagnostics
            .iter()
            .map(|(_, diagnostic)| diagnostic.code())
            .collect()
    }

    #[test]
    fn the_included_lines_are_pasted_at_the_include_line() {
        let files = project(&[
            ("main.m68k", "    nop\n    include lib.m68k\n    rts\n"),
            ("lib.m68k", "    move.l #1,d0\n"),
        ]);
        assert_eq!(
            sequence(&files, "main.m68k"),
            vec!["main.m68k:0", "main.m68k:1", "lib.m68k:0", "main.m68k:2"]
        );
    }

    #[test]
    fn a_file_may_be_included_twice() {
        let files = project(&[
            ("main.m68k", "    include lib.m68k\n    include lib.m68k\n"),
            ("lib.m68k", "    nop\n"),
        ]);
        assert_eq!(
            sequence(&files, "main.m68k"),
            vec!["main.m68k:0", "lib.m68k:0", "main.m68k:1", "lib.m68k:0"]
        );
        assert!(codes(&files, "main.m68k").is_empty());
    }

    #[test]
    fn the_chain_is_innermost_first() {
        let files = project(&[
            ("main.m68k", "    include a.m68k\n"),
            ("a.m68k", "    include b.m68k\n"),
            ("b.m68k", "    nop\n"),
        ]);
        let (unit, _) = expand(&files, "main.m68k").expect("an entry file");
        let deepest = unit.len() - 1;
        assert_eq!(unit.path(deepest), "b.m68k");
        let chain: Vec<String> = unit
            .chain(deepest)
            .iter()
            .map(|location| format!("{}:{}", location.file, location.line))
            .collect();
        assert_eq!(chain, vec!["a.m68k:0", "main.m68k:0"]);
        assert!(unit.chain(0).is_empty(), "the entry file has no chain");
        assert!(unit.is_in_the_entry_file(0));
        assert!(!unit.is_in_the_entry_file(deepest));
    }

    #[test]
    fn a_path_resolves_beside_the_including_file_first_and_at_the_root_second() {
        let files = project(&[
            (
                "lecture/main.m68k",
                "    include io.m68k\n    include ..\\lib\\shared.m68k\n",
            ),
            ("lecture/io.m68k", "    nop\n"),
            ("lib/shared.m68k", "    nop\n"),
        ]);
        assert_eq!(
            sequence(&files, "lecture/main.m68k"),
            vec![
                "lecture/main.m68k:0",
                "lecture/io.m68k:0",
                "lecture/main.m68k:1",
                "lib/shared.m68k:0"
            ]
        );
    }

    #[test]
    fn a_path_at_the_root_is_found_from_a_subdirectory() {
        let files = project(&[
            ("lecture/main.m68k", "    include lib/io.m68k\n"),
            ("lib/io.m68k", "    nop\n"),
        ]);
        assert_eq!(
            sequence(&files, "lecture/main.m68k"),
            vec!["lecture/main.m68k:0", "lib/io.m68k:0"]
        );
    }

    #[test]
    fn quotes_are_optional_and_either_kind_will_do() {
        assert_eq!(written_path("io.m68k"), "io.m68k");
        assert_eq!(written_path("'input output.x68'"), "input output.x68");
        assert_eq!(written_path("\"lib/io.x68\""), "lib/io.x68");
        assert_eq!(written_path("'it''s.x68'"), "it's.x68");
        assert_eq!(written_path("'"), "'");
    }

    #[test]
    fn a_backslash_is_a_separator_and_dot_segments_are_resolved() {
        assert_eq!(join("", "..\\lib\\io.x68"), "lib/io.x68");
        assert_eq!(join("lecture", "../lib/io.x68"), "lib/io.x68");
        assert_eq!(join("lecture/deep", "../io.x68"), "lecture/io.x68");
        assert_eq!(join("", "/./a//b.x68"), "a/b.x68");
        assert_eq!(join("", "../../above.x68"), "above.x68");
    }

    #[test]
    fn a_cycle_is_refused_with_the_chain_that_made_it() {
        let files = project(&[
            ("main.m68k", "    include a.m68k\n"),
            ("a.m68k", "    include main.m68k\n"),
        ]);
        let (_, diagnostics) = expand(&files, "main.m68k").expect("an entry file");
        assert_eq!(diagnostics.len(), 1);
        let message = diagnostics[0].1.message();
        assert_eq!(diagnostics[0].1.code(), "include_cycle");
        assert!(
            message.contains("main.m68k -> a.m68k -> main.m68k"),
            "{message}"
        );
    }

    #[test]
    fn a_file_that_includes_itself_is_a_cycle() {
        let files = project(&[("main.m68k", "    include main.m68k\n")]);
        assert_eq!(codes(&files, "main.m68k"), vec!["include_cycle"]);
    }

    #[test]
    fn a_nest_deeper_than_the_limit_is_refused() {
        let mut files = Files::new();
        files.insert_text("main.m68k", "    include f1.m68k\n");
        for step in 1..=MAX_INCLUDE_DEPTH + 1 {
            files.insert_text(
                &format!("f{step}.m68k"),
                format!("    include f{}.m68k\n", step + 1),
            );
        }
        files.insert_text(&format!("f{}.m68k", MAX_INCLUDE_DEPTH + 2), "    nop\n");
        assert_eq!(codes(&files, "main.m68k"), vec!["include_too_deep"]);
    }

    #[test]
    fn a_missing_file_offers_the_closest_paths() {
        let files = project(&[
            ("main.m68k", "    include io.m68k\n"),
            ("lib/io.m68k", "    nop\n"),
        ]);
        let (_, diagnostics) = expand(&files, "main.m68k").expect("an entry file");
        assert_eq!(diagnostics[0].1.code(), "unreadable_file");
        assert_eq!(
            diagnostics[0].1.hint().as_deref(),
            Some("did you mean `lib/io.m68k`?")
        );
    }

    #[test]
    fn a_project_of_one_file_says_so() {
        let files = project(&[("main.m68k", "    include io.m68k\n")]);
        let (_, diagnostics) = expand(&files, "main.m68k").expect("an entry file");
        let hint = diagnostics[0].1.hint().unwrap_or_default();
        assert!(hint.contains("no other file"), "{hint}");
    }

    #[test]
    fn including_bytes_points_at_incbin() {
        let mut files = Files::new();
        files.insert_text("main.m68k", "    include sprite.bin\n");
        files.insert_bytes("sprite.bin", vec![1, 2, 3]);
        let (_, diagnostics) = expand(&files, "main.m68k").expect("an entry file");
        assert_eq!(diagnostics[0].1.code(), "unreadable_file");
        assert!(diagnostics[0]
            .1
            .hint()
            .unwrap_or_default()
            .contains("incbin"));
    }

    #[test]
    fn an_entry_file_that_is_missing_or_binary_is_the_one_failure() {
        let files = project(&[("main.m68k", "    nop\n")]);
        let missing = match expand(&files, "other.m68k") {
            Err(diagnostic) => diagnostic,
            Ok(_) => panic!("an entry file the project has not got"),
        };
        assert_eq!(missing.code(), "unreadable_file");
        assert_eq!(missing.location.file, "other.m68k");

        let mut files = Files::new();
        files.insert_bytes("main.m68k", vec![0xff]);
        let binary = match expand(&files, "main.m68k") {
            Err(diagnostic) => diagnostic,
            Ok(_) => panic!("an entry file that holds bytes"),
        };
        assert_eq!(binary.code(), "unreadable_file");
        assert!(binary.message().contains("holds bytes"));
    }

    #[test]
    fn a_file_included_twice_is_parsed_and_reported_once() {
        let files = project(&[
            ("main.m68k", "    include lib.m68k\n    include lib.m68k\n"),
            ("lib.m68k", "    move.l #'ab,d0\n"),
        ]);
        assert_eq!(codes(&files, "main.m68k"), vec!["unterminated_string"]);
    }

    #[test]
    fn the_two_include_lines_of_a_file_included_twice_are_found() {
        let files = project(&[
            ("main.m68k", "    include lib.m68k\n    include lib.m68k\n"),
            ("lib.m68k", "x   equ 1\n"),
        ]);
        let (unit, _) = expand(&files, "main.m68k").expect("an entry file");
        let (first, second) = unit.included_twice(1, 3).expect("two include lines");
        assert_eq!((first.file.as_str(), first.line), ("main.m68k", 0));
        assert_eq!((second.file.as_str(), second.line), ("main.m68k", 1));
        assert_eq!(
            unit.included_twice(0, 2),
            None,
            "two positions of different files"
        );
    }
}
