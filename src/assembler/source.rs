//! The source Files an assembly is made of, and the places inside them.
//!
//! Three things live here, and every other module of the Assembler rests on
//! them:
//!
//! * [`Files`], the Project the Assembler is given: a map from a root-relative
//!   path to a [`FileContent`], text or bytes.
//! * [`Span`], a byte range inside **one** Source line. The tokenizer and the
//!   parser work in spans, because nothing in this language crosses a line.
//! * [`Location`], the place a [`Diagnostic`](super::diagnostics::Diagnostic)
//!   points at: a File, a 0-based line and a 0-based half-open range of
//!   columns. A Location counts *characters*, not bytes, so a diagnostic lands
//!   where the editor draws it.
//!
//! [`SourceFile`] ties the two together: it splits a text File into lines
//! (`\n`, `\r\n`, and a last line with no terminator at all) and turns a Span
//! on one of them into a Location.

use std::collections::BTreeMap;

use serde::Serialize;

/// The path the Entry file gets when the Assembler is handed a bare string
/// instead of a Project (the design record, "Files, `include`, `incbin`").
pub const DEFAULT_ENTRY_PATH: &str = "main.m68k";

/// A byte range inside one Source line, `start` included and `end` excluded.
///
/// Spans are byte offsets because that is what the tokenizer walks; they become
/// character columns only when a [`Location`] is made of them, which is the one
/// place the difference matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Span {
    /// Offset of the first byte of the span, from the start of the line.
    pub start: usize,
    /// Offset one past the last byte of the span.
    pub end: usize,
}

impl Span {
    /// The span from `start` to `end`. `end` is clamped up to `start`, so a
    /// span is never backwards.
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end: if end < start { start } else { end },
        }
    }

    /// The empty span at `offset`, which is what a diagnostic about something
    /// that is *missing* points at.
    pub const fn empty(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// How many bytes the span covers.
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the span covers no bytes at all.
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both, which is how a node's span is built
    /// from the spans of its parts.
    pub fn join(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// Whether `offset` falls inside the span.
    pub const fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// The text the span covers in `line`.
    ///
    /// Total: a span that runs past the end of the line, or that does not fall
    /// on character boundaries, gives back what it can rather than panicking,
    /// because a diagnostic is never worth a crash.
    pub fn text<'a>(&self, line: &'a str) -> &'a str {
        let start = clamp_to_boundary(line, self.start);
        let end = clamp_to_boundary(line, self.end).max(start);
        &line[start..end]
    }
}

/// Round `offset` down to a character boundary of `line`, and into the line.
fn clamp_to_boundary(line: &str, offset: usize) -> usize {
    let mut offset = offset.min(line.len());
    while offset > 0 && !line.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Where in the source something is: a File, a line, and a range of columns.
///
/// `line` is 0-based and counts every line of the File, blank and Comment lines
/// included, which is the convention the fixtures use
/// (`tests/corpus/README.md`, "Conventions"). `column` is 0-based and counts
/// **characters**, so a tab is one column wide (`docs/grammar.md` 1.1) and a
/// Latin-1 letter is one column wide whatever its UTF-8 length.
/// `end_column` is exclusive, so a Location covering nothing has
/// `column == end_column`.
///
/// It is serialised **camelCase** — `{ file, line, column, endColumn }` — which
/// is the shape the TypeScript side receives and the fixtures record (the
/// design record, "Public API"). It is the one shape: a Location inside a
/// Diagnostic, an instruction, an undo step or a call-stack frame is written
/// the same way everywhere.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// The root-relative path of the File this is in.
    pub file: String,
    /// 0-based index of the Source line.
    pub line: usize,
    /// 0-based column of the first character.
    pub column: usize,
    /// 0-based column one past the last character.
    pub end_column: usize,
}

impl Location {
    /// A Location from its four parts. `end_column` is clamped up to `column`.
    pub fn new(file: impl Into<String>, line: usize, column: usize, end_column: usize) -> Self {
        Self {
            file: file.into(),
            line,
            column,
            end_column: end_column.max(column),
        }
    }

    /// The Location of `span` on the line `line_text`, the `line_index`th line
    /// of `file`.
    ///
    /// This is the one conversion from bytes to columns; every diagnostic goes
    /// through it rather than counting columns itself.
    pub fn from_span(
        file: impl Into<String>,
        line_index: usize,
        line_text: &str,
        span: Span,
    ) -> Self {
        let (column, end_column) = columns_of(line_text, span);
        Self {
            file: file.into(),
            line: line_index,
            column,
            end_column,
        }
    }

    /// The Location of a whole line, which is where a diagnostic about the line
    /// as a whole ("this macro definition is never closed") points.
    pub fn whole_line(file: impl Into<String>, line_index: usize, line_text: &str) -> Self {
        Self::from_span(file, line_index, line_text, Span::new(0, line_text.len()))
    }
}

/// The character columns `span` covers on `line`: the first, and one past the
/// last.
///
/// The same conversion a [`Location`] does, for the callers that have a line
/// but no File to name it — `parseLine`, which reads one line on its own.
pub fn columns_of(line: &str, span: Span) -> (usize, usize) {
    let start = column_of(line, span.start);
    (start, column_of(line, span.end).max(start))
}

/// How many characters of `line` come before the byte `offset`.
///
/// Total, like [`Span::text`]: an offset past the end of the line gives the
/// length of the line in characters, and an offset inside a character counts
/// that character as not yet reached.
fn column_of(line: &str, offset: usize) -> usize {
    let mut column = 0;
    for (index, _) in line.char_indices() {
        if index >= offset {
            return column;
        }
        column += 1;
    }
    column
}

/// What one File of a Project holds.
///
/// Text Files are what `include` reads and what the Assembler assembles; byte
/// Files are what `incbin` reads. A text File can also be read by `incbin`,
/// which contributes its Latin-1 bytes ([ADR
/// 0004](../../../docs/adr/0004-characters-are-latin-1-bytes.md)); the
/// conversion is the `incbin` Directive's, in phase 4, and not this module's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    /// A source File, as text.
    Text(String),
    /// A binary File, as bytes.
    Bytes(Vec<u8>),
}

impl FileContent {
    /// Whether this is a text File.
    pub fn is_text(&self) -> bool {
        matches!(self, FileContent::Text(_))
    }

    /// The text of a text File, or `None` for a binary one, which is how
    /// `include` tells the two apart before pointing at `incbin`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            FileContent::Text(text) => Some(text),
            FileContent::Bytes(_) => None,
        }
    }

    /// The bytes of a binary File, or `None` for a text one, whose bytes are a
    /// Latin-1 encoding rather than a slice of the stored `String`.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            FileContent::Text(_) => None,
            FileContent::Bytes(bytes) => Some(bytes),
        }
    }
}

/// The Files of one Project: everything the Assembler is allowed to read.
///
/// Paths are root-relative, with `/` separators, the same notion as the
/// asm-editor's File. A `\` is normalised to `/` and `.` segments are dropped
/// when a path goes in or is looked up ([`normalise_path`]), so a program that
/// writes `include "..\lib\io.x68"` finds `../lib/io.x68`. Paths are kept
/// sorted, which is what makes "the closest existing paths" of a missing
/// `include` reproducible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    files: BTreeMap<String, FileContent>,
}

impl Files {
    /// An empty Project.
    pub fn new() -> Self {
        Self::default()
    }

    /// The Project a bare source string makes: one text File at
    /// [`DEFAULT_ENTRY_PATH`], which is the 2.0 API's "a single string wraps as
    /// `main.m68k`".
    pub fn from_source(source: impl Into<String>) -> Self {
        let mut files = Self::new();
        files.insert_text(DEFAULT_ENTRY_PATH, source);
        files
    }

    /// Add a text File, replacing whatever was at that path.
    pub fn insert_text(&mut self, path: &str, text: impl Into<String>) -> Option<FileContent> {
        self.files
            .insert(normalise_path(path), FileContent::Text(text.into()))
    }

    /// Add a binary File, replacing whatever was at that path.
    pub fn insert_bytes(&mut self, path: &str, bytes: Vec<u8>) -> Option<FileContent> {
        self.files
            .insert(normalise_path(path), FileContent::Bytes(bytes))
    }

    /// The File at `path`, if the Project has one.
    pub fn get(&self, path: &str) -> Option<&FileContent> {
        self.files.get(&normalise_path(path))
    }

    /// The text of the File at `path`, if it is there and is text.
    pub fn text(&self, path: &str) -> Option<&str> {
        self.get(path).and_then(FileContent::as_text)
    }

    /// Whether the Project has a File at `path`.
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(&normalise_path(path))
    }

    /// Every path of the Project, in sorted order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// How many Files the Project has.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the Project has no Files at all.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// The canonical spelling of a path: `\` becomes `/`, empty and `.` segments
/// are dropped, and no leading `/` survives.
///
/// `..` segments are **kept**: resolving a path against the including File's
/// directory is the `include` Directive's business (phase 4), and this function
/// only settles how the same File is spelled twice.
pub fn normalise_path(path: &str) -> String {
    let mut normalised = String::with_capacity(path.len());
    for segment in path.split(['/', '\\']) {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if !normalised.is_empty() {
            normalised.push('/');
        }
        normalised.push_str(segment);
    }
    normalised
}

/// The byte spans of the lines of `text`, terminators excluded.
///
/// A line ends at `\n`, at `\r\n`, or at the end of the text; a File whose last
/// line has no terminator still has that line, which
/// `tests/corpus/editor/bad-apple.x68` needs. A File that *does* end in a
/// terminator has no extra empty line after it.
pub fn split_lines(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            lines.push(Span::new(start, end));
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    if start < bytes.len() {
        lines.push(Span::new(start, bytes.len()));
    }
    lines
}

/// One text File, split into lines and ready to be read line by line.
///
/// It borrows the text of a [`Files`] entry rather than copying it, which
/// matters for `tests/corpus/editor/bad-apple.x68` and its 3.3 MB.
#[derive(Debug, Clone)]
pub struct SourceFile<'a> {
    path: &'a str,
    text: &'a str,
    lines: Vec<Span>,
}

impl<'a> SourceFile<'a> {
    /// Split `text` into lines and remember which File it is.
    pub fn new(path: &'a str, text: &'a str) -> Self {
        Self {
            path,
            text,
            lines: split_lines(text),
        }
    }

    /// The root-relative path of the File.
    pub fn path(&self) -> &'a str {
        self.path
    }

    /// The whole text of the File.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// How many lines the File has.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The `index`th line, without its terminator.
    pub fn line(&self, index: usize) -> Option<&'a str> {
        self.lines.get(index).map(|span| span.text(self.text))
    }

    /// Every line, with its 0-based index.
    pub fn lines(&self) -> impl Iterator<Item = (usize, &'a str)> + '_ {
        let text = self.text;
        self.lines
            .iter()
            .enumerate()
            .map(move |(index, span)| (index, span.text(text)))
    }

    /// The Location of `span` on the `line`th line of this File.
    ///
    /// A line index the File does not have gives a Location on that line with
    /// no columns, which keeps the conversion total.
    pub fn location(&self, line: usize, span: Span) -> Location {
        match self.line(line) {
            Some(text) => Location::from_span(self.path, line, text, span),
            None => Location::new(self.path, line, 0, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_texts(text: &str) -> Vec<&str> {
        split_lines(text)
            .into_iter()
            .map(|span| span.text(text))
            .collect()
    }

    #[test]
    fn line_end_splits_on_lf() {
        assert_eq!(line_texts("a\nb\nc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn line_end_splits_on_crlf() {
        assert_eq!(line_texts("a\r\nb\r\n"), vec!["a", "b"]);
        assert_eq!(line_texts("\r\n"), vec![""]);
    }

    #[test]
    fn line_end_keeps_a_last_line_without_a_terminator() {
        assert_eq!(line_texts("a\nb"), vec!["a", "b"]);
        assert_eq!(line_texts("dc.b 1"), vec!["dc.b 1"]);
    }

    #[test]
    fn line_end_adds_no_empty_line_after_a_final_terminator() {
        assert_eq!(line_texts("a\n"), vec!["a"]);
        assert_eq!(line_texts("a\n\n"), vec!["a", ""]);
        assert!(line_texts("").is_empty());
    }

    #[test]
    fn line_end_leaves_a_lone_carriage_return_in_the_line() {
        // A line terminator is LF or CRLF; a lone CR is an ordinary character
        // and the tokenizer answers for it (`unexpected_character`).
        assert_eq!(line_texts("a\rb\n"), vec!["a\rb"]);
    }

    #[test]
    fn span_text_is_total() {
        let line = "move.l d0,d1";
        assert_eq!(Span::new(0, 4).text(line), "move");
        assert_eq!(Span::new(7, 200).text(line), "d0,d1");
        assert_eq!(Span::new(200, 300).text(line), "");
        assert_eq!(Span::empty(4).text(line), "");
    }

    #[test]
    fn span_join_covers_both() {
        assert_eq!(Span::new(2, 4).join(Span::new(8, 9)), Span::new(2, 9));
        assert_eq!(Span::new(8, 9).join(Span::new(2, 4)), Span::new(2, 9));
    }

    #[test]
    fn a_column_counts_characters_and_not_bytes() {
        // "città" is six characters and seven bytes; the `,` after it is at
        // column 6 and byte 7.
        let line = "dc.b 'città',0";
        let comma = Span::new(13, 14);
        assert_eq!(comma.text(line), ",");
        let location = Location::from_span("main.m68k", 3, line, comma);
        assert_eq!(location.column, 12);
        assert_eq!(location.end_column, 13);
    }

    #[test]
    fn a_tab_is_one_column_wide() {
        let line = "\t\tmove.l d0,d1";
        let location = Location::from_span("main.m68k", 0, line, Span::new(2, 8));
        assert_eq!((location.column, location.end_column), (2, 8));
    }

    #[test]
    fn a_location_out_of_the_line_is_clamped() {
        let line = "nop";
        let location = Location::from_span("main.m68k", 0, line, Span::new(10, 20));
        assert_eq!((location.column, location.end_column), (3, 3));
    }

    #[test]
    fn whole_line_covers_every_column() {
        let location = Location::whole_line("main.m68k", 7, "  bra .loop");
        assert_eq!(
            (location.line, location.column, location.end_column),
            (7, 0, 11)
        );
    }

    #[test]
    fn source_file_reads_lines_and_locations() {
        let file = SourceFile::new("lib/io.x68", "start:\n    nop\n");
        assert_eq!(file.line_count(), 2);
        assert_eq!(file.line(1), Some("    nop"));
        assert_eq!(file.line(2), None);
        let location = file.location(1, Span::new(4, 7));
        assert_eq!(location, Location::new("lib/io.x68", 1, 4, 7));
        assert_eq!(
            file.lines().collect::<Vec<_>>(),
            vec![(0, "start:"), (1, "    nop")]
        );
    }

    #[test]
    fn a_location_on_a_line_the_file_does_not_have_is_still_a_location() {
        let file = SourceFile::new("main.m68k", "nop\n");
        assert_eq!(
            file.location(9, Span::new(0, 3)),
            Location::new("main.m68k", 9, 0, 0)
        );
    }

    #[test]
    fn files_hold_text_and_bytes() {
        let mut files = Files::new();
        files.insert_text("main.m68k", "nop\n");
        files.insert_bytes("data/sprite.bin", vec![1, 2, 3]);
        assert_eq!(files.text("main.m68k"), Some("nop\n"));
        assert_eq!(files.text("data/sprite.bin"), None);
        assert_eq!(
            files.get("data/sprite.bin").and_then(FileContent::as_bytes),
            Some(&[1, 2, 3][..])
        );
        assert!(files.contains("main.m68k"));
        assert!(!files.contains("missing.x68"));
        assert_eq!(
            files.paths().collect::<Vec<_>>(),
            vec!["data/sprite.bin", "main.m68k"]
        );
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn a_bare_source_string_wraps_as_the_default_entry_file() {
        let files = Files::from_source("nop\n");
        assert_eq!(files.len(), 1);
        assert_eq!(files.text(DEFAULT_ENTRY_PATH), Some("nop\n"));
    }

    #[test]
    fn a_path_is_normalised_on_the_way_in_and_on_lookup() {
        assert_eq!(normalise_path("..\\lib\\io.x68"), "../lib/io.x68");
        assert_eq!(normalise_path("/a//b/./c"), "a/b/c");
        assert_eq!(normalise_path("main.m68k"), "main.m68k");
        let mut files = Files::new();
        files.insert_text("lib\\io.x68", "nop\n");
        assert!(files.contains("lib/io.x68"));
        assert!(files.contains("./lib/io.x68"));
        assert_eq!(files.paths().collect::<Vec<_>>(), vec!["lib/io.x68"]);
    }
}
