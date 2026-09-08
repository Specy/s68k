# The assembler rewrite

Design record of the interview held on 2026-09-07 for [s68k#8](https://github.com/Specy/s68k/issues/8) (redo the lexer and parser), [asm-editor#15](https://github.com/Specy/asm-editor/issues/15) (labels without colon), [asm-editor#65](https://github.com/Specy/asm-editor/issues/65) (SR and CCR) and [asm-editor#63](https://github.com/Specy/asm-editor/issues/63) (`movep`), tracked by [asm-editor#77](https://github.com/Specy/asm-editor/issues/77) and feeding the multi-file step of [asm-editor#76](https://github.com/Specy/asm-editor/issues/76). Terms are the glossary's ([CONTEXT.md](../../CONTEXT.md)). The four decisions that are hard to reverse are ADRs ([0001](../adr/0001-easy68k-is-the-reference-dialect.md), [0002](../adr/0002-hand-written-parser-with-a-grammar-document.md), [0003](../adr/0003-operands-are-parsed-independently-of-the-instruction.md), [0004](../adr/0004-characters-are-latin-1-bytes.md)); everything else is recorded here and can change.

## Goal

Replace the regex lexer, the semantic checker and the compiler with one Assembler that follows EASy68K, explains every rejection in terms of what would have been right, assembles a Project of several Files, and reaches the asm-editor as `@specy/s68k` 2.0. The diagnostics are the product; the emulator stays a teaching tool.

## Why now

Verified on 1.4.2 with a probe file: a label without a colon in column 1 is "Unknown instruction"; EASy68K's bare comment field is glued into the operand (`d0traptask23`); `x equ 5` rewrites `next` into `ne5t`; `bra.s` is "Unknown size"; `move sr,d2` is "Invalid absolute"; `*` as the current address is taken as a comment; `end` is an unknown instruction; the compiler stops at its first error and reports it as a bare string. The checker and the compiler keep separate instruction lists that already disagree (`extb`).

## Scope

In this plan: the source-line grammar, symbols and expressions, diagnostics, layout, the directives and instructions below, `include` and `incbin`, the 2.0 API, and the tests that prove it.

Out, each with a "not implemented" diagnostic that names the feature: macros and conditional assembly (maybe a later milestone), structured control (left out for good, the diagnostic can say what to write instead), `memory`, `rte`, `stop`, `reset`, `move usp`. Real instruction sizes are a later, separate decision; the front end stores a size per instruction so that it does not have to be reopened.

## Decisions

### Dialect (ADR 0001)

EASy68K is the reference. An EASy68K program assembles unchanged as far as implemented features go; s68k keeps its leniencies (instructions in column 1, spaces beside commas, default origin `$1000`, a label named `START` as fallback entry point). Where s68k is stricter it is listed in the ADR: overlap is an error, `end` in an included file is an error, `equ` no longer aliases arbitrary text.

### Source line

- **Label**: an identifier followed by `:` anywhere; or an identifier in column 1 that is not a mnemonic, directive or (later) macro name. A word with a size suffix is never a label. An indented unknown word gets the hint "start it in column 1 or end it with a colon if it is a label"; a label named like a mnemonic needs the colon.
- **Operand field** ends at whitespace not adjacent to a comma, at `;`, or at the end of the line. Whitespace inside parentheses and quotes is allowed.
- **Comment**: a line starting with `*` or `;`; `;` anywhere; `*` after a complete operand field; and EASy68K's bare comment after the operand field, flagged once per file as a `suggestion` to use `;`. A space *before* a comma gets its own hint (almost always a broken operand list; a comment field cannot begin with a comma at all, which is why the hint moved — see the implementation notes, step 1). `*` where an expression term can stand is the current address or multiplication, so `lea *,a0` and `org (*+1)&-2` work; `#2 * 3` ends the operand at `#2` and a warning says that expressions cannot contain spaces.
- Mnemonics and directives are case insensitive; symbols are case sensitive.

### Parser (ADR 0002)

Hand-written tokenizer and recursive-descent parser, Pratt loop for expressions, a span on every token and node, error recovery per line. `docs/grammar.md` is the EBNF the parser follows and the tests are named after.

### Symbols and expressions

- Four kinds: Label, Constant (`equ`), Variable (`set`), Register list (`reg`). Redefinition of a Label or Constant is an error pointing at both definitions.
- Forward references allowed in instruction operands and `dc` data; refused where the value decides the layout (`org`, `ds`, `dcb` counts, `equ`, `set`), with a specific error.
- Local labels `.name` scoped between global labels, as EASy68K.
- EASy68K operators verbatim: unary `-`, `~`, `*`; binary `+ - * / \ & ! | ^ << >>`; precedence highest to lowest: `<< >>`, then `& ! | ^`, then `* / \`, then `+ -`. `**` is dropped (in EASy68K `2**3` is `2 * (current address) * 3`).
- Character literals up to four characters, warning beyond; `''` inside quotes is a literal quote. Values evaluate in 64 bits and are range-checked against the operand size.

### Diagnostics (ADR 0003)

- One `Diagnostic` for every assembly phase: severity, kind (a Rust enum serialised as a stable snake-case `code`), Location (file, line, column, end column), message, optional hint, related Locations.
- Assembly continues after an error as far as it sensibly can; a student sees every error of a build.
- Operands are parsed independently of the instruction; the analyzer compares them with the instruction table and names what is allowed and what was probably meant. Unknown mnemonics get a did-you-mean from the table, the label hint, or the reason a real feature is not implemented.
- `warning`: odd `org` (rounded up), character literal over four characters, constant over 32 bits, code after `end`, `end` without an address, an expression split by a space (`#2 * 3`). `suggestion`: bare comment (once per file), a double-quoted string (once per file), a space before a comma, a bare number below the program's origin where `#` was probably meant. Runtime errors stay separate. *(The last three of those changed while `docs/grammar.md` was written and reviewed; the implementation notes say why each moved, and they are the one part of this section not yet ratified by the owner.)*

### Directives

| Bucket | Directives |
|---|---|
| Implement with EASy68K meaning | `end`, `set`, `reg`, `fail`, `simhalt`, `include`, `incbin`, then `offset` and `section` last |
| Accept and ignore | `opt`, `list`, `nolist`, `page` |
| Refuse with a diagnostic that names the feature | `memory`, `macro`/`endm`/`mexit`, `ifxx`/`endc`, the structured control keywords |

`end expr` sets the Entry point; lines after it are ignored with one warning on the first non-blank line. Without `end`: a label named `START`, else the first instruction; no diagnostic for a missing `end`. No `even`/`cnop`: `ds.w 0` is the EASy68K idiom.

### Layout

`org` may go anywhere, including backwards. Two lines laid out over the same address are an error at the second with the first as related location. Word and long `dc`, `ds`, `dcb` and instructions align to even addresses; an odd `org` warns and rounds up. Instructions stay 4 bytes each, stepped by a per-instruction size field rather than a literal 4. Default origin `$1000`.

### Instructions

- SR is a 16-bit register whose low byte is the CCR; the high byte (trace, supervisor, interrupt mask) is stored and readable but has no effect, the program always running as supervisor as in EASy68K. `move` to and from SR and CCR, `andi`/`ori`/`eori` to both.
- `movep` as the byte-interleaved transfer it is.
- `addx`, `subx`, `negx`, `roxl`, `roxr`, `abcd`, `sbcd`, `nbcd`, `tas`, `rtr`; `chk`, `trapv`, `illegal` end the run with an exception as address errors do.
- Addressing modes: PC-relative displacement and index; `.w`/`.l` on absolute addresses; `.s`/`.w`/`.l` on branches, accepted without a range check until real sizes exist. *(Completed in the review step: `.b` is accepted on a branch too and means exactly what `.s` means — "EASy68K will accept .B or .S to force 1-byte offsets and .W or .L to force 2-byte offsets", `Reference/68ks9b.htm`. This section was silent on `.b`, and the rule for silence is to follow EASy68K.)*
- One instruction table for mnemonics, operand rules, sizes and defaults, shared by parser, analyzer and encoder.

**This section is finished (step 15).** Everything it asks for is implemented: the status register and `movep` in step 13, the extend-flag and binary-coded-decimal group in step 14, and the Addressing modes in step 15, which is the last of them. Two things the bullets above could not decide and step 15 did, both recorded in the implementation notes and in `README.md`: a PC-relative Operand is written as the address it reaches and **stored as the distance from the instruction's own extension word to it**, which the Interpreter adds back, because a fixed four-byte instruction has no encoded displacement to read; and `label.w` and `label.l` name the **same address**, `.l` saying nothing and `.w` being checked against the sixteen bits an absolute short reference holds. What is left of the design record for the instructions is one thing it always deferred, "Real instruction sizes are a later, separate decision" (Scope), and the four Mnemonics and the one register refused for good.

### Files, `include`, `incbin`

- Input: the Project's Files, a map from root-relative path to text or bytes, plus the Entry file's path. A single string wraps as `main.m68k`.
- `include` is textual: same section and address, one symbol namespace, local-label scopes running across the boundary. Paths resolve relative to the including File's directory, then the project root; `\` is normalised to `/`; quotes optional. A miss lists the closest existing paths. Cycles are an error showing the chain; a depth limit is the backstop. Including a File twice is allowed and the duplicate-symbol errors it causes say so. `include` of a binary File is an error pointing at `incbin`.
- `incbin` inserts a File's bytes at the current address; a text File contributes its Latin-1 bytes.
- Every Location carries its File; a diagnostic in an included File carries the Include chain as related Locations. Breakpoints, the current line, undo steps and call-stack frames are (file, line).

### Characters (ADR 0004)

One byte in Latin-1 everywhere; a source character above 255 is an assembly error; terminal decoding is total.

### Public API, `@specy/s68k` 2.0

- `S68k.assemble(source, options?)`, `source` a string or `{ files, entry }`, returns `{ diagnostics, program? }`; `program` present only without errors. Live checking calls it and ignores `program`. `new Interpreter(program, options)` stays separate.
- Diagnostics are plain objects serialised once; the `SemanticError` class goes.
- `getCurrentLocation()`, breakpoints as `{ file, line }[]`, an instruction as `{ address, size, location, source }`.
- `parseLine(text)` replaces `lexOne`: kind, mnemonic, size, and each operand with its addressing mode and span, so the editor can explain the operand under the cursor.
- Released in lockstep with the asm-editor change; Cargo version aligned to 2.0.0.

**As shipped (step 9).** The plan above is what was built; four things it did not
say, decided while building and recorded here so that this section is the whole
contract: `assemble` takes a second argument `{ entry? }`, which names the File a
bare source string is filed under (`main.m68k` by default) and overrides a
project's own `entry`; the Program crosses as a **handle**, not as data —
`program.getInfo()` answers `{ entryPoint, endAddress, instructionCount, symbols }`
and the instructions stay on the Rust side, where the Interpreter looks them up;
`Program` and `Interpreter` therefore have a `dispose()`, and `assemble` frees the
handle itself when there is no Program to keep, so live checking leaks nothing; and
the Assembler's shapes are camelCase (`endColumn`, `entryPoint`) while the
Interpreter's 1.4.2 shapes keep their snake_case field names, because 2.0 changes
its surface only where a line index became a Location. The reasons are in step 9's
implementation notes.

**This section is finished (step 17).** Everything it asks for is implemented and
the surface is closed. Phase 4 changed it in **one** place, `InstructionLine`
gaining `includeChain: Location[]` (step 16), and step 17 added no field at all:
it made the input side say what it already accepted — `wasm_assemble`'s first
argument is declared `SourceFiles = Record<string, string | Uint8Array>` in the
generated `.d.ts` instead of `any` — wrote the four cases of a Project into the
smoke test, and made the command line read a Project rather than a File. The one
thing the third bullet above understates is the instruction, which is
`{ address, size, location, includeChain, source }` and not the four fields it
names; the fourth bullet's `parseLine` is as written. What a Program still does
not carry is the list of Files a build read: the chains on the instructions are
the only trace, and the reverse question ("which Files did this build read?") is
a later step's to add.

### Tests

1. Golden fixtures of 1.4.2's output for the 30 editor programs (24 lecture playgrounds, 6 runnable `.x68`; all 30 assemble on 1.4.2, baseline run on 2026-09-07), updated only on purpose with a note.
2. Golden diagnostics for the 3 EASy68K originals: only the "not implemented" errors for macros, structured control and the unsupported traps.
3. One snapshot case per diagnostic kind (`insta`), EASy68K's error catalogue checked off against ours.
4. Grammar tests named after the EBNF rules, including the label and comment tables.
5. Interpreter tests per new instruction with the manual's flag semantics.
6. The TypeScript smoke test grows a multi-file case and a diagnostics case; CI runs everything.

## Work plan

Everything is built in one go on a single branch and merged once, as 2.0.0; the phases below are the order of work, not release points. Phase 0 comes first so that the fixtures are taken from 1.4.2 before anything changes.

0. Safety net: fixtures, corpus harness, snapshot setup.
1. New front end, single file, with the file-carrying Location and the 2.0 API shape from day one; old lexer, checker and compiler deleted at the end.
2. Directives.
3. Instructions.
4. `include`, `incbin`, Files input, runtime Locations, smoke tests, release; then the asm-editor PR.

Maybe later: macros with conditional assembly as a milestone of their own.

## Consequences for the asm-editor

Per-file breakpoints and current line; diagnostics with columns, hints and related locations; hover through `parseLine`; the terminal decoding bytes as Latin-1 and refusing characters above 255; binary Files (base64) for `incbin`; documentation of the new directives and instructions; the examples README no longer saying the EASy68K originals cannot assemble here beyond their macros and structured control.

## Implementation notes (phase 1)

A running record of the work of phase 1, one bullet a choice, so that the next step can read the state of it from the repository. Nothing above this heading is rewritten except to fix a factual error, and such a fix says so here.

### Step 1 — `docs/grammar.md`, the parser's specification (ADR 0002)

- `docs/grammar.md` is written and is the specification the tokenizer and parser follow. It has six sections: lexical rules, the EBNF, the resolved ambiguities, the parser's diagnostics, the index of every rule name, and what the corpus was checked against. No code was written in this step.
- **Rule names are the test names.** Section 5 of the document is the index; a parser test is named after the rule it exercises (`label_rule`, `operand_field_extent`, `pc_index`, `shift_expression`, …).
- **Module layout**: unchanged from the plan. The document points at `src/assembler/token.rs` for the token kinds it lists and at `src/assembler/ast.rs` for `Line { label, operation, comment, bare_comment }`.
- **Identifiers hold no internal `.`** (EASy68K's own rule: a global name is letters, digits and underscores, a local name is a dot and then the same). That is what makes the size suffix decidable without a table: the disposition of a `.` is positional — directly after an identifier, number, `)` or quoted literal it opens a `size_suffix`, anywhere else it opens a local label. So `dc.b` is one Operation, `.l` in column 1 is the Local label `.l`, and `move.q` is `unknown_size_suffix` rather than a cascade about a symbol `q`.
- **`_` may start a name.** EASy68K's prose says a global label "should start with a letter"; accepting `_start` costs nothing and is a shape students write.
- **`(expr)` is decided after the `)`.** An Operand beginning with `(` is an Addressing mode only when the token after the `(` is an address register or `pc` and the one after that is `)` or `,`; otherwise it is an Expression, and when the token after the closing `)` is a binary operator the Pratt loop carries on. Without that, `ORIGINX equ (640-COLS*SCALE)/2` (line 30 of `tests/corpus/editor/bad-apple.x68`) is lost. `parenthesised_operand` in the document is that decision procedure written out.
- **The `*` cost is paid in the analyzer.** `*` at the start of the Operand field is always the current address, because `org *` and `nop * comment` are the same shape and only the instruction table separates them, which ADR 0003 forbids the parser. The analyzer gets a specific diagnostic for it: an Operation that takes no Operands, given one `current_address` Operand, is told that the `*` was read as the current address and that `;` starts a comment. Phase 3 owes that message.
- **"A comment field beginning with a comma" is unreachable as written.** The Operand field ends at whitespace *not beside* a comma, symmetrically (design record and glossary both), so a comma right after the whitespace is exactly what keeps the field open and a bare Comment can never start with one. The hint the design record asks for is attached instead to the reachable form of the same mistake, a space *before* a comma: `space_before_comma`, a suggestion. It fires on the harmless `d0 ,d1` too, which is the price of catching `trap #15   , display Y`.
- **`#2 * 3` is a warning, not an error.** The design record says "the error says that expressions cannot contain spaces"; the sentence it asks for is the parser's `expression_split_by_space`, raised when the comment field's first token is a binary operator, at `warning` severity. In the shape that bites (`move.l #2 * 3,d0`) the *error* is already the analyzer's missing destination, and this warning is what explains it; at `error` severity the harmless `dc.b 1  * one byte` would stop building.
- **Double-quoted strings are accepted**, with a `double_quoted_string` suggestion once per File naming EASy68K's `'`. The help documents `'` for strings and both quotes for `include`/`incbin` filenames, and says nothing about `"` elsewhere; accepting it is the lenient-superset direction of ADR 0001 and refusing what a student who has written C will type teaches nothing. The once-per-File machinery is the same as `bare_comment`'s.
- **Two new lexical errors that EASy68K has no equivalent for**: `non_breaking_space` (a `$A0`, invisible, and what a paste from a web page leaves) and the typographic-look-alike hint on `character_above_latin1` (`‘ ’ “ ” – —` name the plain character to write). Both exist because the asm-editor is a web editor and the alternative diagnosis would be a lie.
- **No exponent operator, and no unary `+`.** EASy68K has neither. *(Corrected in step 2: this bullet first said that `2**3` "comes out as `2 * current_address * 3`, EASy68K's own reading, from the layered grammar with no rule of its own". It does not. The layered grammar consumes `2 * *` and then stops at the `3`, which is not an operator and which `primary_expression` cannot absorb, so the `3` is left over and the Operand is `unexpected_token_in_operand`. EASy68K's reading is why `**` is not exponentiation; it is not a reading s68k reproduces.)* `#+5` is `plus_is_not_a_unary_operator` with the hint "write `5`".
- **`;` beats an unclosed `(`.** A `;` ends the Operand field at any parenthesis depth, so `move.l (a0,d1  ; save it` reports the unclosed parenthesis rather than swallowing the comment.
- **`fail` takes the rest of the line raw** — EASy68K's `FAIL ERROR, Argument missing…` has commas in its message — and it is the only Operation whose Operand field ignores the end-of-field rule.
- **The refused Directives are not tokenized past their name.** `memory`, `macro`/`endm`/`mexit`, `ifxx`/`endc` and the structured-control keywords parse as `unimplemented_operation`, name plus a `raw_operand_field`, and the lines between a `macro` and its `endm` are skipped whole. That is what leaves one "not implemented" diagnostic on `if.l d1 <hs> #NOON then.s` and on the Macro body's `move.l #\1,d1`, which is what the three `tests/corpus/easy68k/` fixtures are meant to become.
- **`dc` with no size defaults to `.w`**, like `ds` and `dcb`. The help documents the default for those two and is silent for `dc`; every corpus program writes the size.
- **`code_after_end` warns only on a line carrying a Label or an Operation.** Blank and Comment lines after `end` never warn: all three EASy68K originals close with `END START` and then EASy68K's own `*~Font name~Courier New~` editor comments. The Directives phase owes that warning; it is not the parser's.
- **The twenty-one register names are reserved** (`d0`–`d7`, `a0`–`a7`, `sp`, `pc`, `sr`, `ccr`, `usp`): a Symbol may not be called one, error `reserved_name_as_symbol`. A `reg` Symbol standing in for a register list is *not* a grammar form — a bare identifier parses as `absolute` and becomes a Register list only when the Symbol is resolved, which is ADR 0003 applied.
- **Corpus check.** A throwaway model of the label rule, the end-of-field rule and the Operand and Expression grammar was run over all 33 corpus programs: every one of their 9672 lines split into the four fields and all 8711 Operand fields parsed. The document's section 6 lists what the run turned up and what the corpus does *not* exercise (Local labels, binary and octal numbers, `''`, double quotes, a space before a comma, `*` outside an Expression), which is the list of rules whose tests have to be written by hand.
- **Not done in this step**, and next: `src/assembler/` does not exist yet. The order the plan implies is `source.rs` and `diagnostics.rs` first (every other module needs `Location`, `Span` and `Diagnostic`), then `token.rs` and `tokenizer.rs`, then `ast.rs` and `parser.rs` against this grammar, with a test per rule name.

### Step 2 — `docs/grammar.md` under review

A review of step 1's document raised eleven findings; every one is answered in the document itself, and this is the list of the choices made. No code was written in this step either, and `docs/grammar.md` is still the only artefact.

- **The parser asks exactly two questions about a name, and both are written down.** The instruction table answers `label_rule`'s "is this word a Mnemonic or a Directive name?" (ADR 0003 unchanged); a second, *closed* list in the grammar document — `text_operation` = `fail`, `include`, `incbin`, and every refused keyword — answers "does this Operation take its Operand field as text rather than as an `operand_list`?". Section 2.4 said "and nothing else", which was wrong: 2.6 already required the parser to key on four groups of names. The second question has to be the parser's because the answer decides whether the text is tokenized at all, and no later phase can undo tokenizing: read as Operands, `include io.x68` takes `.x` for a size suffix and leaves `68` over.
- **§2.6's productions belong to the Directives phase.** The parser reads every line by 2.2–2.5 into one `Line` and never by 2.6's rules; the `[ label_field ]` each of them restates is a requirement phase 2 checks against that Line, not a second parse of the same text. Only `text_operation` is the parser's. This is now stated at the head of 2.6, which removes the two overlapping grammars for the same text.
- **`macro` … `endm` is the one line-range construct**, and §1.3's "no construct spans lines" now says so instead of being contradicted by 2.6. The parser enters the skip on an Operation named `macro`, leaves it on one named `endm`, tokenizes nothing in between, raises the single `unimplemented_operation` on the `macro` line, and, when no `endm` arrives, raises the new error `unterminated_macro_definition` — EASy68K's own "ERROR: ENDM expected" — rather than swallowing the rest of the File in silence.
- **`2**3` is rejected, not reinterpreted.** The claim that the layered grammar reproduced EASy68K's `2 * current_address * 3` was false: the grammar consumes `2 * *` and leaves the `3`, which no rule can absorb. The document now says the Operand is `unexpected_token_in_operand`, keeps EASy68K's reading only as the reason `**` is not exponentiation, and the step 1 bullet above is corrected in place.
- **New kind `unexpected_token_in_operand`** for a token left over after a complete Operand — the commonest parser failure, which had no message. It is EASy68K's "ERROR: Comma expected — The operand is not complete", and it covers `2**3`, `move.l d0),d1` (there is no unmatched-`)` kind of its own) and every other trailing token. It is deliberately *not* `malformed_operand`, whose promise is to name the shape the Operand tried to be.
- **A dotted name is one diagnostic, not a size plus junk.** The token for a `.` in suffix position now takes the whole run after the dot ("longest run, then diagnose", the convention `number` already used). In the operation field a run that is not `b`/`w`/`l`/`s` is `unknown_size_suffix` (`move.ll` included); in the Operand field a run of two or more characters is the new error `dot_in_name`, "a name holds no dot after its first character", so `array.length` is diagnosed once instead of becoming `.l` with `ength` left over.
- **Precedence between `malformed_operand` and `unclosed_parenthesis`** is stated in 2.5, 3.11 and under the table of 4: `malformed_operand` once an Addressing mode has been recognised (it can name the shape), `unclosed_parenthesis` only for a `(` that opened a grouped Expression with no mode recognised. §3.11's own example, `move.l (a0,d1  ; save it`, is therefore a `malformed_operand`, and 3.11 grew a second example for the other kind.
- **`expression_split_by_space` no longer fires on an EASy68K comment.** A `*` at the start of the comment field is the `explicit_comment` marker of 1.6, so "the comment field's first token is a `binary_operator`" would have told `move.l d0,d1  * copy` to start a comment with `;` when it already had. The trigger is now "a binary operator, then a *number or character literal*, then the end of the field, a `,`, or another operator", which keeps `#2 * 3` and `move.l #2 * 3,d0` and drops `* copy`, `* one byte` and `* 2 registers saved`. Re-measured on the corpus: 0 of 9672 lines have a comment field starting with a binary operator at all, so neither form fires on a fixture.
- **A register range may cross from `d7` into `a0`.** `d5-a2` was an error with no citation, which ADR 0001 would have required to be recorded as a deviation; the help says nothing, the `movem` mask is one contiguous field over `d0`…`d7`,`a0`…`a7`, and `tests/corpus/README.md` already specifies that the *printer* splits such a range, which only makes sense if one can be written. `register_range_crosses_register_kinds` is withdrawn and replaced by `register_range_out_of_order` (`d5-d2`, `a2-d5`), so the reachable mistake still has a name. Nothing is added to ADR 0001, because accepting is the lenient direction.
- **`double_quoted_string` excludes filenames.** `Directives/include.htm` and `incbin.htm` document both quote characters and their own examples are double quoted, so the suggestion is now about strings and character literals only. It cannot fire on a filename in any case, since a `file_specification` is read from raw text rather than tokenized.
- **The Latin-1 errors stop at the comment.** `character_above_latin1`, `non_breaking_space` and `unexpected_character` are raised nowhere inside a `comment_line` or a `comment_field` and everywhere else, quoted literals included. A pasted em-dash in a comment produces no byte; the same character in a `dc.b` string has to produce one and cannot. The help has no equivalent — EASy68K reads a byte at a time — so this is s68k's choice, recorded in 1.1.
- **`symbol_reference` excludes the twenty-one reserved names**, which is what 3.8 and 3.9 were already resting on, and a register where an Expression term must stand is the new error `register_in_expression` (`#a0+4`, `4+d0`). It is the parser-side neighbour of EASy68K's "Register list symbol used in an expression", which is about a `reg` Symbol and stays the evaluator's.
- **A `;` starts a Comment on every line of the language, without exception.** `message_text` (`fail`) and `raw_operand_field` (the refused keywords) ignore rules 1 and 4 of 1.5 — quotes and whitespace are ordinary characters in them — but keep rule 2, so they still end at a `;`. `include` and `incbin` keep all four, which is how a quoted path with spaces survives. That keeps the corpus's `if <cs> then.s   ; if set` with its Comment, and costs only a semicolon inside a `fail` message, about which the help is silent.
- **Two deferrals written down where they belong**: the forward-reference rule (allowed in instruction Operands and `dc` data, refused in `org`, `ds`, `dcb`, `equ`, `set`) is a note on 2.6 and the Directives phase's to raise, citing `Directives/org.htm`; Local-label *scope* (between Global labels) is a note on 1.7 and the symbol table's, 1.7 giving the syntax only.
- **Two corpus quotations corrected**: 1.5 cited `move.l (a0, d5),d6` where line 16 of `binary-search-1.asm` reads `move.w (a0, d5), d6`, and 2.7 cited `#COLS*2` where line 21 of `two-dimensional-array-1.asm` reads `add.l #COLS*2, a1`. Section 6 also records the re-measurement: with the text Operand fields in place 0 of the 8711 Operand fields fail, and without them exactly two lines do (`clockDigital.X68` line 22, a Macro body; `mouseWindowSize.X68` line 223, a structured-control line).
- **Three changed decisions are now visible in the record above** — `expression_split_by_space` as a warning rather than an error, `space_before_comma` in place of the unreachable "comment field starting with a comma", and the added `double_quoted_string` suggestion — with the "Diagnostics" list corrected in place and marked as awaiting the owner's ratification. **`docs/adr/0001` still carries the old comma-comment consequence** ("A comment field beginning with a comma gets its own hint"); an accepted ADR is not edited here, and it is the one thing this step leaves for the owner to settle along with the ratification.
- **Not done in this step**, and unchanged from step 1: `src/assembler/` still does not exist. The next step is `source.rs` and `diagnostics.rs`, then `token.rs` and `tokenizer.rs`, then `ast.rs` and `parser.rs`, with a test per rule name of section 5 — which now includes `text_operand_field`, `quoted_file_name`, `text_operation`, `refused_operation` and `macro_definition`, and five diagnostic kinds that did not exist before (`dot_in_name`, `unexpected_token_in_operand`, `register_in_expression`, `register_range_out_of_order`, `unterminated_macro_definition`).

### Step 3 — `src/assembler/`: the Files, the Diagnostics, the tokens and the tree

The first code of the rewrite. `src/assembler/` now holds `source.rs`,
`diagnostics.rs`, `token.rs`, `tokenizer.rs` and `ast.rs`, with 73 tests beside
them; `src/lib.rs` gained `pub mod assembler;` and nothing else. The old
pipeline (`lexer.rs`, `semantic_checker.rs`, `compiler.rs`) is untouched and its
35 tests, the corpus fixtures included, still pass unchanged — `cargo test` is
107 green — and no snapshot moved.

- **The module layout is the plan's**, name for name. `mod.rs` declares the five
  modules and documents the seven still to come, in the order phase 1 wants
  them: `parser.rs`, `expr.rs`, `symbols.rs`, `instructions/`, `analyzer.rs`,
  `layout.rs`, `program.rs`.
- **`assemble` and `parse_line` are not declared yet.** `mod.rs` says where they
  go and what they will return; a stub that answered nothing would be a lie in
  the one file a reader opens first, and the parser is what makes them possible.
- **`Span` is a byte range inside one line; a `Location` counts characters.**
  Nothing in this language crosses a line, so a span never needs a line number,
  and the one conversion, `Location::from_span`, is where bytes become columns
  (a tab is one column, an accented letter is one column, `docs/grammar.md`
  1.1). Both are total: a span past the end of its line, or off a character
  boundary, gives what it can rather than panicking, because no diagnostic is
  worth a crash.
- **`split_lines` gives spans, not strings**: LF and CRLF end a line, a last line
  with no terminator is still a line (`bad-apple.x68`), a File that ends in a
  terminator grows no empty line after it, and a lone CR stays inside its line,
  where the tokenizer answers for it with `unexpected_character`.
- **`Files` normalises a path on the way in and on lookup** — `\` to `/`, empty
  and `.` segments dropped, no leading `/` — and keeps `..` for phase 4's
  resolution to deal with. It is a `BTreeMap`, so `paths()` is sorted and "the
  closest existing paths" of a missing `include` will be reproducible.
  `Files::from_source` is the 2.0 API's "a single string wraps as `main.m68k`",
  spelled `DEFAULT_ENTRY_PATH`.
- **`SourceFile<'a>` borrows its text** rather than copying it; `bad-apple.x68`
  is 3.3 MB and is read once.
- **Severity belongs to the kind, not to the raiser.**
  `DiagnosticKind::severity()` answers it and `Diagnostic::new` takes it from
  there, so the table of `docs/grammar.md` section 4 is true of every Diagnostic
  carrying that code. `with_severity` exists for a case that has not turned up
  yet and says in its own doc comment that it is rarely right.
- **Thirty-six `DiagnosticKind` variants**: the twenty-five of the grammar's
  section 4, `number_too_large` (below), and the ten first analyzer kinds the
  design record asks for (`unknown_mnemonic`, `mnemonic_used_as_label`,
  `invalid_addressing_mode`, `invalid_size`, `immediate_out_of_range`,
  `unimplemented_operation`, `symbol_already_defined`, `undefined_symbol`,
  `forward_reference_not_allowed`, `division_by_zero`). Several are raised by
  nothing yet; they are written now so that the wording is decided once, against
  the help, rather than in the middle of the phase that needs them.
- **A code cannot be forgotten.** `code()`'s match has no catch-all arm, so a new
  variant does not compile until it is given a code, and `ALL_CODES` lists every
  code in declaration order while `every_kind_has_a_stable_code` compares it,
  element for element, with a table holding one sample of every variant.
- **The label-hint cases are one field and one kind.** The indented-word hint of
  `label_rule` row 8 is data on `unknown_mnemonic`
  (`{ name, suggestion, could_be_label }`), because it is the same diagnostic
  with a different sentence under it; row 5's is its own kind,
  `mnemonic_used_as_label` — "`clr` in column 1 is the instruction `clr`, not a
  label", hinted "write `clr:`" — and it is a **suggestion**, because the line's
  real error is already the analyzer's ("`clr` takes one operand") and ADR 0003
  keeps errors for what is actually wrong.
- **"Not implemented" is `unimplemented_operation`**, the grammar's name for it
  rather than the design record's prose, carrying `{ name, reason, alternative }`
  so that the message names the feature and the hint can say what to write
  instead.
- **`malformed_operand` names the shape from an enum**, `OperandShape`, which
  carries a description and a written example, so "this looks like an indexed
  operand, `4(a0,d1.w)`, but the `)` is missing" is built from the type and
  cannot drift back into "invalid syntax". `Operand::description()` in `ast.rs`
  is the same idea for "what was found".
- **Serialisation is one-way and flat**: `{ severity, code, message, hint,
  location, related }`, the message and the hint already rendered, `related` an
  array of `{ location, message }`. The `DiagnosticKind`'s own data is
  deliberately not serialised — the code and the message carry it — so the
  TypeScript side never sees a Rust enum's shape and a variant may gain a field
  without breaking it.
- **One new dev-dependency, `serde_json`**, already in the tree through insta's
  `json` feature: the serialised shape is asserted as data
  (`the_serialised_shape_is_flat`) rather than as a snapshot, which keeps the
  contract readable in the test. No runtime dependency was added.
- **The tokenizer is a cursor, not a pass.** `Tokenizer::next_token` walks one
  line, and `mark()`/`rewind()` undo the position **and** the Diagnostics raised
  since the mark. The parser needs that twice: the two-token lookahead of
  `parenthesised_operand`, and rule 4 of `operand_field_extent`, which has to
  look past a run of whitespace into text that may turn out to be a Comment
  field — and a Comment field is never tokenized (`docs/grammar.md` 1.6), so a
  `character_above_latin1` raised while peeking into one has to disappear. **The
  next step uses `mark`/`rewind` for every lookahead** and never a second
  tokenizer over the same text.
- **`tokenize_line` is for tests.** It runs the cursor to the end of the line,
  which over-tokenizes a bare Comment field; the parser drives the cursor itself
  and stops it where the Operand field ends. Both say so in their doc comments.
- **The tokenizer raises five Diagnostics and no others**:
  `character_above_latin1`, `non_breaking_space`, `unexpected_character`,
  `unterminated_string`, `invalid_number`. `unknown_size_suffix` and
  `dot_in_name` are the parser's, because they depend on the field the token
  stands in (1.11); `bare_comment` and `double_quoted_string` are the
  Assembler's, because "the first one in a File" is not something one line can
  know. The `QuoteKind` on the token is what lets it raise the second.
- **A broken token is still the token it tried to be**: a number with a bad digit
  is a `Number`, an unterminated literal is a `StringLiteral`, each with its
  Diagnostic beside it. Only a character that starts nothing at all becomes
  `Error`. That is what lets the parser finish the line and the analyzer add its
  own sentence, instead of a cascade.
- **The tokenizer decides the two lexical readings and no more**: a `;` starts a
  Comment anywhere, a `*` starts one when it is the first non-blank character of
  the line, and the disposition of a `.` follows adjacency to the previous token
  (an `Identifier`, `LocalIdentifier`, `Number`, `RightParen` or `StringLiteral`
  ending exactly where the `.` begins opens a `SizeSuffix`; anything else opens a
  `LocalIdentifier`). Everything else about a `*` is the parser's.
- **Each token carries `column_one` and `preceded_by_whitespace`**, the two facts
  `label_rule` and `operand_field_extent` turn on, so neither rule ever
  recomputes them from the text. `column_one` is true of the *first* token of the
  line, which on an indented line is the whitespace.
- **`ast.rs` follows the layout, with four deviations, each for a reason**:
  (a) `Operation` carries `text: Option<TextOperandField>` beside
  `operands: Vec<Operand>`, because `operand_field` has two alternatives
  (`docs/grammar.md` 2.5) and a `text_operation` fills the first while every
  other Operation fills the second; (b) the three special registers are one
  variant, `Operand::SpecialRegister { register: SpecialRegister }`, following
  2.5's single `special_register` alternative rather than three variants;
  (c) `Line::bare_comment` is kept although it mirrors
  `comment.kind == CommentKind::Bare`, because the layout asks for it and the
  once-per-File suggestion reads it; (d) an Operation's spans are named fields
  (`name_span`, `size_span`, `operand_field_span`, `span`) rather than a `spans`
  struct, which is what every node in the tree does.
- **`Register` is `{ kind, number, span }`** and `mask_index()` gives `movem`
  order (`d0`–`d7` 0 to 7, `a0`–`a7` 8 to 15), which is what
  `register_range_out_of_order` will compare and what makes a range crossing from
  `d7` into `a0` legal by construction. `sp` parses to `a7` and the spelling is
  not kept, because the printer never writes it back out.
- **`Expr::CharacterLiteral` holds Latin-1 `bytes`** and is the `dc` string node
  as well, because one token has two readings (1.9); `''` and `""` are read as
  one quote by `token::string_literal_bytes`, which is the only place that
  unescaping happens.
- **`!` and `|` are one operator**, `BinaryOperator::Or`, as the help has them;
  the spelling is recovered from the token's span when a message needs it.
  `BinaryOperator::precedence()` is EASy68K's four levels (`>> <<` 4, `& ! | ^`
  3, `* / \` 2, `+ -` 1) and is what the Pratt loop will run on;
  `from_token_kind` keeps the token-to-operator mapping in one place, for the
  loop and for the "an expression was expected after `+`" message alike.
- **The tests are named after the grammar's rules** — `label_rule_…`,
  `operand_field_extent_…`, `comment_rule_…`, `size_suffix_…`, `number_…`,
  `invalid_number_…`, `string_literal_…`, `character_set_…`, `case_rule_…`,
  `operator_…`, `punctuation_…`, `local_identifier_…` — so section 5 of the
  document and the test list can be read against each other. Every corner case
  the design record names is among them: the bare comment field, `''` inside a
  literal, `$ff`, `%1010`, `@17`, `'ab'`, and a lone `*` in the Operand field.
- **One test is a corpus pass**: `the_tokens_of_a_line_cover_it_exactly`
  tokenizes all 9672 lines of the 33 corpus programs and asserts the walk's one
  invariant — the tokens of a line are contiguous, none is empty, and together
  they are the line — which is also the proof that no real program makes the
  tokenizer loop or panic. It asserts nothing about the Diagnostics, because it
  tokenizes the Comment fields the parser never will. Its line count is the
  document's own (section 6), so `split_lines` and the throwaway model of step 1
  agree on what a line is.
- **Five changes to `docs/grammar.md`**, which ADR 0002 requires to come before
  the code and which are the whole of what this step could not tokenize as
  written:
  1. **1.1** said the three character errors are raised "everywhere else, a
     quoted literal included", which contradicts the rule in the same sentence:
     a no-break space and a control character *are* bytes (`$A0`, `$09`), so
     inside quotes they are ordinary characters and only
     `character_above_latin1`, the one character with no byte at all, is raised
     there. Outside quotes all three hold: a `dc.b` string holding a no-break
     space writes one byte and is not an error.
  2. **1.1** listed only "any other character below `$20`" as
     `unexpected_character`, while section 4's own example is `?`. The bullet now
     says what the code does: any character that starts no token, control
     characters, a lone `<` or `>`, and `=`, `` ` ``, `[`, `{` and their like.
  3. **1.7 and 1.11** were silent on a `.` with nothing after it. A `.` where a
     name may begin and with no name character after it is
     `unexpected_character`; a `.` in suffix position with an empty run is
     `unknown_size_suffix`, the message reading "`.` is not a size".
  4. **1.8** specified the "longest run, then diagnose" convention for prefixed
     numbers only. A `decimal_number` takes the same run, so `12ab` is one bad
     number rather than a number and a name — splitting it would report a missing
     comma instead of the mistake. And a literal that does not fit in the 64 bits
     values are computed in has no value to carry on with: the new error
     `number_too_large`, added to the table of section 4. EASy68K has no
     equivalent, working in 32 bits with a warning above them.
  5. **1.14** did not list `LocalIdentifier` among the token kinds, although 1.7
     requires the tokenizer to decide that a `.` opens one. It is now in the
     list, with the reason a decision already taken is not thrown away.
- **Not done in this step**, and next: `parser.rs`, one line to an `ast::Line`,
  against sections 2 and 3 of the document, with a test per rule name of section
  5 and per row of the tables of 1.4 and 1.6. The parser is where
  `unknown_size_suffix`, `dot_in_name`, `bare_comment`, `space_before_comma`,
  `double_quoted_string`, `expression_split_by_space` and the whole of section 4
  beyond the tokenizer's five are raised, and `assemble`/`parse_line` can be
  declared in `mod.rs` once it exists.

### Step 4 — `src/assembler/parser.rs`: the four fields, the Operands, the Expressions

The parser of `docs/grammar.md` sections 2 and 3, with a test per rule name of
section 5. `src/assembler/` gains `parser.rs` and `names.rs`; `mod.rs` gains the
2.0 API's `parse_line`; `ast.rs`, `token.rs` and `source.rs` gain the `Serialize`
derives the editor's hover needs. `cargo test` is 191 green (83 new: 79 in
`parser.rs`, 4 in `names.rs`), the corpus fixtures are untouched and no snapshot
moved.

- **A line is read in two halves, and that is the shape of the module.** The
  first half drives the `Tokenizer` over the line — `label_rule` (1.4), the
  Operation and its `size_suffix`, then the tokens of the Operand field,
  stopping exactly where `operand_field_extent` (1.5) says — and everything
  after that point is the Comment field, which is never tokenized (1.6). The
  second half walks the *collected tokens* with a small cursor (`peek`,
  `peek_at`, `bump`) and builds the Addressing modes and the Expressions. The
  split is what makes the Comment field unreachable from the Operand parser by
  construction, rather than by care: it is not in the tokens.
- **Every lookahead is `Tokenizer::mark`/`rewind`**, as step 3 required: the
  two-token lookahead of `label_rule`, the size-suffix peek, rule 4's peek past
  a whitespace run, and the Comment-field peek of `expression_split_by_space`.
  No second tokenizer is ever made over the same text.
- **The two diagnostic lists are merged at the end and sorted by column.** The
  tokenizer's stay in the tokenizer while the fields are being found — a rewind
  has to be able to drop what a lookahead raised, and a drained Diagnostic
  cannot be dropped — so `Parser::finish` appends them to the parser's own and
  sorts the whole by `location.column`, which is the order a reader expects and
  the order the tests assert.
- **`names.rs` is one module more than the layout has**, and it is where the
  parser's two questions live: `is_operation_name` (`label_rule`'s "is this word
  a Mnemonic or a Directive name?") and `text_operation` (2.5's "does this
  Operation take its Operand field as text?"). `MNEMONICS`, `DIRECTIVES` and
  `REFUSED_OPERATIONS` are the static set this step was told to write; when
  `instructions/` exists it becomes the source of truth for the first question
  and `is_operation_name` keeps its name and signature while its body becomes a
  table lookup — one function body, no call site. The second list is grammar,
  not instruction data, and stays in `names.rs` for good.
- **The Mnemonic set holds the refused instructions too** (`rte`, `stop`,
  `reset`), because `label_rule` has to read `reset` in column 1 as an Operation
  and leave "not implemented" to the analyzer. `tests/corpus/editor/flappy-bird.x68`
  writes `reset:` with a colon, so nothing in the corpus turns on it.
- **`parse_line(text, file, line_index) -> (ast::Line, Vec<Diagnostic>)`** is the
  parser's entry point, and `assembler::parse_line(text) -> ast::Line` in
  `mod.rs` is the 2.0 API's hover call, which drops the Diagnostics: hovering is
  not checking, and a line out of its File knows no Symbol, address or Macro.
- **`parse_file(file, text) -> ParsedFile` is the parser's other entry point**,
  and it exists because three of section 4's Diagnostics are not about a line:
  `bare_comment` and `double_quoted_string` are "the first one in a File" and
  `unterminated_macro_definition` is "no `endm` before the end of the File".
  `assemble` will call it once per File. `ParsedFile.lines` has one entry per
  Source line, so an index is never off by one, and the body of a Macro
  definition comes back as blank `Line`s.
- **`parse_line` raises neither once-per-File suggestion**, and needs no flag to
  say it could have: the tree already carries the facts. `Line::bare_comment`
  answers the first and the `QuoteKind` on every `Expr::CharacterLiteral`
  answers the second, which `parse_file::first_double_quoted_literal` walks.
  That is what lets the corpus test assert that the 30 `editor/` programs raise
  *nothing at all* line by line, rather than "nothing but two suggestions".
- **A text Operand field abandons the tokenizer.** Once the name is known to be
  a `text_operation`, the field and the Comment after it are read from the raw
  text by offset: `..\lib\io.x68` keeps its dots, `FAIL ERROR, Argument missing`
  keeps its comma, and `if.l d1 <hs> #NOON then.s` never meets the `<`. A
  `file_specification` runs `operand_field_extent` over characters
  (`file_specification_extent`, quotes, parenthesis depth and rule 4 included);
  a `message_text` and a `raw_operand_field` end at the first `;` or at the end
  of the line. Trailing whitespace is trimmed off all three, and an empty field
  is `None` rather than an empty `TextOperandField`.
- **The Macro skip recognises `endm` by a raw word scan**, because the body is
  not tokenized "not even its Label" (2.6): the first two words of the line, a
  trailing `:` stripped, compared case insensitively. It is the one place in the
  parser that reads a line without the tokenizer, and it is what makes
  `clockDigital.X68` come out with no Diagnostic at all.
- **Recovery is "report, skip to the next `,` of the Operand list, carry on".**
  A broken Operand costs its own Operand and nothing else: the Label, the
  Operation, the Operands beside it and the Comment all survive, which
  `a_broken_operand_does_not_lose_the_rest_of_the_line` pins.
- **An Expression term that cannot be read answers `TermError::Reported` or
  `TermError::Absent`**, which is how one mistake stays one message. `Reported`
  means a Diagnostic naming the real mistake has been raised (a register in an
  Expression, an unclosed `(`, a `+` where a term must begin) and the caller
  says nothing; `Absent` means there was no term at all and the caller — which
  knows what it would have followed — raises `expression_expected` "after `#`".
- **The Pratt loop can be entered with a left term already read**
  (`parse_expression_from`), which is what `parenthesised_operand` rule 2's
  second case needs: a `( … )` that turns out to have been a grouped Expression
  carries on as the left side of `(640-COLS*SCALE)/2`. `with_span` rebuilds the
  node with the parentheses inside its span, because `(1+2)` *is* `1+2` and has
  no node of its own, and an editor underlining it should cover the `(`.
- **`*` needs no special case at all.** In prefix position the loop reads it as
  `current_address` and in infix position as multiplication, which is exactly
  the table of 1.6; `2**3` therefore consumes `2 * *` and leaves the `3` as
  `unexpected_token_in_operand`, the reading 1.13 specifies.
- **`number_too_large` is raised only when every digit belongs to the base**, so
  a bad digit is one Diagnostic (the tokenizer's `invalid_number`) and not two;
  the value falls back to 0 so that the rest of the line still parses.
- **A character literal over four characters is not the parser's.** 1.9 gives
  that warning to the evaluator, and there is no `DiagnosticKind` for it yet;
  the parser stores the Latin-1 bytes and phase 2 counts them.
- **`ast` is `Serialize` for the wasm layer**: `Operand`, `Expr` and
  `RegisterListItem` are internally tagged as `{"kind": "immediate", …}` with
  snake_case names, the unit enums (`SizeSuffix`, `RegisterKind`,
  `SpecialRegister`, `CommentKind`, the operators, `NumberBase`, `QuoteKind`)
  are snake_case strings, and `Span` gained a derive so that every node carries
  its own. That shape is asserted as data in
  `a_line_serialises_as_plain_objects_for_the_editor`, not as a snapshot, and
  `every_line_of_the_corpus_serialises` runs every shape the corpus holds
  through `serde_json` (`bad-apple.x68` skipped by size, as
  `tests/corpus/README.md` allows: its 6526 `dc.b` lines are one shape and cost
  ten seconds). The old `#[serde(tag = "type", content = "value")]` of
  `lexer.rs` is deliberately not copied: `parseLine` is a new API and a flat
  `kind` reads better in TypeScript.
- **Eight changes to `docs/grammar.md`**, which ADR 0002 requires to come before
  the code, and which are the whole of what this step could not parse as
  written. *(Seven when this list was written; the eighth is 2.5 on `move.l
  (a0,`, which the code diverged from without saying so and which step 5 found
  and wrote down — the count and item 8 are step 5's correction of a factual
  error in this bullet.)*
  1. **1.10** — a reserved name gets `reserved_name_as_symbol` where it is
     *defined* and `register_in_expression` where it is *used* as an Expression
     term. The section said a `pc` outside a PC-relative mode was the first,
     which contradicted 2.7 and would have answered `move.l pc,d0` with an offer
     to rename the register.
  2. **2.3** — the `:` of a `colon_label` follows its name directly, because a
     field holds no whitespace (2.1). `loop :` is a `column_one_label` and then
     an `empty_label`.
  3. **2.4** — the whitespace between the Operation and its Operand field is
     **optional** in s68k. `move.l#5,d0` and `dc.b'x'` assemble: the tokenizer
     has already separated the fields and two names with no space between them
     merge into one token rather than opening a second reading, so nothing is
     ambiguous and the alternative is a message about a space.
  4. **2.5** — `parenthesised_operand` rule 1 gains a third case, "or the
     register is the last token of the Operand field", so that `move.l (a0` is
     "this looks like an indirect operand, `(a0)`, but the `)` is missing"
     rather than "`a0` is a register". `(pc)` matches rule 1 and is not one of
     its modes: `malformed_operand` naming `label(pc)`, "the displacement is
     missing".
  5. **2.5** — `-` `(` an address register with no `)` (`move.l d0,-(a7`) is
     `malformed_operand` naming the predecrement shape, for the same reason.
  6. **2.7** — unary operators may be chained (`~-5`), where the layered grammar
     writes one. The lenient direction, one recursive call, unwritten in the
     corpus.
  7. **Section 4** — three of the table's Diagnostics are raised over a File and
     not over a line (above); `operation_expected` also covers a line with no
     Label whose first word is not a name (`  #5`); and `unclosed_parenthesis`
     carries no related Location, because its own Location *is* the `(` and
     `malformed_operand` is the one that points back at it.
  8. **2.5** — `move.l (a0` and `move.l (a0,` do not get the same message. The
     bullet gave both the indirect one; the parser answers the second with the
     indexed shape and the missing index register, because the `,` is evidence
     that an index was meant, and that is the better diagnosis. The document now
     says so (step 5).
- **The tests are one module per grammar section and one name per rule**:
  `lexical_rules` (1), `the_grammar` (2), `ambiguities` (3, one test per
  subsection 3.1–3.11 — 3.12 is the Directives phase's), `parser_diagnostics`
  (4) and `corpus_pass` (6). `parser_diagnostics` holds a `CASES` table with one
  line per code the parser raises and a companion test asserting that the table
  is exactly section 4's list minus the tokenizer's five and `parse_file`'s
  three, so a new parser Diagnostic cannot be added without a case.
- **Two test helpers carry the readability**: `describe` writes an Operand back
  out in the canonical form of `tests/corpus/README.md` (`4(a6,d1.w)`, `-(a7)`,
  `d0-d3/a0-a2`), so `(4,a6)` and `4(a6)` read as the same thing in a table
  because they *are* the same thing; and `render` writes an Expression fully
  parenthesised, so a precedence test reads as the tree it asserts
  (`1<<2+3` is `((1 << 2) + 3)`).
- **Three corpus passes, and they are the step's real evidence.** All 8957 lines
  of the 30 `editor/` programs parse with no Diagnostic; the same lines with
  their indentation removed parse with no Diagnostic *and* give the same Label,
  Operation and Operands, which is `label_rule` shown to rest on column 1 and
  nothing else; and the 3 `easy68k/` originals raise one `bare_comment`
  suggestion each in `clockDigital.X68` and `graphicSound.X68` and nothing at all
  in `mouseWindowSize.X68`, which writes `;` comments throughout. Read line by
  line without the Macro skip those three raise exactly one error,
  `expression_expected` on `move.l #\1,d1` (`clockDigital.X68` line 22), which
  is what section 6 of the grammar predicted and the measure of what the skip
  buys.
- **Not done in this step**, and next: `expr.rs` (the evaluator, where the
  character-literal and 32-bit warnings and `division_by_zero` belong),
  `symbols.rs`, `instructions/` — which is where `names::is_operation_name` has
  to become a table lookup — `analyzer.rs`, `layout.rs`, `program.rs` and
  `assemble`. `unimplemented_operation` is raised by nothing yet: the parser
  reads a refused Operation into `Operation { name, text }` and leaves the
  Diagnostic to the Directives phase, as section 4 has it.

### Step 5 — `parser.rs` under review: nine findings answered

A review of step 4's parser raised one major finding and eight minor ones (one
of them informational). All nine are answered in the code; three of them changed
`docs/grammar.md` first, as ADR 0002 requires. `cargo test` is 196 green (5 new
tests, 191 unchanged), `rustfmt --check src/assembler/*.rs` is clean, no build
warning names `src/assembler`, and nothing under `tests/` moved — no fixture and
no snapshot, because none of these shapes is written in the corpus.

- **A `*` where the Operation would stand opens the Comment field.**
  `start: * entry point` and `start * entry point` were `operation_expected` and
  lost the Comment altogether; the tokenizer only makes a Comment token of a `*`
  that opens the *line*, and nothing downstream re-read it. `Parser::parse` now
  answers a `Star` in the operation position exactly as it answers a `Comment`
  token, which is what 2.2 (the Operation field is optional) and 1.6 (a `*`
  first in the Comment field is an `explicit_comment`) already said between
  them. `docs/grammar.md` 1.6 gained the sentence that spells the two rules out
  together, since the shape reads as a contradiction until they are put side by
  side. Pinned by `comment_rule_reads_a_star_after_a_label`, which also holds
  the `;` spelling of the same line.
- **The Comment after an `include` filename can be a bare one.**
  `parse_text_operand_field` hardcoded `CommentKind::Explicit` for all three
  text fields, so `include io.x68  load it` was an "explicit" Comment with no
  marker and the once-per-File `bare_comment` suggestion could never fire on it.
  Only the `file_specification` was wrong — `message_text` and
  `raw_operand_field` end at a `;` and nowhere else (1.5), so their Comment
  always carries a marker — and the kind is now decided by the first character,
  through the new free function `comment_kind`, which `parse_operand_field` also
  calls instead of its own inline `match`. Pinned in
  `file_specification_keeps_its_spaces_when_quoted` (the kind and the flag) and
  in `bare_comment_and_double_quoted_string_are_raised_once_a_file` (the
  suggestion firing on line 0 of a File whose first bare Comment is an
  `include`'s).
- **The `CASES` table is derived from `ALL_CODES` now, not restated.**
  `number_too_large` was missing from it *and* from the hand-written `expected`
  array beside it, so the two drifted together and the stated invariant — a new
  parser Diagnostic cannot be added without a case — did not hold.
  `the_table_covers_every_diagnostic_the_parser_raises` now filters `ALL_CODES`
  through three named exclusion lists (the tokenizer's five, `parse_file`'s
  three, the ten later phases owe) and compares what is left with the table, so
  a new `DiagnosticKind` fails the test until it is either given a case or
  written into an exclusion list. `number_too_large` and `nesting_too_deep` are
  the two rows added.
- **One bad character costs one message, in every arm.** `parse_prefix` already
  stayed quiet on a `TokenKind::Error` (the tokenizer has said what is wrong
  with it), but the two arms that meet the token *after* a complete Operand did
  not, so `move.l #1<2,d0` was `unexpected_character` **and**
  `unexpected_token_in_operand` on the same column. Both arms —
  `parse_operand_list`'s leftover arm and `parse_parenthesised_operand`'s —
  now make the same exception and recover as they otherwise would.
  `one_bad_character_costs_one_message` pins the three shapes and that the
  Operands beside the bad character survive.
- **`operand_expected` is raised once for one missing Operand.** `dc.b ,` raised
  it twice, once from the `,` arm of `parse_operand` and once from the
  end-of-list check after the same comma had been stepped over.
  `parse_operand_list` now carries a `bool` for "this Operand was reported
  missing" and the second check stays quiet; two missing Operands are still two
  messages (`dc.b ,,`). `operand_expected_is_raised_once_for_one_missing_operand`.
- **`move.l (a0,` names the indexed shape, and the document says so.** The
  parser answered it with `malformed_operand` naming the index register, while
  2.5 promised the indirect message. The code's answer is the better one — the
  `,` is evidence that an index was meant — so ADR 0002 was satisfied the other
  way round: `docs/grammar.md` 2.5 was amended to give `move.l (a0` and
  `move.l (a0,` a sentence each, and step 4's list of document changes above is
  corrected from seven to eight. `malformed_operand_names_the_shape_it_tried_to_be`
  already pinned both messages and is unchanged.
- **A `malformed_operand` covers the token its message names.** The span was
  `operand_start..last_end()` and the offending token had only been *peeked*, so
  `move.l 4(d0),d1` underlined `4(` while the message named the `d0` outside the
  underline. The two places that report about a peeked token —
  `parse_base_and_index`'s "is not an address register" and
  `parse_index_register`'s "the index register is missing" — now step over it
  before building the Diagnostic, which is all `malformed` needs.
  `a_diagnostic_points_at_the_characters_it_is_about` grew the two spans.
- **The Macro skip ends on a line whose *Operation* is `endm`.**
  `closes_a_macro_definition` answered `true` if either of the first two words
  was `endm`, so a body line `  bra endm` closed the definition early and the
  rest of the body was tokenized against the grammar's own rule (2.6). The scan
  still reads words, because the body is not tokenized "not even its Label", but
  it now reads them by `label_rule`: a Comment line has no Operation, a first
  word ending in `:` is a Label, a first word in column 1 that is not an
  Operation name is a Label, and what is left is the Operation. So `endm`,
  `done: endm` and `done endm` close a definition while `  bra endm`,
  `* endm in a comment` and `endm:` do not.
  `macro_definition_ends_on_an_operation_named_endm`.
- **A depth backstop under the Expression parser, `nesting_too_deep`.** A line of
  three thousand `(` overflowed a 1 MiB stack — wasm's default — and aborted
  instead of diagnosing; no diagnostic is worth a crash, and a trap is not a
  diagnostic at all. `MAX_NESTING_DEPTH` is 64 and the counter sits at the entry
  of `parse_prefix`, whose body moved to `parse_prefix_term`: **every** recursion
  of the Expression parser passes through there — a `(` descends through
  `parse_expression`, a chained unary operator calls it directly — so one guard
  covers nested parentheses and `-----…5` alike, which is why the message and
  the section 4 row say "levels" and not "parentheses". At the limit the descent
  stops with one Diagnostic and `TermError::Reported`, which every caller already
  answers by staying quiet, so the line costs one message. The new kind is the
  twenty-seventh row of section 4 and the thirty-seventh `DiagnosticKind`.
  `nesting_too_deep_stops_the_descent_rather_than_the_stack` runs a 5000-deep
  line on a thread with wasm's own 1 MiB stack, so the test is as much that it
  comes back at all; 64 `(` still parse, and the `CASES` row asserts its own
  parenthesis count against `MAX_NESTING_DEPTH` rather than trusting the eye.
- **What the review checked and did not find** is worth keeping: all 52 lines
  lifted from the EASy68K help parse with no Diagnostic (`quickStart`, the
  Directives pages, `Reference/68ks1e`'s effective-address examples), as do
  every row of the 1.4 label table, the 1.6 `*` table, the 3.5 table, the
  Addressing-mode spellings of 2.5, the register lists, the four bases and the
  precedence table read against `Directives/operators.htm`; 22,000 fuzzed lines
  panic on nothing, leave no Diagnostic column outside its line and no Operand
  span off a character boundary; and the parser is linear (8,000 Operands on one
  line in 1.5 ms release, the 6,645-line `bad-apple.x68` through `parse_file` in
  242 ms release).
- **Not done in this step**, and unchanged from step 4: `expr.rs`, `symbols.rs`,
  `instructions/` — where `names::is_operation_name` becomes a table lookup, its
  signature unchanged — `analyzer.rs`, `layout.rs`, `program.rs` and `assemble`.
  The analyzer still owes `unknown_mnemonic`, `mnemonic_used_as_label`,
  `unimplemented_operation` and the sentence 3.4 asks for on `nop *`; the
  evaluator still owes the over-four-character and over-32-bit warnings and
  `division_by_zero`.

### Step 6 — `src/assembler/instructions/` and `analyzer.rs`: the table, the checks, the lowering

The instruction table of ADR 0003 and the analyzer that reads it.
`src/assembler/` gains `instructions/` (`encoded.rs`, `table.rs`, `lowering.rs`)
and `analyzer.rs`; `src/instructions.rs` becomes a re-export; `names.rs` loses
its Mnemonic list to the table; `tests/diagnostics/` is new, with one program
and one snapshot per Diagnostic. `cargo test` is 238 green (42 new), `rustfmt
--check` is clean over every file touched, and `cargo build` raises the same 14
warnings it did before, none of them naming a new file. No corpus fixture and no
corpus snapshot moved.

- **The layout is the plan's, with `instructions/` in three modules.**
  `encoded.rs` is `Instruction`, `Operand`, `RegisterOperand`, `IndexRegister`,
  `Condition`, `ShiftDirection`, `Sign`, `Size` and `TargetDirection`, moved out
  of `src/instructions.rs` and documented on the way; `table.rs` is the table;
  `lowering.rs` is a checked Operation to an encoded `Instruction`.
  `src/instructions.rs` keeps `Label`, the Interrupts, and a `pub use` of all
  nine moved types, so the Interpreter, the debugger and the old pipeline name
  them where they always did and nothing outside the Assembler changed.
- **`names::is_operation_name` is a table lookup now**, as step 4 said it would
  be: one function body, no call site, and `MNEMONICS` is deleted rather than
  kept in step with the table. The table holds *exactly* the 125 Mnemonics that
  list held — checked against it name for name while the change was made — so
  `label_rule` reads every word the way it did yesterday. `DIRECTIVES` and
  `REFUSED_OPERATIONS` stay in `names.rs`: they are grammar, and the Directives
  phase has no table of its own yet.
- **A row is a Mnemonic, and a `Form` is a shape it may be written in.**
  `InstructionSpec { mnemonic, implementation, forms, value_rule }`, and
  `Form { operands: &[Modes], sizes: SizeRule, combination }` where `operands`
  has one entry per position and its length *is* the Operand count. Two
  instructions have two Forms of one arity (`cmp`, whose second is the `cmpm`
  shape, and `movem`, whose two are the two directions) and four have two of
  different arities (the shifts). Everything else has one.
- **`Modes` is a `bitflags` set and the names in it are the old checker's**
  (`Dn`, `An`, `(An)`, `(An)+`, `-(An)`, `d(An)`, `d(An,Xn)`, `Ea/<label>`,
  `Im`, `<register list>`), because those are the notation the asm-editor's
  documentation already writes. The manual's groups are named constants —
  `DATA`, `ALTERABLE`, `DATA_ALTERABLE`, `CONTROL`, `MEMORY`, `MOVEM_TO_MEMORY`,
  `MOVEM_FROM_MEMORY`, `COUNT`, `ANY_REGISTER` — so a row reads as the manual's
  own sentence and a group is written once.
- **The PC-relative modes are deliberately not in any `Modes` set.** They are
  parsed, and the analyzer answers them with the new
  `unimplemented_addressing_mode` before it looks at the position rule, so a
  student is never offered a mode that would then be refused. The same check
  covers `sr`, `ccr` and `usp`. Phase 3 adds the modes and deletes the check;
  the `Modes` flags for them can be added then.
- **The size rules are the help's "DATA LENGTH".** `SizeRule` is `Unsized`,
  `Any`, `WordOrLong`, `LongOnly`, `WordOnly`, `ByteOnly`, `ByteOrLong` and
  `Branch`, and the last is `.s`/`.w`/`.l` accepted with no range check, as the
  design record's "Instructions" asks. Reading the help's reference pages turned
  up that 1.4.2 refuses a size on eleven kinds of instruction that EASy68K
  documents one for — the help's own `MOVEQ` example is `MOVEQ.L #3,D0` — so
  `moveq.l`, `lea.l`, `pea.l`, `exg.l`, `swap.w`, the four `.w` divides and
  multiplies, `DBcc.w`, `Scc.b` and `.b`/`.l` on the four bit instructions are
  all accepted now. ADR 0001 asked for it and no corpus program writes one;
  `tests/corpus/README.md` records it with the rest.
- **The default size is the Form's**, word wherever there is a choice (the
  68000's own default, and what 1.4.2 stored), long for `extb` and `moveq`, none
  for the unsized ones, none for a branch (the fixture printer writes no size on
  one) and none for a bit instruction, whose width the destination decides and
  whose encoded form holds no size at all.
- **The byte rule fires only where a byte was a choice**, `SizeRule::Any`.
  Otherwise `scc a0` would be told both that `Scc` takes no address register and
  that a byte never reaches one, and only the first is the mistake.
- **`Family` is the encoding, not the Mnemonic**, and it carries what differs:
  `Family::Bcc(Condition::Equal)` for `beq`, `Family::Shift(Arithmetic, Left)`
  for `asl`, `Family::AddSub { subtract: true }` for `sub`. `lower` matches on
  it with no catch-all arm, so a new Family does not compile until it is given
  an encoding, and the normalisations of `tests/corpus/README.md` live in one
  function each: `add #1,a0` is an `addi` and `cmp #1,a0` a `cmpa`, in that
  order, because that is the order 1.4.2 wrote them in.
- **`Implementation::NotImplemented { reason, alternative }` is a row like any
  other.** Seventeen Mnemonics carry one — `movep`, `addx`, `subx`, `negx`,
  `abcd`, `sbcd`, `nbcd`, `roxl`, `roxr`, `tas`, `rtr`, `rte`, `trapv`, `chk`,
  `illegal`, `stop`, `reset` — with a reason that is a clause of "`movep` is not
  implemented: …" and an alternative that is a hint of "write … instead".
  Fourteen of them say "yet", because phase 3 adds them and this is the sentence
  phase 3 deletes; `rte`, `stop` and `reset` do not, because they are out for
  good and their reason is about the machine s68k simulates.
- **`trap` is two rules.** The vector is a `ValueRule` of 0 to 15, which is the
  instruction's own field; a vector in range but not 15 is an
  `unimplemented_operation` named "`trap #3`" — the Interpreter answers one trap
  and saying so is more use than "invalid immediate". 1.4.2 refused the same
  thing with "Only implemented TRAP is 15 for IO".
- **`ValueRule` is the field the instruction encodes, and not the operand
  size.** It carries `subject`, `min`, `max`, `max_in_memory` and a `hint` used
  word for word, and it is checked against the first Operand when that Operand
  is an immediate: the counts of `addq`/`subq` (1 to 8), the value of `moveq`
  (-128 to 255 — a `moveq` value is a byte pattern and `#$ff` is how a program
  writes -1, where 1.4.2 stopped at 127), a shift count (1 to 8) and a bit
  number (0 to 31 in a data register, 0 to 7 in a byte of memory, where 1.4.2
  allowed 0 to 255 whatever the destination). The range check against the
  *size* is separate and generic: every immediate, signed low to unsigned high,
  so `move.b #-1,d0` and `move.b #$ff,d0` are both a byte and `move.b #300,d0`
  is not.
- **The Form is chosen by "the first that fits, else the first of that arity".**
  Only `cmp` and `movem` have two of one arity and the order is deliberate: the
  general Form first, so that `cmp (a0)+,(a1)` — which fits neither — is judged
  against it ("there it takes Dn or An") rather than against the `cmpm` shape it
  half resembles. A test in `table.rs` fails if a third instruction grows two
  Forms of one arity without the choice being thought about.
- **The analyzer runs after the Layout**, which is what the step was told to
  design for and what makes the value checks real: a Label is an address only
  once the program is laid out, and `Context { symbols, current_address, origin
  }` is what the phases before it hand over. `Context::evaluate` is a fold with
  no diagnosis in it — an undefined Symbol, a division by zero and an overflow
  all answer `None` and the check is skipped — because those are the evaluator's
  Diagnostics and reporting them here would double every message. **When
  `expr.rs` arrives it takes that body over and the signature does not change**,
  which is the same arrangement step 4 made for `names.rs`.
- **A line the parser has already failed on is judged on its name and its size
  and nothing else.** Recovery leaves whatever survived — `move.l (d0),d1`
  reaches the analyzer with one Operand — and "move takes two operands" under a
  real mistake teaches nothing. The size is still judged, because the parser
  never looks at it.
- **One mistake, one message, twice over**: an Operand that fails its position
  rule is not then asked about its value, and an Operand this phase does not
  implement is not asked about either. `asl #9` is "the first operand of `asl`
  cannot be an immediate" and not that as well as "the count of a shift is 1 to
  8".
- **`mnemonic_used_as_label` is raised only when the line has an error**, which
  is `label_rule` row 5's real shape: `clr` alone in column 1 is "`clr` takes
  one operand" *and* "write `clr:` if `clr` is a label", while `clr d0` in
  column 1 is an instruction and nothing is said. It stays a suggestion, as
  step 3 decided.
- **`nop *` is a warning and the `*` is dropped.** The design record asks the
  analyzer for the sentence and says nothing about severity; an error would stop
  `nop     * do nothing` from building, which is an EASy68K program ADR 0001
  says has to assemble unchanged. So: an Operation whose every Form takes no
  Operands, given exactly one Operand that is a bare current address, warns that
  the `*` was read as the current address, says `;` starts a comment, and
  assembles. `nop *+2` is not that shape and stays a `wrong_operand_count`
  error.
- **Seven new `DiagnosticKind`s**, each with a golden case:
  `wrong_operand_count`, `both_operands_in_memory`, `address_register_byte_size`,
  `unimplemented_addressing_mode`, `value_out_of_range`, `bare_number_as_address`
  (a suggestion) and `star_is_the_current_address` (a warning).
  `invalid_addressing_mode` gained a `suggestion` field, which is where the
  named mistakes go: `clr a0` is answered with `suba.l a0,a0`, an address
  register where a data mode belongs with "move it into a data register first",
  an immediate written to with "an immediate is a value, and nothing can be
  written to it", and a register list outside `movem` with "only `movem` takes a
  register list".
- **A message names what was found in prose and what is allowed in notation**:
  "the first operand of `clr` cannot be an address register" and then "there it
  takes Dn, (An), (An)+, -(An), d(An), d(An,Xn) or Ea/<label>". The prose is
  `ast::Operand::description()`, which step 3 wrote for exactly this, and the
  notation is `Modes::names()`; mixing them was a choice, and the reason is that
  a sentence reads and a list of nine spellings does not.
- **`bare_number_as_address` fires narrowly**: a literal number (never a
  Symbol), at least zero and below the origin, in a position where the
  instruction would also have taken an immediate. So `move.l 5,d0` is answered
  and `bra $10`, `move.l $2000,d0` and `move.l count,d0` are not.
- **Two changes to the parser, and `docs/grammar.md` 2.5 first**, as ADR 0002
  requires. `(d0)`, `(d0,d1)` and `-(d1)` were `register_in_expression` — "an
  expression holds no registers", which describes a program nobody wrote —
  because `parenthesised_operand` rule 1 and `looks_like_predecrement` both
  asked for an *address* register. They now ask for any register and the failure
  is `malformed_operand` naming the shape: "this looks like an indirect operand,
  `(a0)`, but `d0` is not an address register", which is the sentence `4(d0)`
  already produced and the one ADR 0003 promises. The related "this `(` is never
  closed" is no longer attached when the `(` *is* closed and the base is the
  mistake.
- **`tests/diagnostics/` is 40 programs and 40 snapshots**, one per code the
  parser and the analyzer can raise, snapshotted as the serialised Diagnostics
  themselves — severity, code, message, hint, location, related — so the
  snapshot is what the TypeScript side receives and is meant to be read as a
  student would read it. Two of them were rewritten after reading the first
  generation: `malformed_operand` carried a related location saying a closed `(`
  was never closed, and `invalid_size` wrote its sizes without backticks.
  `every_diagnostic_kind_has_a_case` compares the directory with `ALL_CODES` and
  fails on a new kind until it has a program or is written into
  `RAISED_BY_A_LATER_PHASE` (the four the evaluator and the symbol table owe).
- **`diagnostics_of` in `src/test/diagnostics.rs` is `assemble` with two phases
  missing**: parse every line of a File, then run the analyzer over every
  Operation, sorted into source order. It is the shape `assemble` will have, and
  the snapshots are what will say whether `assemble` still answers the same when
  it replaces it.
- **The evidence the table is compatible**: `the_analyzer_is_silent_on_every_editor_program`
  runs all 30 `editor/` programs, `bad-apple.x68` included, through parser and
  analyzer and asserts **no Diagnostic at all**. The three `easy68k/` originals
  come out with five: one `unknown_mnemonic` on a Macro invocation
  (`clockDigital.X68` line 67 — macros are not implemented, so the call site is
  a word the Assembler does not know), one `unimplemented_addressing_mode` on
  `andi.w #$00,SR`, and three `unimplemented_operation` on `rte`. That is the
  shape the design record's "Tests" item 2 predicts, a Macro invocation aside.
- **What the table refuses that 1.4.2 accepted, and the other way round**, is
  written out in `tests/corpus/README.md` under "What the instruction table of
  the rewrite accepts and refuses differently", with the note that no fixture
  moves. In one line: `extb`, branch sizes, the memory shift's `.w`, the help's
  data lengths, `cmp (a0)+,(a1)+` and `moveq #255` are new; `tst a0`, `jmp
  (a0)+` and its kind, `movem` through the wrong side of a walk, a count read
  from memory and a bit number wider than its destination are refused.
- **Not done in this step**, and next: `expr.rs` (which takes over
  `Context::evaluate` and owes the over-four-character and over-32-bit warnings,
  `division_by_zero` and `undefined_symbol`), `symbols.rs` (`symbol_already_defined`,
  `forward_reference_not_allowed`, and the `SymbolValues` the analyzer already
  asks), `layout.rs`, `program.rs` and `assemble`. The Directives are still
  phase 2's: the analyzer returns without a word on any Operation whose name is
  a Directive, `unimplemented_operation` on a refused Directive included. Two
  smaller things worth carrying: a Macro *invocation* is an `unknown_mnemonic`
  today, which is honest but may deserve its own sentence when macros are
  decided; and `link a6,#-4` has no range check on its displacement, because the
  instruction is unsized and the generic immediate check hangs off the size.

### Step 7 — `symbols.rs`, `expr.rs`, `layout.rs`, `program.rs`: `assemble` builds a Program

The end of phase 1 for a single File. `src/assembler/` gains `symbols.rs`,
`expr.rs`, `layout.rs` and `program.rs`; `mod.rs` gains `assemble` and
`Assembly`; `src/test/corpus.rs`'s dump is the new Assembler's. `cargo test` is
302 green (64 new), every file this step wrote or touched is rustfmt clean,
`cargo build` raises the same 14 warnings it did before and `cargo clippy
--all-targets` no new one. **Five corpus snapshots moved, eight entries in all,
and every one of them is the `ds` fix**; `tests/corpus/README.md` has the table
and the reasons.

- **The module layout is the plan's**, name for name, and `mod.rs` now declares
  the whole of it. `assemble(&Files, entry) -> Assembly` is what the plan asks
  for; `assemble_source(&str)` beside it is the 2.0 API's "a single string wraps
  as `main.m68k`" and is what every test calls.
- **`Assembly { diagnostics, program }`, and `program` is `Some` exactly when no
  Diagnostic is an error.** The Program is built either way — the Layout has no
  reason to stop — and `assemble` is the one place that decides whether it may
  be handed out, which keeps "assembly continues after an error" (ADR 0003) a
  property of the pipeline rather than of every phase.
- **The pipeline is four calls**: `parser::parse_file`, `layout::lay_out` (which
  runs both passes and the analyzer inside them), the sort by (line, column),
  and the error test. The sort is stable, so two Diagnostics about the same
  columns keep the order the phases ran in: the parser's, then the Layout's,
  then the analyzer's.
- **A Local label's full name is EASy68K's own.** `quickStart.htm` ("Label
  Field") says the assembler "creates a unique name for local labels by
  appending the local label name to the preceding global label and replacing the
  dot with a colon", so `.loop` under `start` is `start:loop` — not
  `start:.loop`, which is what this step first wrote. A Local label before any
  Global one is in the File's nameless scope and comes out as `:loop`. EASy68K
  keeps only the first 32 characters of a name significant and s68k keeps all of
  them, which is the lenient direction of ADR 0001.
- **Only a Label opens a scope.** `equ`, `set` and `reg` name a value, and a
  value is not a Label (CONTEXT.md, "Global label"), so a Constant between two
  Labels does not close the first one's Local scope.
- **A Variable is a list of definitions, not a value.** "Each use sees the
  latest definition above it" (CONTEXT.md, "Variable") is implemented literally:
  `Symbol::value_at(at)` finds the last `set` at or above `at`, and a use above
  the first `set` has no value at all. `at` is the Source line index today;
  phase 4 makes `include` textual and has to make it a position in the assembled
  order instead, and `Symbol::number_at` is the only place that compares two.
- **`SymbolTable::define` answers the Diagnostic, boxed.** The "already defined"
  error carries both Locations and is built where the rule is, not at the four
  call sites. It is a `Box<Diagnostic>` because clippy is right that the error
  half of that `Result` is much the larger one.
- **`SymbolsInScope` is the one view both questions ask.** It implements
  `expr::Symbols` (the evaluator's, which tells a value from a Register list
  from a forward reference from an unknown name) and `analyzer::SymbolValues`
  (the analyzer's, which is a value or nothing), so the two phases cannot
  disagree about what a name means on a given line.
- **`analyzer::Context::evaluate` kept its signature and lost its body**, as
  step 6 promised: it is `expr::value` through a small adapter.
  `analyzer::NoSymbols` is now a re-export of `expr::NoSymbols`, so a caller
  with no program has one name for it whichever question it is answering.
- **The arithmetic, written down because the help does not write it down**
  (`expr.rs`'s module comment is the same list): `/` truncates towards zero and
  `\` is the remainder of that division, both Rust's own and both the C
  behaviour EASy68K inherits; `+ - *` wrap in 64 bits rather than overflowing,
  because a Diagnostic about a number nobody can write is worth less than the
  value the rest of the line still gets; `<<` and `>>` work on the low 32 bits
  and give back that pattern as a signed 32-bit number, which is the width
  EASy68K computes in and the only width that gives `>>` a meaning, so `-1>>0`
  is `-1` and `-1>>1` is `$7fffffff`; a shift count is read as an **unsigned**
  32-bit count, so 32 or more — a negative count included — shifts everything
  out and gives 0. A character literal packs its Latin-1 bytes first byte
  highest, as 1.4.2 did, and keeps its last eight.
- **The evaluator collects Problems and the caller turns them into
  Diagnostics.** `expr::evaluate` knows nothing about lines or Directives; it
  answers `(value, Vec<Problem>)`, and `expr::diagnose` adds the Location and —
  for a forward reference — the name of the Directive that could not wait.
  That is what lets one fold serve pass 1 (where a forward reference is an
  error), pass 2 (where it cannot happen) and the analyzer (which says nothing
  at all). Both sides of a binary operator are evaluated even when the first has
  no value, so two undefined names in one Expression are two messages.
- **A forward reference is told from an unknown name by a set collected before
  pass 1.** `declared_names` walks the parsed lines for every name the File
  defines; a name pass 1 cannot resolve that is in the set is
  `forward_reference_not_allowed` and one that is not is `undefined_symbol`.
  Without it the two are the same failure and the message would have to guess.
- **Pass 1 is a plan per line**: an address, the Global label in force, and what
  the line puts in the program (nothing, an instruction, `dc`/`dcb` bytes, or
  `ds` room). Pass 2 reads the plans back, so the two passes share no mutable
  state and pass 2 can be read on its own.
- **Alignment happens before the Label is defined**, so a Label on a `dc.w` after
  an odd byte names the padded address and not the byte before it. `org` is the
  exception: its Label names the address `org` moved to, which is what a student
  writing `data org $2000` means.
- **`*` is the address the line is laid out at**, alignment applied — and for
  `org` the address it is about to leave, which is what makes
  `org (*+1)&-2`, the help's own alignment idiom (`Directives/org.htm`), work.
- **`dc` lengths are pass 1's and `dc` values are pass 2's.** The length of an
  item depends only on the size suffix and on whether the item is a quoted
  literal, so the Layout never needs a value a `dc` item might not have yet,
  which is exactly the forward reference the design record allows there.
- **A `dc` item is a string only when it is one bare quoted literal.** `'A'+1`
  is a value (`docs/grammar.md` 1.9), and a string is padded up to a whole
  number of items, so `dc.w 'abc'` is two words — which is what 1.4.2 did.
- **Every `dc` and `dcb` value is range checked against its size**, signed at the
  bottom and unsigned at the top, the same range an immediate of that size
  holds, so `dc.b 255` and `dc.b -1` are both a byte and `dc.b 300` is
  `value_out_of_range`. 1.4.2 truncated in silence.
- **Nothing may be laid out past 16 MB.** An `org` address, a `ds` or `dcb`
  count and the end of every placement are checked against the address space the
  Interpreter has, with `value_out_of_range` naming what was too big. 1.4.2
  would have tried to allocate it.
- **Overlap is one sweep, and it names the later line.** The placements are
  sorted by address and walked once, so `bad-apple.x68`'s 6 526 data lines cost
  a sort and not a comparison of every pair; the Diagnostic goes on whichever of
  the two lines is further down the File, with the other as its related
  Location.
- **`end` sets the Entry point in pass 2**, because the Entry point decides
  nothing about the Layout and a forward reference is therefore allowed in it —
  `END START` is written at the bottom of every EASy68K program. `end` with no
  Operand warns and falls back to `START`, then to the first instruction in
  source order.
- **`code_after_end` warns on the first line after `end` that carries a Label or
  an Operation**, and never on a blank or Comment line, which is what step 3
  asked for: all three EASy68K originals close with `END START` and then
  EASy68K's own editor comments.
- **The Directives of this step are `org`, `equ`, `set`, `dc`, `ds`, `dcb` and
  `end`**; `opt`, `list`, `nolist` and `page` are accepted in silence; every
  other name `names::is_directive` knows raises `unimplemented_operation` with a
  reason and, where there is one, something to write instead
  (`layout::unimplemented_reason` is the table). `simhalt` is among them, which
  means a program that writes it does not build until phase 2 — the reason names
  `move.b #9,d0` and `trap #15` as what to write meanwhile.
- **A Directive's mistakes reuse the analyzer's kinds where they are the same
  finding**: the wrong number of Operands is `wrong_operand_count` (with the
  Directive's name where a Mnemonic would be), a size a Directive does not carry
  is `invalid_size` with an empty `allowed` list ("`org` carries no size"), and a
  count outside what memory holds is `value_out_of_range`. Only what has no
  neighbour got a new kind.
- **Eight new `DiagnosticKind`s**: `odd_origin`, `address_used_twice`,
  `code_after_end`, `end_without_an_address`, `directive_needs_a_label`,
  `value_expected` (a Directive given an Addressing mode where a value belongs),
  `register_list_in_expression` (EASy68K's "Register list symbol used in an
  expression", which phase 2's `reg` makes reachable) and `unreadable_file` (the
  Entry file is missing, or holds bytes; phase 4's `include` reuses it). The
  first four are the design record's warnings; the rest are errors.
- **The four Diagnostics the earlier steps owed are raised**, each with its
  golden case: `symbol_already_defined`, `undefined_symbol`,
  `forward_reference_not_allowed` and `division_by_zero`.
  `RAISED_BY_A_LATER_PHASE` in `src/test/diagnostics.rs` is replaced by
  `WITHOUT_A_CASE`, which is down to the two kinds no single File of source can
  raise and says which phase makes each reachable.
- **`diagnostics_of` in `src/test/diagnostics.rs` is `assemble` now**, as step 6
  intended, and **one of the 40 snapshots moved for a reason worth keeping**:
  `unterminated_macro_definition.asm` gained the `unimplemented_operation` its
  `macro` line always deserved, which is the Directives phase arriving. The
  other 39 are unchanged, so the analyzer answers the same with a real symbol
  table and real addresses as it did with none.
  `unimplemented_addressing_mode.asm` grew a `greeting: dc.b 'hello',0` line, so
  that its `greeting(pc)` is a defined name and the case stays about the
  Addressing mode.
- **`Program` holds four things and looks an instruction up by binary search.**
  The instructions are sorted by address and no two share one — that is an
  error, so a Program cannot hold it — which means no index beside the list.
  Every instruction carries its `size` (4 today), its `Location` and its Source
  line; every memory run carries its Location, its address and either its bytes
  or the room it reserves; the Symbols are a `BTreeMap` by full name.
- **`lower_operand` reads an absolute address `as u32 as usize`** rather than
  `as usize`, so a negative Expression is the address its two's complement names
  and not a 64-bit number. 1.4.2 stored the same 32 bits; the bug was
  unreachable from the corpus and is fixed before it is not.
- **The corpus dump is the Program's**, and the fixtures held: over the 30
  `editor/` programs every instruction, address, byte, Label, `line` and Entry
  point is identical to 1.4.2's, and the only entries that moved are the `ds`
  runs — 8 of them, in 5 fixtures — which now say how much room they reserve
  instead of how many zeros 1.4.2 wrote. `tests/corpus/README.md` has the table,
  the reason and the rules that changed without moving a fixture (data
  alignment, Constants as Symbols, `end` as an Entry point, Local label names).
- **The `-run.snap` and `-errors.snap` fixtures are still the old pipeline's**,
  as this step was told to leave them: the Interpreter takes a `Compiler` until
  the next step, and `src/test/corpus.rs` calls that path `old_pipeline` now so
  that the two cannot be confused.
- **The three EASy68K originals come out with 40 Diagnostics and a finding.**
  `the_easy68k_originals_raise_only_what_is_not_implemented` counts them by code:
  35 `unimplemented_operation` (the Macro definition, the structured-control and
  conditional keywords, `simhalt`, `rte`), 1 `unimplemented_addressing_mode`
  (`andi.w #$00,SR`), 2 `bare_comment` suggestions, 1 `unknown_mnemonic` (a Macro
  *invocation*) — and **1 `undefined_symbol`**: `mouseWindowSize.X68` writes its
  Label `start` and its last line `END START`. Symbols are case sensitive here
  (CONTEXT.md, "Symbol") and EASy68K's own example proves that its are not, so
  ADR 0001's "an EASy68K program assembles unchanged" and the case rule cannot
  both hold. The test keeps the line visible rather than hiding it; **which of
  the two to keep is the owner's decision and is the one question this step
  leaves open.**
- **Not done in this step**, and next: the Interpreter still takes a `Compiler`,
  and switching it to the `Program` is the next step — `Program::instruction_at`,
  `final_instruction_address`, `memory()` and `symbols()` are what it needs, and
  the `ds` fix will move every `-run.snap` of a program with a `ds` in it,
  because memory starts as `$ff` and nothing zeroes a reserved block any more.
  The old pipeline (`lexer.rs`, `semantic_checker.rs`, `compiler.rs`) is still
  in the crate and still assembles; `lib.rs` still exposes only it to wasm, and
  the 2.0 API's `S68k.assemble` is the step after that. The evaluator still owes
  the two warnings of `docs/grammar.md` 1.9 — a character literal of more than
  four characters, and a constant over 32 bits — which have no `DiagnosticKind`
  yet.

### Step 8 — the Interpreter runs the Program, and the old pipeline is deleted

The end of phase 1. `src/interpreter.rs` and `src/debugger.rs` take the
[`Program`](../../src/assembler/program.rs); `src/lexer.rs`,
`src/semantic_checker.rs`, `src/compiler.rs`, `src/utils.rs` and
`src/constants.rs` are deleted, and with them the `regex` and `lazy_static`
dependencies; `src/main.rs` is a command line over the Assembler; the corpus
fixtures all three run on the rewrite. `cargo test` is **314 green** (12 new),
`cargo build --all-targets` raises **no warning at all** where it raised 14
before, `cargo clippy --all-targets` no new one, and every file this step wrote
or touched is rustfmt clean.

- **The Interpreter owns a `Program` and looks an instruction up in it.** The
  `Vec<InstructionLine>` and the dense `instruction_map` — a `usize` for every
  address up to the last instruction — are gone;
  `Program::instruction_at` (binary search) is the whole of the look-up, and
  `Interpreter::get_instruction_at` is a one-line call to it. Ten thousand
  instructions cost fourteen comparisons and no allocation.
- **`Program::final_instruction_address` became `Program::end_address`**, which
  is the last instruction's address *plus its stored size*: one past the last
  byte of the program. Every "has the run walked off the bottom" test is
  `pc >= end_address`, where it was `pc > address of the last instruction`. The
  two agree while every instruction is 4 bytes and every jump lands on one; they
  differ on a jump into the middle of the last instruction, which used to
  terminate the program quietly and is now the address error it is. Step 7's
  notes name `final_instruction_address` as what the Interpreter would need;
  this is that method, renamed with its meaning.
- **`Program::instruction_ending_at(address)` is new**, and it is what replaces
  `get_instruction_at(pc - 4)` in undo: the return address on the stack is one
  past the instruction that called, and only the size stored with each
  instruction turns it back into the call. It is a `partition_point` and a
  check, so an address that ends no instruction answers `None` rather than the
  instruction before it.
- **Three `pc - 4`s became one field.** `BSR`, `JSR` and `RTS` recorded the
  address of the instruction being executed as `self.pc - 4`, the program
  counter having already been stepped. The step records that address as
  `current_instruction_address` (1.4.2's `last_line_address`, renamed for what
  it holds: the instruction being executed, and after the step the one that has
  just run), and the three sites read it. Nothing in the Interpreter now
  computes an instruction address by arithmetic on the program counter.
- **The program counter is stepped by the instruction's own `size`**, read out
  of the `AssembledInstruction` beside the `Instruction` itself, so a version
  that stores real sizes changes the Assembler and nothing here.
- **`prepare_memory` is a free function over `(&mut Memory, &Program)`** — the
  `TODO` on it since 1.4.2 — because the Interpreter now owns the Program it
  would be reading while writing its own memory. It writes
  `MemoryContent::Bytes` runs and skips `MemoryContent::Reserved` ones, which is
  the running half of the `ds` fix.
- **A breakpoint is a `Breakpoint { file, line }`**, a Location without its
  columns, `Serialize` and `Deserialize` so that the wasm layer can pass the
  editor's own objects. `Interpreter::get_breakpoint_addresses` turns them into
  the set of addresses of the instructions those lines assembled to, so a
  breakpoint on a Comment, a Directive or a Label alone stops nothing, and a
  breakpoint on a line of another File stops nothing either. The dense
  `Vec<bool>` indexed by address is gone with the instruction map; a `HashSet`
  of addresses replaces it, and the "do not stop on the line the pc is already
  on" rule is unchanged.
- **`get_current_location() -> Option<&Location>`** replaces
  `wasm_get_current_line_index() -> usize`, which could only ever answer a line
  of one File and answered `0` for "no instruction here" and for "line 0" alike.
- **An undo step carries a `Option<Location>`, not a line.** `ExecutionStep.line`
  became `location`, filled by `Debugger::set_location`, and it is `None` for
  the empty step the history starts with. **The Location is cloned only when a
  history is kept**: it holds the path of its File, and the corpus run harness
  keeps none, so a step of a six-million-step run allocates nothing.
- **A call-stack frame carries the Location of its Label.**
  `PrettyStackFrame.label_line: usize` became
  `label_location: Option<Location>`, `None` being the frame whose address has
  no Label ("Unknown"), which used to be written as line 0. `Debugger::new`
  takes the Program's Symbols (`&BTreeMap<String, ProgramSymbol>`) instead of
  the old `HashMap<String, Label>` and keeps only the Labels — a Constant, a
  Variable and a Register list name values, not addresses. Where two Labels sit
  on one address the first in name order wins, deterministically, where 1.4.2
  took whichever the `HashMap` iterated last.
- **`instructions::Label` is deleted** and `src/instructions.rs` is what step 6
  made it, a re-export of the encoded types, plus the Interrupt catalogue, which
  is the Interpreter's and belongs to no phase of the Assembler.
- **`S68k` and `WasmSemanticErrors` are deleted with the pipeline they wrapped**,
  and `src/lib.rs` is module declarations and a doc comment saying so. The 2.0
  API — `S68k.assemble(source, options?)`, `parseLine`, breakpoints as
  `{ file, line }` — is the next step's, and **until it lands `ts-lib` does not
  build**: `ts-lib/src/index.ts` names `RawS68k`, `SemanticError`, `LexedLine`,
  `ParsedLine` and `wasm_compile`, none of which exist any more. CI's Rust steps
  are green and its "Build TypeScript library" and "Run smoke test" steps are
  not, and cannot be until the API step. Nothing was half-written to keep them
  alive, because a half API is one the next step has to unpick.
- **`src/ts_types.rs` describes the new shapes.** `InstructionLine`,
  `ParsedLine`, `LexedLine`, `LexedOperand`, `LexedRegisterType` and `Label`
  were the old pipeline's and are gone; `Location`, `AssembledInstruction`,
  `Breakpoint` and `ProgramSymbol` are new, `Step` names the new instruction and
  `ExecutionStep` carries `location` in place of `line`. These declarations are
  hand-written and only a reader keeps them true, which their new module comment
  says.
- **The CLI takes a file and never asks a question.** `s68k [FILE]` assembles
  `FILE` (`code-to-run.asm` by default), prints every Diagnostic as
  `file:line:column: severity: message` with its hint and its related locations
  under it — 1-based there, as every other command line tool counts, and 0-based
  in the `Location` itself — and refuses to run a program with an error in it,
  exiting 1. The mode is a flag (`--step`, `--benchmark`, `--run`,
  `--show-program`, `--no-debug`) where it used to be a prompt that read the
  terminal in a `while` loop: `console`'s `Term::read_line` answers an empty
  string for ever when standard input is not a terminal, which is the endless
  loop this step was told to remove. In `--step` mode anything that is not one
  of the four keys, the end of the input included, stops the run. Its interrupt
  handler grew the four display tasks a terminal can honestly do and 1.4.2's
  could not — a number in a base, a number in a field, a string with a number,
  a string with a number read — because half the corpus stopped on
  "Unhandled interrupt" before reaching its output.
- **`code-to-run.asm` is a hello-world now.** It held
  `move.w label-4(a0,d1.w),d0`, which the Assembler correctly refuses — a
  displacement is -128 to 127 and `label-4` is 4092 — so `cargo run` with no
  argument printed a Diagnostic and stopped. It is the file the README tells a
  newcomer to run, so it is a program that runs.
- **`mod test` is `#[cfg(test)]`.** It was not, so a plain `cargo build`
  compiled the test helpers and warned that seven of them were unused; those
  were half of the 14 warnings the build had. The other seven were the deleted
  pipeline's.
- **`src/test/test.rs` is ported name for name**, `lex_and_run` becoming
  `assemble_and_run` and `prepare` and `run_answering` keeping theirs;
  `lex_only`, which nothing called, is gone. Every trap case runs unchanged.
  **One case changed meaning because the language did**: `equ_substitution` read
  `ten equ #10` and `register_1 equ d1`, which is 1.4.2's `equ` aliasing
  arbitrary text — the `#` of an immediate and a register name included. ADR
  0001 records that `equ` no longer does that, so the case is now `ten equ 10`
  with `move.l #ten,d1` and it asserts the value arrives in the register, which
  is what the case was about.
- **Twelve new tests**, eleven of them in `src/test/test.rs`'s
  `running_a_program` module: the Entry point, an empty Program, the end of the
  program, `dc` writing and `ds` reserving, the current Location line by line,
  a breakpoint stopping on its line, a breakpoint of another File and of a line
  that assembles to nothing stopping nothing, the call stack naming a Label and
  its Location, undo putting the call stack back, and an undone step carrying
  the Location of the line that ran. The twelfth is
  `the_calling_instruction_is_found_by_the_address_it_returns_to` in
  `program.rs`.
- **`src/test/corpus.rs` is one pipeline now.** `old_pipeline` is deleted, `run`
  builds an `Interpreter` over the assembled `Program`, and
  `ds_zeroes_less_than_it_reserves` is `ds_reserves_memory_without_writing_it`,
  which asserts `255,255` where it asserted `0,255`.
- **The `easy68k/*-errors.snap` fixtures are the Assembler's Diagnostics**,
  serialised as `tests/diagnostics/` serialises them, which is what
  `tests/corpus/README.md` has said they would become since phase 0 ("A total
  rewrite of these three files is the intended outcome of the rewrite, not a
  regression"). 88, 138 and 64 strings of the old checker became 16, 4 and 20
  Diagnostics, and their sum is the 40 that
  `the_easy68k_originals_raise_only_what_is_not_implemented` counts by code, so
  the two tests cannot drift apart.
- **Three `-run.snap` fixtures moved, one line each, and all three are the `ds`
  fix.** `flappy-bird`, `number-to-string-1` and `snake-1` have a different
  `memory` hash and an identical `status`, `steps`, `d`, `a`, `pc`, `flags` and
  `output`; the other 27 are byte for byte unchanged. Two of the five programs
  with a `ds` did not move: `counting-loop-1` fills every byte of its block
  before it ends, and `variables-in-memory-1` reserves four bytes, of which
  1.4.2 zeroed an eighth rounded down, which is none. The table and the reasons
  are in `tests/corpus/README.md` under "What step 8's Interpreter changed".
- **Two things worth carrying into the API step.** A call frame's
  `source_address` is the address the call *returns to* and not the address of
  the calling instruction — it always was, and the editor reads it — so the name
  is a lie the API step may want to correct rather than the arithmetic; and
  `wasm_get_last_line_address` keeps its 1.4.2 name while answering
  `current_instruction_address`, which is the same number it always answered.
- **Not done in this step**, and next: the wasm 2.0 API (`S68k.assemble`,
  `parseLine`, the Program crossing into JavaScript, breakpoints as
  `{ file, line }` from the editor) and the `ts-lib` wrapper and smoke test that
  go with it; then phase 2's Directives, phase 3's instructions and phase 4's
  `include`. `docs/grammar.md` is untouched by this step, the parser having not
  changed.

### Step 9 — the 2.0 public surface: `assemble`, `parseLine`, the Program handle

The end of the API work. `src/lib.rs` is the WebAssembly boundary again —
`wasm_assemble`, `wasm_parse_line`, the `WasmAssembly` handle and the
`Interpreter` constructor — `src/ts_types.rs` describes every shape that crosses
it, `ts-lib/src/index.ts` is the `S68k` / `Program` / `Interpreter` wrapper the
asm-editor calls, and the whole chain CI runs (`cargo test`, `wasm-pack build`,
`npm ci`, `npm run build-lib`, `npm test`) is green from a clean `pkg` and
`dist`. `cargo test` is **322 green** (8 new), `cargo build --all-targets`
raises no warning, `cargo clippy --all-targets` no new one (the 6 that remain
are all older than this step: `Memory::new`/`Default` and a `single_match` in
`src/interpreter.rs`, a `redundant_field_names` in `tokenizer.rs`, a
`to_digit_is_some` in `token.rs`, `module_inception` and "items after a test
module" in `src/test/`), `cargo doc` no warning in a file this step wrote, and
everything is rustfmt clean. Both versions are **2.0.0**.

- **`assemble(files, entry)` is one function and the Program never crosses.**
  `wasm_assemble` answers a `WasmAssembly`, a handle holding the Diagnostics and
  the `Program`; JavaScript reads `wasm_get_diagnostics()` (an array of plain
  objects), asks `wasm_has_program()`, and hands the *handle* to
  `new Interpreter(assembly, options)`, which clones the Program out of it. A
  program of ten thousand instructions is never serialised, one assembly builds
  any number of Interpreters — which is what restarting a run is — and nothing
  the Interpreter does can reach the Diagnostics beside it.
- **What the Program says about itself is `ProgramInfo`**:
  `{ entryPoint, endAddress, instructionCount, symbols }`, the symbols by full
  name. The instructions are deliberately not in it: the Interpreter already
  answers `wasm_get_instruction_at` for the one the editor is looking at, and a
  symbol table is what a program listing actually needs.
- **A Location is serialised camelCase**, `{ file, line, column, endColumn }`,
  and that is the one shape everywhere — inside a Diagnostic, an assembled
  instruction, an undo step, a call-stack frame. The alternative was to keep
  `end_column` in Rust and translate in `ts-lib`, which would have meant
  translating every object that carries a Location on every step of a run. The
  cost is one key in the 53 snapshots that hold a Location, regenerated with
  `INSTA_UPDATE=always` and justified in `tests/corpus/README.md` under "What
  step 9's public API changed in these fixtures"; the counts, messages, codes,
  lines and columns are all exactly what step 8 left.
- **The Assembler's shapes are camelCase and the Interpreter's are not.**
  `ExecutionStep.old_ccr`, `StackFrame.source_address`,
  `InterpreterOptions.keep_history` keep the names 1.4.2 gave them, because the
  brief for 2.0 changes the Interpreter's surface only where a line index became
  a Location, and renaming the rest is churn in the editor for nothing. The rule
  is written at the top of `src/ts_types.rs` so that the next shape added lands
  on the right side of it.
- **`to_plain_js` is the boundary's serializer.** `serde_wasm_bindgen`'s default
  writes a Rust map as a JavaScript `Map`, which `JSON.stringify` renders as
  `{}` — the Program's symbols arrived empty in the first smoke test. It is
  `Serializer::new().serialize_maps_as_objects(true)`, so a map crosses as the
  plain object the design record promises, and every value that leaves this
  crate goes through it.
- **`js-sys` is a new dependency, and the reason is in `Cargo.toml`**: reading a
  JavaScript object of path to `string | Uint8Array` needs `Object::entries` and
  `Uint8Array`. It was already in the tree as `serde-wasm-bindgen`'s own
  dependency, so it adds nothing to the build. A file that is neither a string
  nor a `Uint8Array` throws, naming the path; a missing or binary *entry* file
  is a Diagnostic (`unreadable_file`) and not a throw, because that is the
  editor renaming a buffer and not a caller with a bug.
- **`assemble(source, options?)` takes `{ entry? }` and nothing else.** One
  rule: the entry path is `options.entry`, else the project's own `entry`, else
  `main.m68k` for a bare string. It is what lets the editor file one buffer under
  the name the student sees, so every Diagnostic and every Location names it.
- **`parseLine` answers a reading of the line, not the tree.** `ast::Line`
  carries every Expression of every Operand; the editor wants the Label, the
  Operation with its size, and each Operand's Addressing mode, text and columns.
  `ParsedLine` in `src/lib.rs` is that reading — `kind`, `label`, `operation`,
  `comment` — and it is a TypeScript declaration a person can keep true, which
  the whole tree would not be. `kind` is `blank`, `comment`, `label`,
  `instruction`, `directive` or `unknown`, decided by the Operation's name
  against `names::is_mnemonic` and `names::is_directive`, so a typo reads as
  `unknown` rather than as nothing.
- **A `parseLine` span is in characters, not bytes.** `LineSpan { start, end }`
  goes through `source::columns_of`, which is the conversion a Location's
  columns already use (it was lifted out of `Location::from_span`), so a span
  and a Diagnostic's columns can be compared without knowing that one counted
  UTF-8 bytes.
- **`Operand::mode_name()` is new**, and it is the serde `kind` tag of the
  Addressing mode spelled as a `&'static str` so that `parseLine` can name the
  mode without serialising the Operand.
  `every_addressing_mode_names_itself_as_it_serialises` builds one Operand of
  each of the thirteen and asserts the two agree, so they cannot drift.
- **The `Step` tuple never existed.** `ts_types` declared
  `Step = [instruction, status]` while `wasm_step` serialised a bare
  `InterpreterStatus`, so `const [_, status] = step` in the 1.4.2 wrapper read
  the second character of the string `"Running"` and
  `stepWithInterruptHandler` never saw an interrupt. `wasm_step` now returns
  `InterpreterStatus` like `wasm_run`, `wasm_step_only_status` — which existed
  to work around it — is deleted, and `Step` is gone from the declarations. The
  wrapper's `step()` answers the status, and `stepGetStatus()` stays as a
  deprecated alias so that nothing the editor calls disappears.
- **The TypeScript name of an `AssembledInstruction` is `InstructionLine`**,
  which is what the editor has always called it and what the brief for this step
  asks for; the Rust type keeps the glossary's name and `src/ts_types.rs` says
  which is which. Its shape is the one the design record specifies —
  `{ address, size, location, source }` — plus the encoded `instruction`, still
  typed `any` until someone writes the instruction declarations.
- **A field that may be missing is declared `?`, not `| null`.**
  `serde_wasm_bindgen` writes a Rust `None` as `undefined`, so
  `Diagnostic.hint`, `ExecutionStep.location` and `StackFrame.label_location`
  are optional fields; a method that answers "there is none" with an explicit
  `JsValue::NULL` — `getInstructionAt`, `getCurrentLocation` — is declared
  `| null`. `src/ts_types.rs` says so at the top.
- **The `StackFrame` declaration moved** from `src/debugger.rs` to
  `src/ts_types.rs`, so that every hand-written TypeScript declaration is in the
  one file its module comment claims; `debugger.rs` keeps a line saying where it
  went.
- **`console_error_panic_hook` is installed by `set_panic_hook()`**, called at
  each entry point as 1.4.2 called it at each method of `S68k`. It is
  `#[cfg(feature = ...)]` inside, so a build without the feature compiles, which
  1.4.2's would not have.
- **`Program` and `Interpreter` have a `dispose()`.** They are handles on
  WebAssembly memory, which garbage collection cannot reach; and
  `S68k.assemble` frees the handle itself when the source built no Program, so
  live checking on every keystroke leaks nothing. This is new: 1.4.2 leaked its
  `S68k` and `CompiledProgram` handles and nobody noticed because the editor
  kept one of each.
- **The wrapper's interpreter options default to a history.**
  `new Interpreter(program)` keeps 100 steps, which is what 1.4.2's
  `S68k.compile` did and what undo needs; `InterpreterOptions::default()` in
  Rust still keeps none, because a corpus run of six million steps wants none.
- **Eight new tests.** Seven in `src/lib.rs`'s `api_tests` — a parsed line's
  operands and their columns, the six line kinds, a span counting characters
  through a Latin-1 letter, a `text_operation` keeping its raw field,
  `parseLine` answering a shape for eight malformed lines, `ProgramInfo` and its
  camelCase Location, and an assembly with an error carrying Diagnostics and no
  Program — and the Addressing mode name test in `ast.rs`. They are serialised
  with `serde_json`, so the JavaScript contract is asserted in Rust and not only
  in the smoke test.
- **The smoke test is the chain's test.** `ts-lib/test/smoke.mjs` assembles and
  runs a program through `dist/`, reads a Program's entry point, symbols and
  their Locations, assembles the same source under `lecture/one.x68` through the
  `{ files, entry }` form, stops on a `{ file, line }` breakpoint and checks
  `getCurrentLocation` and `getNextInstruction` against it, asserts the severity,
  code, message, file, line and columns of one error and that no Program comes
  with it, asserts that a `bare_comment` suggestion still builds one, reads a
  line with `parseLine`, and keeps 1.4.2's undo and step-identity case with an
  undo step's Location added.
- **Still open, and not this step's.** A call frame's `source_address` is still
  the address the call *returns to*: renaming it is an editor-visible change to
  the Interpreter's 1.4.2 surface and the brief for 2.0 keeps that surface, so
  it is written down in `src/ts_types.rs` where the editor will read it rather
  than renamed here. `web/` — the little webpack demo — still calls `new S68k`
  against the committed 1.4.2 `pkg/` at the root: it has been dead since step 8,
  it is in no CI job, and rewriting it is a piece of work of its own. Phase 2's
  Directives, phase 3's instructions and modes, phase 4's `include` and the two
  `docs/grammar.md` 1.9 warnings are unchanged; so is the owner's question about
  `mouseWindowSize.X68`'s `END START` against a Label written `start`.

### Step 10 — phase 1 under review: twenty findings answered

Two reviewers read the whole of phase 1 (a diagnostics lens and a hygiene lens)
and raised twenty findings. Every one is answered below, with what changed or
why it did not. `cargo test` is **328 green** (6 new), `cargo fmt --check`,
`cargo clippy --all-targets` (the same 4 warnings, all older than this phase)
and `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings` are clean, and the
wasm-pack / `npm ci` / `npm run build-lib` / `npm test` chain is green from a
clean `pkg`. **Two corpus fixtures moved, one entry each**, both `-errors.snap`
of an EASy68K original; `tests/corpus/README.md` has them under "What the review
of phase 1 changed in these fixtures".

- **The Entry point is the one name read case insensitively, and ADR 0001 now
  says so.** `mouseWindowSize.X68` is byte-identical to the EASy68K
  distribution, declares `start` in column 1 and ends `END    START`: that is
  the only primary evidence there is about EASy68K's symbol look-up, the help
  claims neither way, and it says the look-up is not case sensitive. Since ADR
  0001 promises that an EASy68K program assembles here unchanged and CONTEXT.md
  says Symbols are case sensitive, the two are reconciled at the one point where
  they meet: `layout::entry_by_case` resolves `end`'s Operand — when it is one
  bare name (`Expr::as_symbol`, which had no caller until now) that resolves
  nowhere — against the Labels, ASCII case insensitively, and
  `layout::label_named_start` does the same for the `START` fallback. Everywhere
  else a name is still compared exactly, `end START+2` included, because the
  rule is about one bare name and not about Expressions.
- **`end` warns and the fallback does not**, which is a deliberate half of the
  reviewer's proposal. The `end` form raises `entry_point_case_mismatch`, a
  warning: the student wrote `START`, it is not defined, and the sentence says
  where the Entry point came from. The `START` *fallback* says nothing, because
  `START` is s68k's own convention rather than a word the program wrote — five
  of the thirty `editor/` programs declare `start:` and write no `end` at all,
  and a warning on each of them would be a diagnostic about a name nobody typed.
  All five keep the Entry point they had (their `start:` is the first
  instruction's address), so no fixture moved.
- **Two Labels differing only in case are not guessed between.** `Start:` and
  `start:` with `end START` is the ordinary `undefined_symbol`: an assembler
  that picked one would be guessing about a program that is genuinely ambiguous.
- **A Macro invocation names the Macro.** `parse_file` already skipped a Macro's
  body; it now keeps the name too — `ParsedFile::macros`, a `MacroDefinition
  { name, location }` taken from the `macro` line's Label field, which is where
  `quickStart.htm` says the name goes. `analyzer::Context` carries the slice
  (empty by default, so `Context::new` is unchanged), and `unknown_mnemonic`
  looks there first: `DELAY 1` is now `unimplemented_operation`, "`DELAY` is not
  implemented: it is a macro, and macros are not assembled yet", hinted "write
  the lines of the macro here instead", with a related Location on the `macro`
  line. The old message offered to write `DELAY:`, which would have made the
  program worse. The name is matched case insensitively, like every other
  Operation name (`docs/grammar.md` 1.3).
- **`.b` on a branch is `.s`.** `SizeRule::Branch` accepts all four suffixes:
  "EASy68K will accept .B or .S to force 1-byte offsets and .W or .L to force
  2-byte offsets" (`Reference/68ks9b.htm`, and the same sentence on `BRA`). The
  design record was silent, and silence follows EASy68K; its "Instructions"
  bullet is corrected in place and says so. `Analyzer::check_size` returns the
  Form's default for a `Branch` rule rather than the written suffix, so none of
  the four reaches the encoded instruction and `bra.b` and `bra.s` lower to the
  same `Instruction` — which is what `branch_takes_b_as_it_takes_s` asserts.
- **`address_register_byte_size` says "used", not "written".** In `cmp.b a0,d1`
  the address register is the operand *read*, and the old sentence was false for
  exactly the half its golden case did not exercise. The rejection is unchanged
  and the citations are the same (`Reference/68ks5g.htm`, `Reference/68ks4d.htm`);
  the case file now holds both directions.
- **A missing comma has its own sentence**, `missing_comma_between_operands`, a
  warning, written into `docs/grammar.md` 3.7 with the table of what fires it.
  `move.l  d0 d1` is one Operand and a bare Comment field, and the count error
  alone said nothing about where the second operand went; the `bare_comment`
  suggestion cannot, because it is once per File and is spent on the first real
  comment. The trigger is as narrow as 3.5's: the Operand count short of every
  count the Operation takes, a **bare** Comment field, and its first word both
  reading as an Operand (a register, `#`, `(`, `-(`, or a Symbol the program
  defines) and being the whole of the field — an explicit Comment may follow it,
  a word of prose may not. It is raised by the analyzer, which is the only phase
  that knows an Operand is missing; 4's preamble in the grammar says so.
- **The two evaluator warnings the design record promised are raised**, in
  `expr.rs` where the packing and the fold already live, as `ProblemKind`s that
  `diagnose` turns into Diagnostics: `character_literal_too_long` (EASy68K's
  "ASCII constant exceeds 4 characters") and `constant_above_32_bits` (its
  "Numeric constant exceeds 32 bits"). Both quote the source text, sliced from
  the line by `diagnose`, rather than the value: the whole complaint about the
  old behaviour was a message naming `#7017280452245743464`, a number nobody
  wrote.
- **The second is about a written number, not about `equ`.** `docs/grammar.md`
  1.8 and `errors.htm` both say so — "The literal number in the source file is
  too big" — so `move.l #$1FFFFFFFFF,d0` warns as well as `big equ $1234567890`
  does, and the warning lands on the literal rather than on the Directive. The
  code is `constant_above_32_bits` and not the reviewer's `constant_too_large`,
  because `number_too_large` (the 64-bit parser error) already exists and two
  codes that differ by one adjective would be read wrong.
- **A character literal that is too long suppresses the immediate range check.**
  `move.l #'abcdefgh',d0` now warns and assembles, which is what EASy68K does
  with its own warning; `expr::holds_a_long_character_literal` is what the
  analyzer asks before it range-checks (`Analyzer::check_immediate_sizes`).
- **`d8` is told how many registers there are.** `undefined_symbol`'s hint checks
  the name against the shape of a register first — a `d` or an `a` followed by
  digits that name no register — and answers "the data registers are `d0` to
  `d7` and the address registers `a0` to `a7`". No "did you mean `d0`?": `d8` is
  one edit from every one of `d0` to `d7`, so naming one would be a guess, and
  the range is the fact that was missed. The symbol table's own "did you mean"
  is unchanged for every other name.
- **The two dropped `equ` leniencies say how to rewrite themselves**, which ADR
  0001 promised and the generic hint did not do. `ValueExpected` gained an
  `advice` field, filled at the raise site by `Layout::rewrite_of_an_equate`:
  `ten equ #10` is answered with "write `ten equ 10`, and keep the `#` where the
  value is used, as in `move.l #ten,d0`" — the value is quoted from the source —
  and `r equ d1` with "`equ` names a value, not a register: write the register
  itself where `r` is used, or a `reg` list once `reg` is implemented". Every
  other Operand keeps the generic sentence.
- **The data Directives count in values, and print their size.**
  `WrongOperandCount` gained `at_least`, which the new `Layout::wrong_item_count`
  sets for `dc`: "`dc.b` takes at least one value, and this line has none",
  hinted "write the values after it, `dc.b 1,2,3`". `ds` and `dcb` keep the
  exact-count sentence and now name their size the way `value_expected` does
  (`ds.w`, `dcb.l`), which also improves their `value_expected` and
  `forward_reference_not_allowed` messages. Instructions are untouched.
- **`cargo doc` is clean and CI runs it.** The five redundant link targets and
  the two bad links in `mod.rs` and `parser.rs` are fixed, `interpreter` and
  `debugger` have module headers (which were the only two `missing_docs` in
  `lib.rs`), and the CI job has a `cargo doc --no-deps` step with
  `RUSTDOCFLAGS: -D warnings`, so the ground rule is enforced rather than
  asserted.
- **The tracked 1.4.2 `pkg/` is gone** — `git rm -r --cached pkg`, and `/pkg/`
  and `ts-lib/src/pkg/` are in `.gitignore`, so no build of the library can go
  stale in the repository again. **`web/` is kept and marked**: it is a
  349-line Monaco page against the raw 1.4.2 bindings, porting it is a piece of
  work of its own, and `web/README.md` now says what it calls, why none of it
  exists any more and what porting it needs, with `ts-lib/test/smoke.mjs` named
  as the example to copy. The root README's build instructions no longer point
  anyone at it.
- **Six unused public helpers: five deleted, one given a caller.**
  `Instruction::get_instruction_name` (whose `unwrap()` could panic and whose
  doc named callers that do not exist), `Analyzer::found_an_error`,
  `Diagnostic::with_severity`, `Token::ends_the_line` and `SourceFile::line_span`
  are gone; `Expr::as_symbol` is what the Entry-point look-up above is built on.
- **The two clippy warnings in phase-1 files are fixed** (`tokens: tokens`, and
  `.to_digit(..).is_some()`); the four that remain are all older than the phase.
- **The CLI files its source under a normalised path and prints the one the user
  typed.** A Project path is root relative (CONTEXT.md, "File"), so an absolute
  argument was normalised into a path that named no file, and the diagnostics
  and the summary line spelled the same file two ways. `src/main.rs` now files
  the source under `source::normalise_path` and maps that back to the on-disk
  path when it prints a Location, its related Locations included.
- **§2.6's rules have an index.** They belong to the Directives phase and their
  tests are named after the behaviour each rule decides, so `docs/grammar.md` 5
  gained the map from each of the seventeen rules to the tests in
  `src/assembler/layout.rs` (and, for `macro_definition`, `parser.rs`) that cover
  it. Renaming the tests would have said the same thing at the cost of a diff
  over every one of them.
- **README's tables agree with the instruction table again**: `unl` is `unlk`,
  `nop`, `extb`, `jmp`, `andi`, `ori` and `eori` are in it, and the Interrupt row
  names the tasks the Interpreter really answers (0-9, 11, 13-15, 17-20, 23, 24,
  33, 61 and 80-96) instead of "0 to 7".
- **The untracked scratch test file was already gone** when this step started —
  `src/test/scratch_fuzz.rs` does not exist and `src/test/mod.rs` declares three
  modules — so there was nothing to delete. The count it inflated is settled
  here instead: 328 tests, of which 6 are this step's.
- **The 267 undocumented public items of the Interpreter half are not this
  step's**, as the finding says. The two module headers are added, which is what
  makes `#![warn(missing_docs)]` a matter of clearing a backlog rather than of
  writing headers first; the backlog itself is a follow-up issue for whoever
  takes phase 3, since `src/instructions.rs` is where it is thickest and phase 3
  rewrites much of it.
- **Not done in this step**, and unchanged: phase 2's Directives, phase 3's
  instructions and addressing modes, phase 4's `include` and `incbin`. Two
  things the next step should know: the analyzer now takes the parsed Macro
  names through its `Context`, which is where a `macro` implementation would
  start; and `missing_comma_between_operands` only fires where an instruction's
  Operand count is short, so a Directive that takes a list (`dc.b 1 2`) still
  says nothing — deliberately, because `dc` accepts any number of items and the
  count alone cannot tell the mistake from the intent.

## Implementation notes (phase 2)

The running record of phase 2, kept the way phase 1's is: one bullet a choice,
so that the next step can read the state of the work from the repository.
Nothing above this heading is rewritten except to fix a factual error, and such
a fix says so here. **The step numbers carry on from phase 1's**, so that a
reference to "step 7" means one thing in this document.

### Step 11 — `reg`, `fail`, `simhalt` and the Directives' label rules

The first half of phase 2: the three Directives of the design record's
"implement with EASy68K meaning" bucket that need no new notion of the Layout,
and the label rules of `docs/grammar.md` 2.6. `src/assembler/layout.rs` grows
the three, the label rule and the register-list resolution;
`src/assembler/diagnostics.rs` grows five kinds; `Instruction::SIMHALT` is a new
variant of the encoded instruction and the Interpreter runs it. `cargo test` is
**349 green** (21 new), `cargo fmt --check` is clean, `cargo build
--all-targets` raises no warning, `cargo clippy --all-targets` no new one (the
same 4, all older than phase 1's step 10) and `RUSTDOCFLAGS=-D warnings cargo
doc --no-deps` is clean. **Two corpus fixtures moved, one entry each**, both
`-errors.snap` of an EASy68K original, and `tests/corpus/README.md` has them
under "What phase 2's first half changed in these fixtures".

- **`reg` defines a Symbol that holds a mask, not a number.** `SymbolKind` and
  `SymbolValue::RegisterList` were written in phase 1 and had no writer;
  `Layout::plan_reg` is it. The Operand is a `register_list` or a single
  register, which is a list of one exactly as it is for `movem`
  (`docs/grammar.md` 1.12); anything else is the new `register_list_expected`.
  The name is defined whatever the Operand turned out to be, which is the rule
  `plan_equate` already followed: a list that could not be read has been
  reported once and leaving the name undefined would report it again at every
  `movem` that uses it.
- **A `reg` name in a `movem` position becomes the list, and the substitution is
  the Layout's.** ADR 0003 gives the analyzer the comparison of an Operand with
  the instruction table, and this is not that comparison: it is a Symbol
  look-up, which is the Layout's, and what it produces is an
  `ast::Operand::RegisterList` that the analyzer then judges by its ordinary
  rules. `Layout::resolve_register_lists` rewrites the line's Operands and hands
  the analyzer the rewritten `Line`; from there the lowering, the mask, the
  predecrement reversal and the fixture printer are the written list's, because
  it *is* one. The alternative was a second question on the analyzer's
  `SymbolValues` trait and the same look-up written twice.
- **"Which position may hold a register list" is the table's answer**, not a
  test for `movem` by name: `InstructionSpec::takes_a_register_list(position)`
  is any Form whose `Modes` at that position holds `REGISTER_LIST`. Today only
  `movem` answers `true`, and it answers for both of its positions, so
  `movem.l AllRegs,-(a7)` and `movem.l (a7)+,AllRegs` both work with no
  direction logic of their own. The Operand *count* is deliberately not part of
  the question: `movem.l AllRegs` is told how many Operands `movem` takes and
  not also that a register list cannot be an Expression.
- **Four answers to a name in a register-list position, and the fourth is
  silence.** A `reg` Symbol defined above becomes the list; one defined below is
  `register_list_not_defined_yet` (EASy68K's "Register list symbol not
  previously defined") with the `reg` line as a related Location; a Symbol of
  another kind is `not_a_register_list` (its "Symbol is not a register list
  symbol"); and a name that is defined **nowhere** is left alone. The evaluator
  answers that one with `undefined_symbol` and its "did you mean", and the
  analyzer adds what `movem` takes there — two sentences that between them
  diagnose a missing `reg` line, and both of them true. Inventing a third
  message for it would have had to guess between a typo and a missing
  Directive.
- **"Not a register list" is only said where nothing else fits.** Both of
  `movem`'s positions accept a list in *some* Form, so a rule that read every
  bare name in either of them as a list would answer `movem.l table,d0-d2` —
  registers read back *from* `table` — with "`table` is a label, not a register
  list", which is a working instruction called a mistake. The two refusals are
  therefore held back unless the line has a count `movem` takes and **no Form
  of it fits as written**, which is `InstructionSpec::has_a_form_that_fits`, a
  `Form::fits` lifted out of the analyzer's own `choose_form` so that the two
  cannot disagree about what fits. The **substitution** has no such guard and
  needs none: a `reg` Symbol has no value at all, so a name that is one can
  never be the address the other direction would have allowed — which is also
  why a list defined below is refused wherever it stands, and not only where
  nothing else fits. Without that last exception a `movem.l table,d0-d2` whose
  `table` was a `reg` line further down would have lowered to nothing at all,
  silently, and the Program would have been short one instruction with no
  Diagnostic to show for it.
- **A register list is the one forward reference refused in an instruction
  Operand.** Everywhere else the rule is the design record's — allowed in an
  instruction Operand and in `dc` data, refused where the value decides the
  Layout — and a Label read before its definition is answered by pass 2. A
  register list is not a value pass 2 can fill in: it is part of how the
  instruction is encoded, EASy68K refuses it by name, and the position is
  compared with the `reg` line's Source line index. That comparison is the
  second place in the Assembler that reads a position as a line index (the
  first is `Symbol::value_at`, for `set`), so phase 4's textual `include` has
  to change both together.
- **When a name in a register-list position is refused, the Operand is not
  judged again.** `movem.l count,-(a7)` with `count equ 4` says "`count` is a
  constant, not a register list" and stops there; the mode check would add "the
  first operand of `movem` cannot be an absolute address", which is true of the
  Operand and false about the mistake. The line takes the path a line the parser
  already failed on takes — the name and the size are judged and nothing is
  lowered — which is the machinery phase 1 built for exactly this.
- **The evaluator is not asked about a name that is a register list.** Pass 2
  evaluates every Expression of every Operand so that an undefined name is named
  once; a `reg` Symbol there is `register_list_in_expression`, which is right for
  `move.l AllRegs,d0` and wrong for `movem.l AllRegs,-(a7)`, where the name is
  the Operand and not an Expression at all. The loop therefore skips a bare name
  in a register-list position that resolves to a Register list, and only that.
  `move.l #AllRegs,d0`, `move.l AllRegs,d1` and `move.l AllRegs+1,d2` all still
  get EASy68K's sentence, which is what the golden case holds.
- **`register_list_in_expression` has a case at last.** It was one of the two
  codes `src/test/diagnostics.rs` listed as unreachable from one File of source,
  because nothing could define a Register list; `unreadable_file` is now the
  only one, and phase 4's `include` owes it.
- **`fail`'s message is the raw text and the Diagnostic is the message.**
  `DiagnosticKind::UserDefinedError` renders the text the parser kept, commas,
  spaces and all (`FAIL ERROR, Argument missing in call to foo macro.` is one
  message), and EASy68K's own default when the line writes none:
  `UNSPECIFIED_FAILURE`, "Unspecified user defined error", without the "ERROR:"
  that assembler prefixes to everything and that is the `Severity` here. The
  hint is what a student needs and the help does not have: that the sentence
  comes from a `fail` line in the program and is not something the assembler
  found. Assembly carries on, as `Directives/fail.htm` says it does — the line
  after a `fail` is still laid out — and the error is what stops the Program
  from being handed out.
- **A Label on a `fail` names the address of the line**, like a Label on any
  Directive that produces nothing. The help's usage line is `[label] FAIL
  message`.
- **`simhalt` is an executable item, not an escape hatch in the Interpreter.**
  EASy68K assembles it to the object code `$FFFFFFFF`, which its simulator reads
  as halt; here it is four bytes at an even address, an
  `AssembledInstruction` like any other, and a Label on it names its address.
  The encoded form is a new `Instruction::SIMHALT`, so the two exhaustive
  matches over `Instruction` — the Interpreter's `execute_instruction` and the
  fixture printer — had to be given an arm, which is what those matches are for.
  It is the one Directive whose line reaches pass 2 for an instruction: the
  Layout's `analyze` answers `Some(Instruction::SIMHALT)` for it before the
  early return that leaves every other Directive to pass 1.
- **Resuming after `simhalt` is not offered.** "Pressing the Pause button on the
  toolbar will re-enable the simulator controls following a SIMHALT. Program
  execution may be continued with the instruction following SIMHALT"
  (`Directives/simhalt.htm`). s68k has no such control: `simhalt` ends the run
  with the status the Terminate task gives, and a terminated Interpreter stays
  terminated — `Interpreter::set_status` refuses to move it. Nor is EASy68K's
  `*[sim68k]SIMHALT_OFF` comment, which turns the same object code back into a
  Line F exception, read here: s68k assembles no `$FFFF` word a program could
  reach by accident, so there is nothing to disable.
- **`simhalt` modifies no register**, which is the help's own sentence, and the
  Interpreter's arm is one call to `set_status`. The program counter stops one
  past it, which is where the step had already left it.
- **The Layout ignores `simhalt`'s Operand field**, and that rule was
  forced by an EASy68K original. `Directives/simhalt.htm`'s usage line is `LABEL
  SIMHALT comment` and line 206 of `tests/corpus/easy68k/graphicSound.X68` is
  `SIMHALT                 Halt Simulator`; read as an ordinary Operation that is
  the Operand `Halt` and the Comment `Simulator`, because rule 4 of
  `docs/grammar.md` 1.5 ends the Operand field at the first whitespace and no
  rule of the parser may consult a Directive's arity (ADR 0003). Implementing
  the Directive would then have *added* a `wrong_operand_count` to an EASy68K
  program that ADR 0001 promises will assemble. `page`, `list` and `nolist`
  already ignore their Operand field for the same reason, and `page`'s help says
  so in as many words ("any comments are ignored").
- **The label rule is one function and three values.** `label_rule_of` in
  `layout.rs` is `Required` for `equ`, `set` and `reg`, `Forbidden` for `page`
  and the ten conditional-assembly Directives, `Optional` for everything else,
  and `Layout::check_label_rule` runs it at the head of `plan_directive`, before
  the Directive itself. `directive_needs_a_label` moved there out of
  `plan_equate`, which is why `equ`, `set` and `reg` now answer the same
  sentence from one place.
- **The Forbidden list is EASy68K's and nothing beyond it.** `page` is "No label
  is permitted" (`Directives/page.htm`) and the conditionals are "IFxx and ENDC
  directives may not be labeled" (`Directives/conditional.htm`). `macro` is
  deliberately absent: its label field holds the Macro's name. The
  structured-control keywords are absent too, because the help says nothing
  about them and silence is answered the lenient way (ADR 0001).
- **A refused Directive is still checked for its label, and that is two
  Diagnostics on one line.** `skip ifeq debug` answers both "`ifeq` takes no
  label" and "conditional assembly is not implemented yet". They are two
  mistakes at two places in the line — the label field and the operation field —
  and the label rule is a fact about the shape of the line whether or not the
  feature exists, which is why it is not held back until conditional assembly
  is. "One mistake, one message" is about one mistake.
- **A Label that is not allowed is still defined** at the address the line sits
  at. The line is already an error so no Program is built either way, and an
  undefined name would be reported again at every use of it — the rule
  `plan_equate` follows for a value it cannot work out.
- **Five new `DiagnosticKind`s**, each with a golden case in
  `tests/diagnostics/`: `label_not_allowed`, `register_list_expected`,
  `not_a_register_list`, `register_list_not_defined_yet` and
  `user_defined_error`. `directive_needs_a_label` was phase 1's and covers the
  "label required" half unchanged.
- **`docs/grammar.md` 2.6 now writes the label rule out** as a table of three
  values with EASy68K's error names beside it, says what `reg` and `simhalt` do
  that their productions cannot carry, and its rule index (5) maps
  `reg_directive`, `fail_directive` and `simhalt_directive` to the tests that
  cover them, where they used to share the "not implemented yet" row.
- **The printer rule for `simhalt` is `simhalt`**, and `tests/corpus/README.md`
  has it with the unsized instructions. `printer_rules` in `src/test/corpus.rs`
  writes the same `movem` twice, once through a `reg` name and once with the
  list written out, and asserts that the two print the same line — which is the
  evidence that a `reg` Symbol is lowered exactly as the literal list is, in
  both directions and with the predecrement mask reversal.
- **Not done in this step**, and next: phase 2's second half, `offset` and
  `section`. What it needs to know about the Layout is in the paragraph below.
  `include`, `incbin` and the refused Directives are unchanged;
  `unimplemented_reason` in `layout.rs` is down to those and to `memory`, the
  Macro Directives, conditional assembly and structured control.

**What `offset` and `section` need from the Layout as it stands.** The Layout is
two passes over one `Vec<LinePlan>`, and a plan is `{ address, scope, item }`
with `item` one of `Nothing`, `Instruction`, `Data(len)` and `Reserved(len)`.
`Layout::address` is the single current address, `Layout::origin` is the first
address anything was placed at, and `Layout::place` is the one function that
takes room, records a `Placement` for the overlap sweep and moves the address
on. Four consequences:

* **`offset` needs an address that produces no bytes.** `Directives/offset.htm`
  says "No machine code is generated by instructions or directives following an
  OFFSET directive" and that `ORG *` ends the section. Nothing in the Layout
  distinguishes "laid out" from "placed" today: `place` is what both moves the
  address and enters the overlap sweep, so an `offset` region has to skip the
  `Placement` while still moving `address` and defining its Labels. That is a
  flag on the Layout read by `place`, not a new `Item`.
* **`section` needs several current addresses.** It restores "the address
  following the last location allocated in the indicated section (or zero if
  used for the first time)", so `address` becomes one per section, sixteen of
  them, with `org` writing the current one. The overlap sweep is over all of
  them at once, which is right: "the assembler does not check for overlapping
  sections" is EASy68K's, and s68k's overlap error is a deliberate deviation
  already recorded in ADR 0001.
* **Both are pass-1 Directives with a forward-reference rule.** `offset`'s
  Expression and `section`'s number decide the Layout, so both go through
  `value_now`, which is what raises `forward_reference_not_allowed` with the
  Directive's name in it. `section`'s Operand may be a Symbol
  (`Directives/section.htm` writes `SECTION DATA` with `DATA EQU 1`), so it is
  an ordinary Expression and not a literal.
* **`section` with no Operand requires a Label** and sets it to the number of
  the current section, which is why `label_rule_of` answers `Optional` for
  `section` today: the rule depends on the Operand and not on the name, and
  `plan_section` has to raise `directive_needs_a_label` itself. The table in
  `docs/grammar.md` 2.6 says so.

### Step 12 — `section` and `offset`: sixteen counters and a region that places nothing

The second half of phase 2, and the last two Directives of the design record's
"implement with EASy68K meaning" bucket that phase 2 owns. `src/assembler/layout.rs`
grows the sixteen location counters, the `offset` region and the two planners;
`src/assembler/diagnostics.rs` grows one kind. `cargo test` is **372 green** (23
new), `cargo fmt --check` is clean, `cargo build --all-targets` raises no
warning, `cargo clippy --all-targets` no new one (the same 4, all older than
phase 1's step 10) and `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` is clean.
**No corpus fixture moved**, in either direction: no program of `tests/corpus/`
writes either Directive, so the three `-errors.snap` are exactly what step 11
left them.

- **The current address is a reading, not a field.** `Layout::address` was one
  `i64`; it is now `Layout::address()` over `sections: [i64; SECTION_COUNT]`
  indexed by `section`, shadowed by `offset: Option<i64>` while a region is
  open, and `set_address` writes whichever of the two is in force. Every
  Directive that moves the address goes through the pair and none of them knows
  which counter it is writing, which is what makes `org` "set the current
  program location" inside a section and inside a region alike.
- **Section 0 starts at `$1000` and the other fifteen at zero.** EASy68K's rule
  is "zero if used for the first time" for every section and s68k's own is the
  default origin of `$1000` (ADR 0001). The two meet at section 0, which is the
  section a program starts in — "by default, the assembler will begin with
  section 0" — and therefore the one the default origin is a statement about;
  the fifteen others are EASy68K's zero, so `section 1` with no `org` lays data
  out from 0 exactly as EASy68K does. `sections_at_the_start` is the whole of
  it, and it is the choice this bullet records.
- **An `offset` region shadows the section's counter and never writes it**,
  which is why `org *` needs nothing saved: the address "in use prior to the
  OFFSET" is the section's own counter, untouched, and closing the region
  reveals it. `resume_address` is that reading. The alternative — moving the
  address and remembering the old one — has the same behaviour and one more
  piece of state to keep true.
- **`*` is the region's counter everywhere but in an `org`.** `here equ *` and
  `dc.l *` inside a region are the offset, because that is what the current
  address means where the names are offsets; `org *` alone is the shadowed
  address, which is `Directives/offset.htm`'s own sentence and the only
  documented way to end a region. `value_now_at` takes the value of `*` as an
  argument for that one caller, and `value_now` passes `address()` for everyone
  else.
- **`place` is still the one function that takes room, and inside a region it
  takes none.** It moves the counter and returns before the `Placement`, the
  origin and the 16 MB check: no byte of the run exists, so the overlap sweep
  must not see it, the first *placed* address is still what `origin` means, and
  the counter is not an address at all — the help's own stack frame counts from
  `-3*4`, and a negative offset is the point of the Directive.
- **What a region holds back is decided once, in pass 1.**
  `hold_back_in_an_offset_region` turns the plan of a line inside a region into
  `Item::Nothing`, so a `dc` writes no `MemoryRun`, a `ds` reserves nothing and
  an instruction is never lowered. Doing it in one place after `plan_line`,
  rather than in each of the five planners, is what keeps "nothing is generated
  after an `offset`" a single rule instead of five that could disagree.
- **A line that would have produced bytes is told; a `ds` is not.**
  `no_bytes_in_an_offset_region` is the one new kind: "an `offset` region
  produces no bytes, and {an instruction | `dc.b` | `simhalt`} produces some",
  with one hint for all of them — write the fields with `ds`, and `org *` below
  them to end the region. EASy68K generates nothing there and says nothing; s68k
  says it, because bytes that reach no memory are the failure the design record
  keeps refusing (step 11's `movem` that lowered to nothing "silently" is the
  same argument). `ds` is what the region is *made of* and raises nothing at
  all.
- **The `dc` case shares the kind rather than getting one of its own.** One
  mistake — "this line produces bytes and the region does not" — said about
  whichever line made it, is one kind with the line named in it; two kinds
  would have been two ways of saying the same sentence. The golden case
  `tests/diagnostics/no_bytes_in_an_offset_region.asm` holds both, so both
  messages are read.
- **A section number outside 0–15 is `value_out_of_range` and not a kind of its
  own.** It is exactly what that kind is for and what the address of an `org`
  and the count of a `ds` already use it for: the subject is "the number of
  `section`", the range is the sixteen, and the advice says there are no more.
  A new code would have added a catalogue entry that says nothing the shared one
  does not.
- **The section in force does not change when the number is refused**, and the
  region is *not* closed either: a line that was not understood moves nothing,
  so the lines below it are laid out where they would have been and the student
  reads one mistake instead of a file of consequences.
- **An `offset` region opens whatever its Expression turned out to be.** That is
  the rule `equ` follows for a value it cannot work out, for the same reason: a
  region that did not open would lay the whole table into memory and answer one
  mistake, already reported, with an `address_used_twice` or a stray `MemoryRun`
  at every line below it. `offset` with no Operand at all opens at 0 for the
  same reason.
- **A name in the label field of a region is a Constant, not a Label.** Its
  value is an offset into a structure; there is no line of the program at it, it
  may be negative, and calling it a Label would put an address that does not
  exist into the symbol listing, into the asm-editor's symbol view and into the
  corpus fixture's `labels` (which clamps a negative address to `$0`). It is
  still the label field, so a Global one still opens a scope for the Local names
  under it. `SymbolKind` keeps its four kinds and CONTEXT.md's glossary is
  unchanged: a Constant is "a name for a value", which is what an offset is.
- **`section` with no number defines a Constant too**, and for the same reason:
  "will be set to the value of the current section (0..15)" is a number. It is
  the one label rule that cannot be read off the Directive's name, so
  `label_rule_of` still answers `Optional` for `section` and `plan_section`
  raises `directive_needs_a_label` itself, which is what step 11's note said it
  would have to do.
- **A Label on a `section` or an `offset` line names where the line leaves the
  address**, which is the rule a Label on an `org` follows: on a `section` with
  a number it is the address that section goes on from, and on an `offset` it is
  the offset the region starts at.
- **`section` ends an `offset` region, and that is a choice.** The help names
  only `org`; a `section` sets the current address exactly as an `org` does, so
  it ends the region, and the alternative — refusing the line — would refuse a
  program EASy68K assembles, which ADR 0001 does not allow without a reason.
  `end` ends a region too, for a plainer reason: nothing after it is assembled,
  and a Label on the `end` line is an address and not an offset.
- **An `org` that moves nothing says nothing, which is a fix to step 7's odd
  origin.** `org *` after a `dc.b` restores an odd address, and the phase-1 rule
  "an odd `org` warns and rounds up" would both warn about it and move the code
  a byte — a warning about an address the program was legitimately at, and a
  silent shift of the line after it. The rule is now "an odd `org` that *moves*
  the address warns and rounds up"; an `org` that lands where the address
  already is is a no-op. Nothing in the corpus writes `org *`, no fixture moved,
  and `an_org_that_moves_nothing_says_nothing_about_an_odd_address` holds it.
- **Alignment rounds *up* from a negative address.** `align` used
  `address % alignment`, which for `-11` moves to `-8`; it is `rem_euclid` now,
  which moves to `-10`. Only an `offset` region can hold a negative current
  address, so nothing else could have seen the difference, and the addition
  saturates because only a region's counter can be near the end of the 64 bits
  an Expression is computed in.
- **The overlap sweep is over every section at once**, unchanged, which is what
  step 11's note said it would be: EASy68K "does not check for overlapping
  sections", s68k's check is the deviation ADR 0001 already records, and an
  address is an address whichever section wrote it. `two_sections_over_one_address_are_still_an_overlap`
  is the test.
- **Two fixture-style tests hold the help's own examples**, in
  `src/test/corpus.rs` beside `printer_rules`, with every address worked out by
  hand in the test: `the_section_example_of_the_help` (`msg1` at `$2000`, code
  at `$1000` in section 0, `msg2` at `$200e` because section 1 went on from
  where it stopped) and `the_offset_stack_frame_example_of_the_help` (`num1`,
  `num2`, `num3` at `-12`, `-8`, `-4`, no memory and no Labels at all, and the
  five instructions from `$1000` carrying the offsets as their displacements —
  `move.l #$11111111,-12(a0)`).
- **`unimplemented_reason` is down to `include`, `incbin`, `memory`, the Macro
  Directives, conditional assembly and structured control.** Phase 2's list of
  Directives is finished: `end`, `set`, `reg`, `fail`, `simhalt`, `offset` and
  `section` all do what EASy68K does, and `include` and `incbin` are phase 4's.
- **Not done, and deliberately.** A region that is never ended is not reported:
  every line inside it that meant to produce bytes is already told one by one,
  and a File whose last line is inside a region has nothing left to warn about.
  A second `offset` inside a region simply moves the counter, as EASy68K's
  location counter would. Neither is in the help.

**What phase 3 needs to know.** The Layout of phase 2 is: a `LinePlan` of
`{ address, scope, item }` per line, `Item` one of `Nothing`, `Instruction`,
`Data(len)` and `Reserved(len)`; sixteen section counters read through
`address()`; an optional `offset` counter that shadows them; `place` the one
function that takes room, and `hold_back_in_an_offset_region` the one that
decides a line produces nothing after all. Three consequences for the
instructions:

* **An instruction's size is still `INSTRUCTION_SIZE`, four bytes**, and pass 1
  gives it its address before pass 2 knows which Form it is. A phase 3 that
  wants real sizes has to work them out in pass 1, from the Operands alone,
  because the address of the next line depends on it — that is a change to
  `plan_instruction` and to nothing else, and `AssembledInstruction::size`
  already carries the number.
* **`Instruction::SIMHALT` is a variant of the encoded instruction**, so the two
  exhaustive matches over `Instruction` — the Interpreter's
  `execute_instruction` and the fixture printer — have an arm that is not a
  68000 instruction at all. A phase 3 that adds variants adds them beside it.
* **Nothing in the Layout reads a line index as a position except two places**,
  and they are still the two step 11 named: `Symbol::value_at` for `set`, and
  the register-list forward-reference check. Phase 4's textual `include` changes
  both together; phase 3 changes neither.

## Implementation notes (phase 3)

The running record of phase 3, kept the way phase 1's and phase 2's are: one
bullet a choice, so that the next step can read the state of the work from the
repository. Nothing above this heading is rewritten except to fix a factual
error, and such a fix says so here. **The step numbers carry on**, so that a
reference to "step 11" means one thing in this document.

### Step 13 — the status register, and `movep`, `tas`, `rtr`, `chk`, `trapv`, `illegal`

The first half of phase 3: the SR model of the design record's "Instructions"
and the first group of Mnemonics the instruction table carried a "not
implemented" reason for. `src/interpreter.rs` grows the register, sixteen
execution arms and three runtime errors; `src/assembler/instructions/` grows sixteen
encoded instructions, six Families and the Forms that name `sr` and `ccr`;
`src/assembler/analyzer.rs` loses the check that refused them. `cargo test` is
**393 green** (21 new since step 12's 372), `cargo fmt --check` is clean, `cargo
build --all-targets` raises no warning, `cargo clippy --all-targets` no new one
(the same 4, all older than phase 1's step 10) and `RUSTDOCFLAGS=-D warnings
cargo doc --no-deps` is clean. The `ts-lib` chain was run — `wasm-pack build`,
`npm run build-lib`, `npm test` — because `src/ts_types.rs` and
`ts-lib/src/index.ts` both changed. **One corpus fixture moved, by one entry**,
`mouseWindowSize-errors`, and `tests/corpus/README.md` has it.

#### The status register

- **The SR is the CCR with a byte on top of it, and the CCR keeps its own bits.**
  `Cpu` gains `system_byte: u8` beside the `ccr: Flags` it always had;
  `Cpu::get_sr` is `(system_byte << 8) | ccr.to_ccr_byte()` and `Cpu::set_sr`
  splits it again. The alternative — storing one `u16` and reading the flags out
  of it — would have moved the bits the editor reads:
  `Flags` is a `bitflags` whose carry is `1 << 1` and whose extend is `1 << 5`,
  one place to the left of the processor's own numbering, and
  `wasm_get_flags_as_number` has answered those bits since 1.4.2.
  `Flags::to_ccr_byte` and `Flags::from_ccr_byte` are the one conversion, and
  every instruction that reads or writes `ccr` goes through them.
- **`$2700` is where a program starts**, `INITIAL_STATUS_REGISTER`, which is
  EASy68K's ("When the simulator starts up the supervisor bit is set on",
  `SIMHELP/Exceptions.htm`) and the design record's. Supervisor set, interrupt
  mask 7, trace clear, no condition code set. It has **no effect on anything**:
  nothing in the Interpreter reads the system byte, which is what makes
  `andi.w #$00,SR` — line 66 of `mouseWindowSize.X68`, commented "put CPU in User
  mode" — a line that assembles, runs and changes nothing that matters.
- **Undo restores the whole register.** `ExecutionStep` gains `old_sr` and
  `new_sr`, the whole 16-bit register before and after the step, and
  `Interpreter::undo` restores through `set_sr` where it used to assign
  `cpu.ccr`. `old_ccr` and `new_ccr` stay exactly what they were — the same
  flags in this crate's own bits, which the editor already reads and which the
  `ts-lib` wrapper still converts from the string `bitflags` serialises — so the
  overlap is deliberate and both are documented as such. A mutation kind
  (`WriteStatusRegister`) was the other option; it would have put a new variant
  in the `MutationOperation` union the editor draws, for state every step
  already carries.
- **Four getters and no setter cross the boundary.** `Interpreter::get_sr`,
  `Interpreter::wasm_get_sr` and `Cpu::wasm_get_sr` in Rust and wasm,
  `Interpreter.getSr()` and `Cpu.getSr()` in `ts-lib` (the second so that a
  snapshot of the registers carries the register too); `set_sr` is public in
  Rust because the execution arms and undo need it, and is not exposed, because
  a program's status register is the program's. The flag getters (`getFlag`,
  `getFlagsAsArray`, `getFlagsAsBitfield`) are untouched and answer what they
  always did. `debug_status` prints an `SR: 0x2700` line above the flags, and
  the smoke test steps a three-instruction program through `MOVE to SR`, `MOVE
  from SR` and `ANDI to CCR` and undoes it.

#### The table, and how a Form names a half of the register

- **`sr` and `ccr` are `Modes` of their own** (`Modes::SR`, `Modes::CCR`,
  `Modes::STATUS` for the two together), so the instruction table answers "which
  position takes one" the way it answers every other question, and
  `check_operand_is_implemented` loses the two arms that refused them. `usp` is
  deliberately **not** a mode: `move usp,an` is not implemented, the analyzer
  says so before it looks at a Form, and a mode that is never allowed anywhere
  would only turn up in "there it takes …" lists as an offer that is a lie.
- **The Forms are the reference's, position by position.** `move <ea>,ccr` and
  `move <ea>,sr` take any **data** addressing mode, an immediate included, and
  are a word; `move sr,<ea>` writes any **data alterable** one, also a word
  (`Reference/68ks4d.htm`, which lists the modes for each of the three). `andi`,
  `ori` and `eori` gain a `[Im, ccr]` **byte** Form and a `[Im, sr]` **word**
  one — "Operations that uses the status register (SR) and the flag register
  (CCR) can only work with word and byte" (`Reference/68ks6b.htm`) — and
  `addi`, `subi` and `cmpi` gain nothing, because the 68000 has no such
  instruction and `addi.w #1,sr` is an `invalid_addressing_mode` naming what
  `addi` does take.
- **`move ccr,<ea>` is assembled although the 68000 has no such instruction.**
  It is the 68010's; the help documents `MOVE to CCR`, `MOVE to SR` and `MOVE
  from SR` and not this one. The design record's "Instructions" asks for `move`
  "to and from SR and CCR", ADR 0001's direction is the lenient one, and reading
  the condition codes back is worth more to a student than the distinction. It
  is written down here, in `README.md` and in the doc comment of the encoded
  variant; nothing else in s68k assembles an instruction the 68000 lacks.
- **A written `sr` or `ccr` narrows the Forms before the first-fit rule runs.**
  `move` has five Forms of two Operands now, and "the first that fits, else the
  first of that arity" would have answered `move a0,sr` with "the second operand
  of `move` cannot be the status register", which is the wrong half of the line.
  `Analyzer::choose_form` keeps the Forms that **agree** about every `sr` and
  `ccr` that was written — `agrees_about_the_status_register` — and only then
  applies the old rule, so `move a0,sr` is judged against `[data, sr]` and reads
  "the first operand of `move` cannot be an address register … there it takes
  Dn, (An), …, Im". A position that names a half of the register names nothing
  else, which is what makes the agreement exact and which a test in `table.rs`
  holds every row to. When nothing agrees (`move sr,ccr`) every Form of that
  arity is a candidate again and the fallback answers as it always did.
- **`invalid_size` names the chosen Form's sizes, not the Mnemonic's.**
  `move.b d0,ccr` was answered with "`move` takes `.b`, `.w` or `.l`", the union
  over five Forms, which offers the size it has just refused. It now reads
  "`move` takes `.w`". The change is visible on the memory form of a shift too
  (`asl.b (a0)` is answered with `.w` where it used to be answered with all
  three), which is the same improvement: the shape being judged is the one the
  Operands chose. `check_size_alone`, which runs when no Form was chosen at all,
  still names the union.
- **A half of the status register where none is allowed says who takes one.**
  `tst.w ccr` is the ordinary `invalid_addressing_mode`, with the suggestion
  "only `move`, `andi`, `ori` and `eori` reach the status register" — the same
  shape as "only `movem` takes a register list", which is ADR 0003's "what was
  probably meant" for a mode that is right nowhere near this Mnemonic. It is
  said only by a Mnemonic that reaches the register in **no** Form, so
  `move ccr,ccr` is told what `move` takes in each position and is not told that
  `move` is one of the four, which it is.
- **`movep` has exactly two Forms and no more**, `[Dn, d(An)]` and `[d(An), Dn]`
  (`Reference/68ks4g.htm`: "ADDRESS METHODS: x(An)"), so `movep.w d0,(a1)` is
  refused — and, because that is the mistake with a name, `suggestion_for`
  answers it with "write the displacement, `0(a1)`".
- **The other five rows are the reference read straight off**: `tas <ea>` is
  data alterable and a byte; `chk <ea>,Dn` is a data mode and a word; `rtr`,
  `trapv` and `illegal` take no Operand and no size. `move usp,an` and
  `move an,usp` stay unimplemented, with `usp`'s own reason ("s68k runs one
  program with one stack pointer, `a7`"), which is the one arm left in
  `check_operand_is_implemented` beside the PC-relative modes.

#### Lowering, and the encoded instruction

- **Sixteen new `Instruction` variants, one per encoding the 68000 has.**
  `MOVEP`, `MOVEtoCCR`,
  `MOVEfromCCR`, `MOVEtoSR`, `MOVEfromSR`, `ANDItoCCR`, `ORItoCCR`, `EORItoCCR`,
  `ANDItoSR`, `ORItoSR`, `EORItoSR`, `TAS`, `RTR`, `CHK`, `TRAPV`, `ILLEGAL`.
  The immediates carry the width of their destination (`u8` for `ccr`, `u16` for
  `sr`) rather than a `Size`, and the four `move`s carry none at all, because
  every one of them is a word by definition. The names are the manual's ("ANDI
  to CCR") in the one spelling that is not `non_camel_case_types`.
- **`sr` and `ccr` never become an encoded `Operand`.** Adding them there would
  have put an arm in `get_operand_value`, `store_operand_value`,
  `get_operand_address` and the fixture printer for a thing that is not an
  addressing mode. `lowering::lower_operation` is the new entry point the
  analyzer calls: it answers the status-register shapes from the **tree**
  (`lower_status_register`, which reads the two Operands and the `Family`) and
  hands everything else to the `lower` that already existed, unchanged. That is
  also why `lower_operand` still answers `None` for a special register.
- **`movep`'s direction is read off the Operands**, as `movem`'s is: a data
  register first is `ToMemory`, second is `FromMemory`. `TargetDirection` is
  reused rather than a second two-valued enum being written.

#### Running them

- **`movep` walks every second byte, most significant first** — the bytes at
  `d`, `d+2`, `d+4`, `d+6` — and touches no flag. A `.w` writes or reads the low
  word of the register and leaves the rest of it alone, which is
  `set_register_value` with `Size::Word` and needs nothing of its own.
- **`tas` sets the flags from the byte *before* it writes.** `set_logic_flags`
  is exactly the help's table (N and Z from the value, V and C cleared, X kept),
  and the store is `value | $80` through `Used::Twice`, so `tas (a0)+` walks its
  register once like every other read-modify-write instruction here.
- **`rtr` is a return.** It pops the word, keeps its low byte as the condition
  codes, pops the return address, and — like `rts` — records a `PopCall`
  mutation and pops the debugger's call stack, so the call stack and undo stay
  true for a subroutine that ends with `move sr,-(sp)` … `rtr`.
- **An exception ends the run, and that is the whole of the model.**
  `Interpreter::end_with_an_exception` sets `TerminatedWithException` and answers
  the error; `chk` outside its bounds, `trapv` with V set and `illegal` are its
  three callers. A 68000 would build a stack frame and jump through the vector at
  `$18`, `$1C` or `$10` (`SIMHELP/Exceptions.htm`); s68k has no vectors, no
  supervisor stack frame and no `rte` to come back from one, so the honest thing
  is to stop where an address error already stops. `chk` sets N on the way out —
  set when the register is below zero, cleared when it is above the bound, which
  is the help's own sentence — and leaves the flags the help calls undefined
  alone.
- **Three new `RuntimeError` variants**, `ChkOutOfBounds { value, bound }`,
  `OverflowException` and `IllegalInstruction`, each naming its instruction, and
  the TypeScript `RuntimeError` union in `src/ts_types.rs` grew the same three.
  The value and the bound are carried as signed numbers, because that is how
  `chk` compares them and a message about "11 is not within 0 to 10" is the
  whole diagnosis.
- **The command line names the line that failed.** `print_runtime_error` in
  `src/main.rs` prints the error and, under it, the file, the line and the
  source of the instruction being executed, mapped back to the path the user
  typed the way the Diagnostics already are. A runtime error is not a Diagnostic
  (CONTEXT.md), but a student reading `Runtime error: ChkOutOfBounds { value: 11,
  bound: 10 }` deserves to be told where.

#### Diagnostics, fixtures and documentation

- **`docs/grammar.md` is untouched, and that is the ADR 0003 point.** The
  parser has read `sr`, `ccr` and `usp` at any position of any Operation since
  step 4 (1.10), and everything this step decided about them is the analyzer's
  and the table's. A phase that had to change the grammar to add an instruction
  would be a phase that had put the instruction table in the parser.
- **"Yet" is now a field, not a habit.** `unimplemented_addressing_mode` said
  "which s68k does not assemble **yet**" of every mode it refused, and with `sr`
  and `ccr` gone from that list the only special register left is `usp`, which
  is out for good (the design record, "Scope"). The kind gains
  `planned: bool` — true for the PC-relative modes, which are in the plan, false
  for `usp` — and the sentence drops the word when there is nothing to wait for.
  It is the same distinction `unimplemented_operation` already made in prose,
  where `rte`'s reason says no "yet" and `roxl`'s does.
- **No new `DiagnosticKind`.** Every rejection this step can produce already had
  a kind that says the right thing: `invalid_addressing_mode` with the Forms
  above, `invalid_size` with the Form's own sizes, `wrong_operand_count`,
  `immediate_out_of_range` against the byte of a `ccr` immediate. `WITHOUT_A_CASE`
  is still `unreadable_file` alone, and phase 4 owes it.
- **Two golden cases changed, because their lines stopped raising their code**:
  `tests/diagnostics/unimplemented_addressing_mode.asm` traded `move.w sr,d0`
  for `move.l usp,a0`, and `unimplemented_operation.asm` traded
  `movep.w d0,4(a0)` for `roxl.w #1,d0`. Both snapshots were regenerated and
  read; `tests/corpus/README.md` records both.
- **One corpus fixture moved**: `mouseWindowSize-errors` loses its
  `unimplemented_addressing_mode` and is 19 entries where it was 20. Nothing else
  did — no `editor/` program writes any Mnemonic of this group, and the summary
  at the top of `tests/corpus/README.md` now says "15, 3 and 19" and no longer
  lists `sr` among the features the three originals name.
- **The printer grew sixteen arms and three rules**, written into
  `tests/corpus/README.md`: `sr` and `ccr` as Operands, no size on the ten
  instructions that name one (the destination is the width), and `movep` with
  its size and its direction. `printer_rules` in `src/test/corpus.rs` writes all
  twenty new lines and asserts them, since no corpus program does.
- **Twenty-one new tests.** Three in `analyzer.rs` — every status shape
  assembling with nothing said, the Form choice on `move a0,sr` and `move sr,#5`
  with the size rules of `andi` to each half, and `movep`'s displacement with
  its message — and eighteen in `src/test/test.rs` under `the_status_register`
  and `movep_tas_rtr_and_the_exceptions`: the initial
  `$2700`, `move` to and from both halves, the help's own "clearing the flag
  register does not set Z", the three immediates, undo putting the system byte
  back, `movep` both ways with the reference's `$12345678` pattern and its
  untouched flags, `tas` on `0` and on `$80`, `rtr` restoring a caller's flags
  and returning, `chk` in range, above and below, `trapv` both ways, and
  `illegal`.

**What the second half must know.**

* **Instruction size is still `INSTRUCTION_SIZE` = 4**, and this step did not
  touch it. The handover of phase 2 stands word for word: real sizes have to be
  worked out in pass 1 from the Operands alone, in `plan_instruction`, because
  the next line's address depends on it.
* **A new `Instruction` variant needs four arms**, and the compiler asks for
  three of them: `execute_instruction`, the fixture printer in
  `src/test/corpus.rs` (both exhaustive, no catch-all), the printer rules of
  `tests/corpus/README.md` (which no compiler checks), and `lower`.
* **A Mnemonic whose Operand is not an addressing mode goes through
  `lower_status_register`**, not through `lower_operand`: that is the pattern to
  copy if anything else ever takes a register the encoded `Operand` has no form
  for.
* **`Analyzer::choose_form` now has two stages**, and a new Form that names
  `sr` or `ccr` beside an ordinary mode in one position would break the first
  one; `table.rs` fails the build if a row does that.
* **What is left of the design record's "Instructions"**: `addx`, `subx`,
  `negx`, `abcd`, `sbcd`, `nbcd`, `roxl`, `roxr` — the extend-flag and
  binary-coded-decimal group — and the PC-relative addressing modes, which are
  the analyzer's `unimplemented_addressing_mode` and a `Modes` flag each.
  `rte`, `stop`, `reset` and `move usp,an` stay out for good, with the reason
  each carries.

### Step 14 — the extend flag and binary coded decimal: `addx`, `subx`, `negx`, `roxl`, `roxr`, `abcd`, `sbcd`, `nbcd`

The second half of phase 3, and the last group of Mnemonics the instruction
table carried a "not implemented" reason for. `src/assembler/instructions/`
grows seven encoded instructions, four Families and a `ShiftKind`;
`src/math.rs` grows the five pieces of arithmetic they need;
`src/interpreter.rs` grows seven execution arms and three flag helpers;
`src/assembler/analyzer.rs` grows the one new Diagnostic of the step. `cargo
test` is **416 green** (23 new since step 13's 393), `cargo fmt --check` is
clean, `cargo build --all-targets` raises no warning, `cargo clippy
--all-targets` no new one (the same 4, all older than phase 1's step 10) and
`RUSTDOCFLAGS=-D warnings cargo doc --no-deps` is clean. The `ts-lib` chain was
**not** run and did not need to be: `src/lib.rs`, `src/ts_types.rs` and
`ts-lib/` are untouched, because an `Instruction` crosses to JavaScript as
`instruction: any` and no `RuntimeError` was added. **No corpus fixture moved**,
and `tests/corpus/README.md` says so.

#### The table: two Forms that are two whole shapes

- **`addx`, `subx`, `abcd` and `sbcd` take `Dy,Dx` or `-(Ay),-(Ax)` and nothing
  else**, which is the reference read literally ("ADDRESS METHODS: Dn, -(An)",
  `Reference/68ks5e.htm`, `68ks5v.htm`, `68ks8e.htm`, `68ks8g.htm`). That is two
  Forms of two Operands each — `EXTENDED_PAIR_FORMS` at `.b`/`.w`/`.l` and
  `DECIMAL_PAIR_FORMS` at `.b` — and it is the first time the table holds a
  Mnemonic whose Forms are two *whole shapes* rather than two positions: every
  earlier pair of same-arity Forms (`cmp`, `movem`, `move`, `movep`, the three
  immediates) differs in one position at a time. The consequence is the new
  Diagnostic below.
- **`negx` and `nbcd` are one data-alterable Operand**, `negx` at any size and
  `nbcd` at a byte (`Reference/68ks5q.htm`, `68ks8f.htm`), which is `neg`'s row
  and `clr`'s with the size rule changed.
- **`roxl` and `roxr` are a `ShiftKind`, not a Family.** They take the three
  shapes of the other six shifts — `#count,Dn`, `Dx,Dy` and one memory word —
  and the same 1-to-8 range on a written count, so they are `shift("roxl",
  ShiftKind::RotateExtend, ShiftWay::Left)` and share `SHIFT_FORMS` and the
  `value` rule with `asl` and `rol`. Only the encoded instruction differs. The
  help's own "when rotating in the memory, you can only use word" is
  `SizeRule::WordOnly` on that Form, already there for the others.
- **Four Families and one `ShiftKind`**: `AddSubExtended { subtract }`,
  `AddSubDecimal { subtract }`, `NegExtended`, `NegDecimal` and
  `ShiftKind::RotateExtend`. The `{ subtract }` flag is the shape `Family::AddSub`
  already had, so `addx`/`subx` and `abcd`/`sbcd` are one arm each in the
  lowering.
- **`unimplemented_operation` is down to three Mnemonics.** Step 6 gave
  seventeen of them a reason and said fourteen of those said "yet", "because
  phase 3 adds them and this is the sentence phase 3 deletes". All fourteen are
  deleted now — six in step 13 and eight here — and the three left are `rte`,
  `stop` and `reset`, whose reasons never said "yet" because they are about the
  machine s68k simulates. No *row* of the table says "yet" any more; the only
  "yet" the analyzer still writes about an instruction is
  `unimplemented_addressing_mode`'s, for the PC-relative modes, and the
  Directives keep theirs until phase 4.
- **Two table tests moved and one is new.**
  `a_refused_instruction_is_in_the_table_with_its_reason` is down to `rte`,
  `stop` and `reset`, which are the whole of what is refused for good;
  `every_implemented_row_has_a_form_and_every_form_a_rule_per_operand`'s list of
  Mnemonics allowed two Forms of one arity grew `addx`, `subx`, `abcd` and
  `sbcd`; and `the_extend_flag_group_takes_what_the_reference_gives_it` holds
  all eight rows to the shapes and the sizes of their reference pages, so a row
  edited by hand fails the build.

#### `invalid_operand_pair`, the one new Diagnostic

- **A position is the wrong thing to name when the shapes are the mistake.**
  `addx d0,-(a1)` is wrong in neither Operand on its own: judged against the
  register Form it reads "the second operand of `addx` cannot be a predecrement
  operand", of a line whose fix is to make the *first* one a predecrement too.
  And `addx (a0),(a1)` is wrong in both positions, so the per-position check
  said the same thing twice. The new kind is **one Diagnostic over both
  Operands**: "`addx` takes two data registers or two predecrement operands, and
  this line has a data register and a predecrement operand", hinted "write `addx
  d0,d1` or `addx -(a0),-(a1)`; `add` takes every addressing mode". That is ADR
  0003's three parts — what was found, what is allowed, what was probably meant
  — with the found half built from `Operand::description()`, which every other
  message already uses.
- **It is raised in `Analyzer::check_operand_pair` and suppresses the
  per-position checks.** The function answers `true` when it has spoken, and the
  Operands are then not judged one by one as well, nor asked about their values:
  one mistake, one message, which is the rule the rest of `check` follows.
  `the_advice_of_a_pair` is what tells the four Mnemonics apart from the rest of
  the table — it reads the `Family` and answers the last clause of the hint —
  so nothing here is keyed on a Mnemonic's spelling.
- **The advice is `add` and `sub` for the extend pair and "move the byte into a
  data register first" for the decimal one**, because `abcd` and `sbcd` have no
  counterpart that reaches memory: decimal arithmetic on the 68000 is those two
  and `nbcd`.
- **Everything else the group can get wrong already had a message.** A size is
  `invalid_size` against the chosen Form ("`abcd` takes `.b`", "`roxl` takes
  `.w`" on the memory form), `negx a0` is the ordinary
  `invalid_addressing_mode` with the address-register suggestion, `roxl #9,d0`
  is `value_out_of_range` against the shift count, and a wrong Operand count is
  `wrong_operand_count`. `WITHOUT_A_CASE` in `src/test/diagnostics.rs` is still
  `unreadable_file` alone, and phase 4 owes it.
- **The golden case is `tests/diagnostics/invalid_operand_pair.asm`**: the three
  reachable shapes of the mistake (neither operand, and each half of a shape the
  other half does not finish), and both correct shapes below them, which raise
  nothing.

#### The encoded instructions

- **Seven new `Instruction` variants**: `ADDX`, `SUBX` and `NEGX` carry a
  `Size`; `ABCD`, `SBCD` and `NBCD` carry **none**, because a byte is the only
  size they have — the same choice `TAS` made in step 13, and the printer rule
  that follows from it; `ROXd` has the shape of `ASd`, `LSd` and `ROd`, so
  `lower_shift` gains a fourth arm and nothing else moves.
- **`lower_operation` was not touched.** Every Operand of this group is an
  ordinary Addressing mode, so `lower_operand` answers all of them and the
  `lower_status_register` path of step 13 is not involved.

#### Running them, and the flag rules

- **The Z flag is the rule of the group's arithmetic, and it has a helper of its own.**
  `clear_zero_if_the_result_is_not_zero` sets Z to false when the result is not
  zero and **touches nothing when it is**, which is what makes a number of any
  width testable in one pass: set Z, work up from the least significant piece,
  read Z at the end ("The Z flag works in another way now… You must set the zero
  flag before making the addition though", `Reference/68ks5e.htm`). **Six of the
  seven arms go through it and `roxl`/`roxr` do not**: their pages give Z as
  "S", set from the result like every other shift's, because a rotate is not
  multi-precision arithmetic — it is the one instruction of the group that
  carries the extend flag without carrying the rule that goes with it. The
  pinning test is
  `the_zero_flag_is_cleared_by_a_result_that_is_not_zero_and_never_set`, and it
  holds the rule in all three directions: a 64-bit sum in two longs whose
  **high half comes out zero while the sum is 2** (Z must stay cleared — the
  case an implementation that sets Z from its own result gets wrong), one whose
  low half comes out zero while the sum is not (Z is cleared again by the high
  half), and one that is zero all through (Z survives).
- **Three rows of the help disagree with the rule this group's arithmetic
  needs, and they are recorded here.** *(This bullet said "one typing slip" and
  named only the first of the three; the other two were found by the review of
  phase 3 and are added here, which is the correction.)*
  `Reference/68ks5q.htm` gives `NEGX`'s Z flag as "Set if the result is not
  zero, else unaffected"; `ADDX`'s own flag table on `68ks5e.htm` gives it as
  "Z - S", set from the result, while the prose two paragraphs above it on that
  same page says the opposite ("You must set the zero flag before making the
  addition"); and `NBCD` on `68ks8f.htm` reads "Cleared if the result was 0,
  else unaffected", the inverse of `ABCD`'s and `SBCD`'s. What the other pages
  state — `SUBX` on `68ks5v.htm`, "Cleared if the result is not zero, else
  unaffected", and `ABCD` on `68ks8e.htm`, "Cleared if the result is NOT zero.
  Unaffected else" — is the rule `ADDX`'s own prose describes and the only one
  that makes a multi-precision sum testable, and s68k implements it for all
  six. The reason is a comment on the `NEGX` arm as well, where somebody
  comparing it with the help will look.
- **X and C are set alike, always** ("C - Same as X" on all eight pages), and N
  and V are the result's for `addx`, `subx` and `negx` —
  `set_extended_arithmetic_flags` is those five bits in one place. The overflow
  is `has_add_overflowed`/`has_sub_overflowed` over the result that already has
  the extend flag in it, which is the 68000's own definition, and the test shows
  it working both ways: `127 + 0 + X` overflows a byte where the addition alone
  would not, and `-128 + -1 + X` does **not** overflow where the addition alone
  would.
- **The decimal three leave N and V exactly where they were.** The help calls
  both undefined for `abcd`, `sbcd` and `nbcd`, and s68k does not invent a value
  for an undefined flag — the same choice step 13 made for the flags `chk`'s
  page calls undefined. So `set_decimal_flags` writes X, C and Z and nothing
  else, and a test asserts that N and V come out of an `abcd` holding what a
  `move` to `ccr` put there. Setting them from the result was the alternative
  (it is what the hardware happens to do); leaving them says "this instruction
  does not answer that question", which is the honest thing for a teaching tool
  and cannot be mistaken for a promise.
- **The decimal arithmetic is the 68000's correction and not a digit-by-digit
  sum.** `add_decimal` and `subtract_decimal` in `src/math.rs` add or subtract
  the low digits, correct by 6 when they pass 9 (which carries a ten into the
  high digits), then the high digits, and correct by `$a0` when the byte passes
  99, which is the carry out. Two well formed BCD bytes give the decimal answer
  either way; a byte holding a digit above 9 gives what the hardware gives,
  which the help says nothing about, and that is why the hardware's version was
  written rather than the tidier one.
- **`nbcd` is `subtract_decimal(0, value, extend)`** and has no arithmetic of
  its own: the tens complement *is* zero less the value, which is why "the tens
  complement to 01 is 99" and why a second `nbcd` under a borrow gives 73 and
  not 74 (`Reference/68ks8f.htm`, and the test says so in as many words).
- **A rotate through the extend flag is a loop over `rotate_with_extend`**, one
  place at a time, `count % 64` places — the same shape as `ROd`, and never more
  than 63 iterations. `set_logic_flags` gives N, Z and a cleared V, and X and C
  are then both set to the bit that came out. **With a count of zero X is left
  alone and C answers it**, which falls out of the loop not running and is the
  one place a rotate's carry is not a bit it moved ("Unaffected if rotation step
  was zero", and "C - Same as X").
- **The predecrement forms read the source first**, which is what makes
  `addx -(a0),-(a1)` walk both registers down in the 68000's order; the
  `Used::Once`/`Used::Twice` pair that every read-modify-write instruction here
  already uses does the rest, and a test asserts both registers and the two
  longs in memory afterwards.
- **Twenty-one new interpreter tests**, in `src/test/test.rs` under
  `the_extend_flag_and_binary_coded_decimal`: every example the eight reference
  pages give (`ADDX D0,D1`, `SUBX.B D0,D1`, `NEGX` of 2 with X set giving
  `0000FFFD`, `ROXL.B #1,D0` and `ROXR.B #1,D0` with X set and clear, `ABCD` and
  `SBCD` of two BCD bytes, the tens complements of 01 and 26), the two
  multi-precision walks through memory (a 64-bit `addx` and a six-digit `abcd`),
  the Z rule above, a rotation of zero places, a rotation of nine places that
  comes back to where it started, the memory form of a rotate, the undefined
  flags of the decimal three, and one undo over a predecrement `abcd`, which
  writes two address registers and a byte of memory in one step.

#### Fixtures and documentation

- **No fixture moved.** No `editor/` program and none of the three `easy68k/`
  originals writes any of the eight Mnemonics, so the 30 assembly and run
  fixtures and the three `-errors.snap` are byte for byte what step 13 left
  them, at 15, 3 and 19 entries. `tests/corpus/README.md` records that, the
  eleven new printer lines and the two changed size lists.
- **The printer grew seven arms and eleven lines of `printer_rules`.** `addx`,
  `subx`, `negx` and the two rotates carry their size; `abcd`, `sbcd` and `nbcd`
  carry none; and the memory form of a rotate normalises to an explicit count of
  one and the word size, exactly as `asl (a0)` does.
- **One golden case changed**: `tests/diagnostics/unimplemented_operation.asm`
  traded its `roxl.w #1,d0`, which now assembles, for `reset`, which is refused
  for good and whose reason carries no alternative — so the case now covers a
  `hint` of `null` as well, which nothing else in it did.
- **`docs/grammar.md` is untouched again**, for the reason step 13 gives: the
  parser has read every one of these shapes since step 4, and the whole of what
  this step decided is the table's and the analyzer's.
- **`README.md`** gains the eight Mnemonics (a "Binary coded decimal" row of its
  own), a paragraph on the shapes and the Z rule, and loses them from its Todo,
  which is now the refused Directives, the PC-relative modes and real
  instruction sizes. `src/assembler/mod.rs`'s "What is still to come" says the
  same.
- **The `nop *` message phase 1 left to phase 3 was already delivered**, in
  phase 1's step 6 (`star_is_the_current_address`, a warning, with the golden
  case beside it); it was re-read against the phase 1 note in this step and
  nothing needed changing. The note's suggested "or put a space before `*`" is
  deliberately not in the hint: a space before the `*` is what `nop * do
  nothing` already has, and in this grammar a `*` where the Operand field begins
  is the current address whatever precedes it, so the only fix is `;`.

**What the addressing-mode step must know.**

* **The PC-relative modes are the only thing left of the design record's
  "Instructions"**, and they are a bigger change than this group was, because
  they reach the *encoded* Operand: `ast::Operand::PcDisplacement` and `PcIndex`
  have no `encoded::Operand` to become. That is a new variant (or two) in
  `encoded::Operand`, and therefore new arms in `get_operand_value`,
  `get_operand_address`, `store_operand_value` and the fixture printer, none of
  which has a catch-all. On the front-end side it is a `Modes` flag each with a
  name in `MODE_NAMES`, the arm in `Analyzer::check_operand_is_implemented`
  deleted, `Modes::of` and `lowering::lower_operand` answering them instead of
  `None`, and the doc comment on `Modes` that says they are deliberately absent
  rewritten.
* **What the PC is while an instruction runs** is the question that step has to
  answer first and that nothing in the crate answers today: instructions are a
  fixed four bytes (below), so `label(pc)` cannot mean what it means on a
  68000 unless the Assembler resolves it at assembly time. Resolving it in the
  Assembler — the displacement worked out from the instruction's own address —
  is the choice that keeps the fixed size honest, and it is the one the
  analyzer's current advice already implies ("write the label on its own").
* **Instruction size is still `INSTRUCTION_SIZE` = 4.** Untouched by both halves
  of phase 3. The handover of phase 2 stands word for word: real sizes have to
  be worked out in pass 1 from the Operands alone, in `plan_instruction`.
* **A new `Instruction` variant needs four arms**, and the compiler asks for
  three: `execute_instruction`, the fixture printer in `src/test/corpus.rs`,
  `lowering::lower` — and the printer rules of `tests/corpus/README.md`, which
  no compiler checks.
* **`Analyzer::choose_form` has two stages and `check` now has a pair check
  before the per-position loop.** A Mnemonic whose Forms are two whole shapes
  goes through `check_operand_pair` and is never judged position by position; a
  Mnemonic whose Forms differ in one position at a time is judged the old way.
  Which of the two a row is is answered by `the_advice_of_a_pair`, from the
  `Family`.

### Step 15 — the Addressing modes: PC-relative displacement and index, and the forced widths of an absolute address

The last part of phase 3, and the end of the design record's "Instructions".
`src/assembler/instructions/` grows two encoded Operands, two `Modes` flags and
the arithmetic that turns an address into a displacement; `src/interpreter.rs`
grows the one function that turns it back; `src/assembler/analyzer.rs` loses the
last of `check_operand_is_implemented` but `usp` and grows three checks.
`cargo test` is **428 green** (12 new since step 14's 416), `cargo fmt --check`
is clean, `cargo build --all-targets` raises no warning, `cargo clippy
--all-targets` no new one (the same 4, all older than phase 1's step 10) and
`RUSTDOCFLAGS=-D warnings cargo doc --no-deps` is clean. **No `editor/` fixture
moved**, and of the three `-errors.snap` only `clockDigital-errors` did, by two
message lines, which are the "yet" sweep below and not the modes.

#### What the PC is here, and the round trip that follows from it

- **A PC-relative Operand is written as an address and stored as a
  displacement.** EASy68K's own syntax is the address — "The displacement word
  (x) is specified as an address relative to the current PC. The assembler
  calculates the relative offset" (`Reference/68ks1e.htm`), whose example
  `MOVE.L $1102(PC),D0` reads `$1102` — so the source says where it wants to
  go and the Assembler works out how far that is. What it stores is
  `label - (address of this instruction + 2)`: the 68000 measures the
  displacement from the **extension word**, which sits one word past the
  operation word, and `EXTENSION_WORD_OFFSET` in
  `src/assembler/instructions/encoded.rs` is that two, read by the lowering,
  by the analyzer's range check and by the Interpreter alike, so that no
  arithmetic of the pair is written twice.
- **The Interpreter adds the same two numbers back**, in
  `Interpreter::pc_relative_address`, from `current_instruction_address` — the
  instruction being executed, not the program counter, which has already
  stepped past it. That is the whole of the round trip: the Assembler's
  `label - (A + 2)` and the Interpreter's `A + 2 + d` give `label` again, and
  the tests are written as the value read rather than as the arithmetic, so a
  pair that disagreed would fail on what the program computed.
- **Resolving it at assembly time is what keeps the fixed instruction size
  honest**, which is what the handover of step 14 said this step had to decide
  first. Instructions are four bytes here and hold no encoded words, so there
  is no extension word to read at run time and nothing but the stored
  displacement says where the operand was measured from. The alternative —
  storing the address and calling the mode PC-relative — would have made
  `label(pc)` a synonym for `label`, which teaches the opposite of what the
  mode is for.
- **`(pc,d1.w)` is the one PC-relative form whose number is not an address**,
  because it has no number at all: the displacement-free `(An,Xn)` is "a
  displacement of zero" (`docs/grammar.md` 2.5) and the same reading gives
  `(pc,d1.w)` the extension word's own address plus the index. Reading the
  absent displacement as "the address 0" would have made every use of it an
  out-of-range error. EASy68K's syntax list always writes the `x`, so this form
  is s68k's own leniency and this is the sentence that says what it means.

#### The modes in the instruction table

- **Two flags, and the group constants did the rest.** `Modes::PC_DISPLACEMENT`
  and `Modes::PC_INDEX` (`Modes::PC_RELATIVE` for the pair) went into `DATA`,
  `MEMORY` and `CONTROL` and stayed out of `ALTERABLE`, which is exactly where
  the manual's four groups put them, and **not one row of the table was
  edited**: `move`, `add`, `cmp`, `and`, `divu`, `chk` and `btst` take them
  because their positions are `ALL` or `DATA`, `lea`, `pea`, `jmp` and `jsr`
  because theirs is `CONTROL`, and every destination refuses them because
  `ALTERABLE` never held them. That the groups carry the whole answer is what
  `the_pc_relative_modes_are_read_and_never_written` in `table.rs` asserts,
  including the four control instructions by name.
- **Two constants had to be split**, and both are the same fact: a mode that is
  read and never written cannot be in a set the instruction writes.
  `MEMORY_ALTERABLE` (the destination of a memory shift) is `MEMORY` less the
  pair, and `MOVEM_TO_MEMORY` is the new `CONTROL_ALTERABLE` plus `-(An)`. So
  `movem.l (data,pc),d0-d2` reads registers back through a PC-relative operand
  and `movem.l d0-d2,data(pc)` is refused — which is the one place `movem`'s two
  directions differ by more than the side the list is on.
- **`MODE_NAMES` grew `d(PC)` and `d(PC,Xn)`**, in the manual's own order
  (after `Ea/<label>`, before `Im`), so every "there it takes …" hint now offers
  them where they are allowed. That moved two hints in
  `tests/diagnostics/snapshots/invalid_addressing_mode.snap` — `divu`'s, whose
  position is `DATA`, and `jmp`'s, whose position is `CONTROL`, while `clr`'s
  data-alterable one is unchanged — and one in `analyzer.rs`'s own tests. The
  change is the point: a mode that is implemented belongs in the list of what
  would have been right.
- **`Modes::of` answers `None` for `usp` alone now**, and
  `Analyzer::check_operand_is_implemented` is one `let … else` about it. With
  the PC-relative modes implemented, the `planned` flag step 13 added to
  `unimplemented_addressing_mode` had no `true` left to carry, so **it is
  deleted**: a field whose one branch nothing can raise is a promise the code
  cannot keep. The code and the message are otherwise unchanged, and the kind
  is now about one thing, which its doc comment says.

#### The three checks the analyzer grew

- **How far a PC-relative Operand has to reach is `value_out_of_range`**, not a
  kind of its own — the same finding as the displacement of `d(An)`, which that
  kind has answered since phase 1, with the subject changed: "the distance a
  `d(PC)` operand reaches is -32768 to 32767, and `192502` is outside it",
  hinted "write `far` on its own: an absolute address reaches anywhere in
  memory". The range is EASy68K's ("Word displacements must be in the range
  -32768 through 32767. Byte displacements must be in the range -128 through
  127", `errors.htm`), and the *value* in the message is the distance and not
  the address, because the distance is what does not fit and it is a number the
  student never wrote. `Analyzer::check_pc_distance` calls
  `lowering::pc_relative_offset`, the same function the lowering stores, so the
  check and the store cannot disagree about what is in range.
- **An address forced to `.w` is checked and one forced to `.l` is not.** The
  two name the same address here — s68k stores addresses and encodes no words —
  so `.l` says nothing at all, while `.w` is a claim about the address that can
  be false: EASy68K's own "Absolute short addressing must be in the range
  -32768 through 32767" (`errors.htm`), as `value_out_of_range` with the subject
  "an address forced to `.w`" and the advice "write `$18000.l`, or `$18000` on
  its own: both reach the same address here".
- **That is a deviation, and ADR 0001 now lists it.** EASy68K *warns* that
  forcing short "disables range checking of extension word" and encodes the low
  word sign extended, so its `$8000.w` reads `$ff8000`. s68k reads the address
  as written, so an address the field cannot name would quietly mean a different
  place here; refusing it is the stricter direction and the ADR is where a
  stricter direction is recorded. The sign extension itself is deliberately not
  reproduced: `.w` and `.l` naming one address is what the rest of this step is
  built on.
- **One new `DiagnosticKind`, `invalid_address_width`**, for the suffix that is
  neither: `move.l table.b,d0` forces the *address* to a byte. It is not
  `invalid_size` — that kind's sentence is "`.b` is not a size for `move`", and
  the mistake here is that the suffix is not the instruction's size at all — so
  the message says what the suffix does where it stands ("`.b` after `table`
  forces the width of the address, and an address is forced to `.w` or `.l`")
  and the hint says where the instruction's own size goes. It catches
  `bra done.s`, which is `bra.s done` written on the wrong field, and it is the
  one kind this step adds; its golden case is
  `tests/diagnostics/invalid_address_width.asm`, which holds both mistakes and
  the three lines they were meant to be.
- **"A PC-relative operand is read and never written" is the new suggestion**
  on `invalid_addressing_mode`, said only where the position reaches memory at
  all — so `move.l d0,data(pc)` gets it and `lea (a0),data(pc)`, whose second
  operand takes `An` and nothing else, is told what it takes instead. It is
  ADR 0003's "what was probably meant" for the one mistake this mode has that a
  list of allowed modes does not explain.

#### Lowering, running and printing

- **`Values` gained `instruction_address`.** The lowering needed one fact the
  tree does not hold, and the trait that already answers "the value of this
  Expression" is where it goes; `analyzer::Context` answers it with
  `current_address`, which the Layout has been filling in since phase 1. No
  call site of `lower_operand` changed.
- **`encoded::Operand` gained two variants and no `Instruction` did.** A
  PC-relative operand is an Addressing mode and not an encoding of its own, so
  `get_operand_value`, `get_operand_address` and the fixture printer grew an arm
  each and `execute_instruction`, `lower` and the printer's instruction match
  grew none — which is why `lea`, `pea`, `jmp` and `jsr` needed no work beyond
  `get_operand_address`.
- **`store_operand_value` answers `IncorrectAddressingMode`** for the pair,
  which is unreachable from any assembled Program (nothing writes through the
  program counter, and `Modes::ALTERABLE` is where that is enforced). Writing to
  the address it names would have been the wrong kind of lenient: the arm exists
  because the match has no catch-all, and it says what it is.
- **The printer writes the displacement, not the address.** `move.l data(pc),d0`
  at `$1000` with `data` at `$1014` prints as `move.l 18(pc),d0`, which is what
  the Program holds; `tests/corpus/README.md` says so and `printer_rules`
  writes seven new lines, a negative displacement and the two forced widths
  among them. A forced width prints as nothing at all, because the Program keeps
  the address and not the suffix.

#### The sweep, and what is left

- **Every remaining "not implemented" reason was read against the plan.** The
  instruction table's three are `rte`, `stop` and `reset`, each about the
  machine s68k simulates and none of them saying "yet"; `usp` is the same and
  says it in the analyzer. `layout::unimplemented_reason` keeps "yet" for
  `include` and `incbin`, which phase 4 really adds, and **loses it for macros
  and conditional assembly**, which no phase of this plan adds — the design
  record has them as "maybe a later milestone" — so "macros are not assembled
  yet" is now "macros are not assembled" and "conditional assembly is not
  implemented yet" is "conditional assembly is not implemented". `memory` and
  the structured-control keywords never had one. That moved two messages in
  `clockDigital-errors` — the whole of what this step moved in the corpus — and
  the same sentences in the golden cases of `unimplemented_operation`,
  `unterminated_macro_definition` and `label_not_allowed`.
- **Two records were corrected in place, and both say so where they stand.**
  `tests/corpus/README.md`'s list of "every real 68000 instruction s68k does not
  implement" still named the eight Mnemonics step 14 implemented; it is `rte`,
  `stop` and `reset` now, with a parenthesis saying what changed and when. And
  `docs/adr/0001` gained one bullet, the range check on an address forced to
  `.w`, because that ADR is where a place s68k is stricter than EASy68K has to
  be listed and this step made one.
- **Branch sizes were confirmed and left**, as this step was told: `.b`, `.s`,
  `.w` and `.l` are `SizeRule::Branch`, accepted and not range checked because
  every instruction is four bytes, and none of them reaches the encoded
  instruction (`branch_takes_b_as_it_takes_s`, phase 1's step 10).
- **`docs/grammar.md` is untouched for the third step running**, and for the
  same reason: the parser has read `label(pc)`, `(d,pc,xn)` and `label.w` since
  step 4, and everything this step decided is the table's, the analyzer's and
  the lowering's. That is ADR 0003 doing its job.
- **The `ts-lib` chain was run and is green** — `wasm-pack build`,
  `npm run build-lib`, `npm test` — although nothing crossing the boundary
  changed shape: `src/lib.rs`, `src/ts_types.rs` and `ts-lib/` are untouched, an
  `Instruction` reaches JavaScript as `instruction: any`, a Diagnostic's `code`
  is declared as `string` rather than as a union of the codes, and `parseLine`
  has answered `pc_displacement` and `pc_index` as mode names since phase 1. It
  was run because this is the last step of phase 3 and the chain is what CI
  runs.
- **Not done, and deliberately: a width forced on a Directive's operand says
  nothing.** `org $2000.w` and `dc.l big.w` are read for their value and their
  suffix is ignored, because the check above is about an Addressing mode — how
  an instruction *reaches* a place — and a Directive's operand is a value, not
  an address. Nothing in the corpus writes one, and the shape that a student
  really does write, `move.l #big.w,d0`, has been answered since phase 1 by the
  parser: "this looks like an immediate operand, `#5`, but an immediate carries
  no size: the size goes on the operation".
- **Twelve new tests**: two in `table.rs` and `lowering.rs` (the groups the
  modes belong to, and the distance the lowering stores in both directions and
  for the displacement-free form), three in `analyzer.rs` (where the modes are
  allowed and what the refusals say, the distance against both fields, and the
  forced widths), and seven in `src/test/test.rs` under `pc_relative_addressing`
  (a read forwards, a read backwards, two lines at two addresses reading one
  place, the index form and the displacement-free one, a `movem` reading three
  registers back, `jmp`/`jsr`, and a `pea` that pushes the address).

**What phase 4 needs to know.**

* **Phase 3 is finished.** Every Mnemonic of the design record's "Instructions"
  is implemented and every Addressing mode CONTEXT.md lists is assembled. What
  is refused for good is `rte`, `stop`, `reset`, `move usp,an`, `memory`,
  macros with conditional assembly and structured control; what is refused with
  a "yet" is `include` and `incbin`, which are phase 4's, and they are the only
  two sentences left in the crate that promise anything.
* **`unreadable_file` is still the one code with no golden case**, and phase 4's
  `include` owes it (`WITHOUT_A_CASE` in `src/test/diagnostics.rs`).
* **Instruction size is still `INSTRUCTION_SIZE` = 4**, and a PC-relative
  operand is now the second thing that depends on it: the round trip is
  `label - (A + 2)` and `A + 2 + d`, where the `2` is `EXTENSION_WORD_OFFSET`
  and not a function of the instruction's size, so real sizes change nothing
  here — but a version that assembled real encodings would have to keep the
  displacement measured from the extension word, which is what the constant's
  doc comment says.
* **The two places that read a line index as a position are unchanged**
  (`Symbol::value_at` for `set`, and the register-list forward-reference check),
  so phase 2's handover to phase 4 stands word for word.

## Implementation notes (phase 4)

The running record of phase 4, kept the way phases 1 to 3 are: one bullet a
choice, so that the next step can read the state of the work from the
repository. Nothing above this heading is rewritten except to fix a factual
error, and such a fix says so here — this phase made two, both listed under
"The twelve findings" below. **The step numbers carry on**, so that a reference
to "step 15" means one thing in this document.

### Step 16 — `include`, `incbin`, and the assembled sequence

The last phase of the plan. `src/assembler/` gains `include.rs`, which turns a
Project into the one sequence of lines the Layout walks; `layout.rs` is indexed
by **position** in that sequence rather than by Source line, and grows
`plan_include` and `plan_incbin`; `program.rs` carries the Include chain on
every assembled instruction; `src/main.rs` reads a Project from disk;
`src/test/include.rs` is new. `cargo test` is **472 green** (44 new since step
15's 428: 16 in `include.rs`, 27 in `src/test/include.rs`, and the split-file
fixture test in `src/test/corpus.rs`), `cargo fmt --check` is clean, `cargo
build --all-targets` raises no warning, `cargo clippy --all-targets` no new one
(the same 4, all older than phase 1's step 10) and `RUSTDOCFLAGS=-D warnings
cargo doc --no-deps` is clean. The `ts-lib` chain was run — `wasm-pack build`,
`npm run build-lib`, `npm test` — because `src/ts_types.rs` and the smoke test
both changed. **No corpus fixture moved**, in either direction, and
`tests/corpus/README.md` says so with the reason.

#### The assembled sequence, and the position that replaces a line index

- **The Assembler does not assemble a File; it assembles the assembled
  sequence.** `include::expand(files, entry)` reads the Entry file, walks its
  parsed lines, and after each `include` line pushes the lines of the File it
  names, recursively; the result is a `Vec<Position>`, one entry per line the
  Assembler will lay out, each carrying `{ file, line, chain }`. `lay_out` takes
  that `Expansion` where it used to take a `SourceFile` and a `ParsedFile`, and
  every `index` in `layout.rs` is now an index into it. That is the whole shape
  of the change: nothing in the Layout knows that a File was included, only that
  line *n* of the program is line *l* of File *f*.
- **A position is what "above" and "below" mean, and it is the two comparisons
  the earlier phases named.** Phase 1 (step 7) wrote that `Symbol::value_at` was
  "the Source line index today; phase 4 makes `include` textual and has to make
  it a position in the assembled order instead", and phase 2 (step 11) added the
  register-list forward-reference check as the second. Both are changed here and
  neither needed a new mechanism: `SymbolTable::define` already took an `at`,
  and the Layout now passes the position; `Symbol` gained `defined_at`, the
  position of its first definition, which is what `resolve_register_lists`
  compares instead of `symbol.location.line`. A File included twice therefore
  has two `set` values and two `reg` definitions in the right order, which
  `a_set_variable_sees_the_latest_definition_above_it_in_the_assembled_sequence`
  and `a_register_list_has_to_be_defined_above_the_movem_in_the_assembled_sequence`
  hold.
- **A line index and a position had to be told apart everywhere a Location is
  built**, and the one place that got it wrong was caught by a test: the
  analyzer takes `(file, line_index, line_text)` and was handed the position,
  so every Diagnostic of an included File named a line of the entry file's
  numbering. `Expansion::location`, `Expansion::whole_line` and
  `Expansion::line_index` are the three accessors, and `Layout::raise` goes
  through the first, so no phase builds a Location out of an index by hand any
  more.
- **CONTEXT.md gained two terms**, because the glossary's "Location" entry
  already said to *avoid* the word "position" for it and phase 4 needed the word
  for something else: **Assembled sequence** ("the Entry file's lines with every
  `include` expanded into them") and **Position** ("an index into the Assembled
  sequence"). The "Location" entry now says which is which. That is a change to
  the domain model and it is recorded here rather than made silently.
- **A File is parsed once however many times it is included**, memoised by path,
  and its parser Diagnostics are reported once, at the positions of the first
  inclusion. A mistake in the *text* of a File is one mistake however often the
  File is pasted in; the two once-a-File suggestions (`bare_comment`,
  `double_quoted_string`) are defined that way in `docs/grammar.md` 1.9 and 1.6;
  and `tests/corpus/editor/bad-apple.x68` is 3.3 MB, which is a reason of its
  own not to parse a File twice.
- **Everything else about a File *is* assembled twice**, which is what textual
  means: two copies of its lines, two sets of addresses, two definitions of
  every name it defines. `the_same_file_may_be_included_twice` holds the memory
  and `a_file_included_twice_stops_at_both_copies` holds the consequence for
  breakpoints.
- **Diagnostics are sorted by position and then by column.** They were sorted by
  `(line, column)`, which in a Project would interleave two Files at random;
  they are collected as `(position, Diagnostic)` and sorted on that, so an
  included File's messages sit between the two halves of the File that includes
  it, which is the order a student reads. For a Project of one File a position
  *is* the line index, so no single-File ordering moved — which is what the 63
  unchanged `tests/corpus/` snapshots and 62 of the 63 `tests/diagnostics/` ones
  say, the one that moved being finding 8's.
- **The `include` line stays in the sequence.** It is not deleted and replaced;
  it is a line that produces nothing, followed by the included lines. That is
  what makes `label include file` work with no rule of its own: the Label is
  defined at the current address, exactly as a Label on a line of its own is,
  and the current address is where the first included byte goes. It also means
  the line still gets its size check (`include.b` is `invalid_size`) and its
  label rule.
- **An `include` after `end` is still read.** The expansion is textual and knows
  nothing about `end`, so the File is read and a mistake in it is reported,
  while the lines it brings in are held back by the same rule that holds back
  every line after `end` and the first of them raises the one `code_after_end`
  warning. The alternative — teaching the expansion about `end` — would have put
  a Directive's meaning in the phase that has no Symbols, no addresses and no
  sections. `an_include_after_end_is_still_read_and_its_lines_are_not_assembled`.

#### Resolving a file name

- **Beside the including File first, then the project root**, which is the
  design record's own order, and `\` is a separator like `/`
  (`Directives/include.htm`'s example is `"C:\EASy68K\macros\input output
  macros.x68"`). `include::join` resolves `.` and `..` on the way and drops a
  `..` that would climb above the root, because a Project has no above-the-root
  (CONTEXT.md, "File"). `Files::normalise_path`, which phase 1 wrote and which
  deliberately kept `..` "for phase 4's resolution to deal with", is unchanged
  and still settles how one File is spelled twice; this is the step that deals
  with `..`.
- **The quotes are not part of the name**, either kind will do, and a doubled
  quote inside a quoted name is one quote — which is the rule the parser's
  `file_specification_extent` already reads the field by, so the two agree.
  `written_path` is the whole of it, and an empty name is treated as no name at
  all.
- **`Files::entry` is new and answers the path as the Project spells it.** Every
  Location of an included File carries that path, and a breakpoint has to match
  it, so the spelling that reaches the editor is the Project's own key and never
  the one the `include` line happened to write.
- **A miss lists the closest existing paths, and the first rule is the name.**
  `include io.m68k` against a Project holding `lib/io.m68k` is the case the
  design record names, and no edit distance over whole paths would find it: the
  first rule is "a File whose *name* is the name that was written", the written
  name with no extension included, and only when that finds nothing does the
  ordinary did-you-mean over the whole path run (`table::edit_distance`, made
  public for it). At most three are offered.
- **A Project with no other File says so.** "did you mean" has nothing to offer
  when there is nothing to have meant, and "there is no file named `io.m68k` in
  this project" with no hint at all would leave a student who has one buffer
  open wondering what they typed wrong. The hint is "this project has no other
  file to read".
- **The command line reads a Project from disk.** `src/main.rs` used to file one
  File and assemble it, which would have made every `include` in a program run
  from the command line a missing file. It now reads the Entry file, scans its
  lines with the Assembler's own `parse_line` for `include` and `incbin` names,
  resolves each with `include::join` — the same function the Assembler resolves
  with, so the two cannot disagree — and reads those Files too, transitively. It
  reads **only the Files the program names**: walking the directory would read
  `target/` and everything else that happens to sit beside the program. A File
  that is not valid UTF-8 goes in as bytes, which is what `incbin` wants. The
  map back to the paths on disk is what every message prints, since a Project
  path has no leading `/` and an absolute command-line path is not one.
  *(**Superseded by step 17**, which reads the Entry file's whole directory
  instead: a scan of the `include` lines can follow only the first of the two
  places a name resolves to, and it can never offer the File that was meant.
  This bullet stays as the record of what step 16 built; step 17's notes have
  the reason and the rules that keep `target/` out.)*

#### The Include chain

- **A chain is a linked list of links, not a list per line.** One link per
  *followed* `include`, `{ site, parent }`, and a position carries the index of
  its innermost link; `Expansion::chain` walks the parents back. A File of six
  thousand lines included twice costs two links.
- **The `site` is the file name, not the whole line**, so an editor underlines
  the thing that pulled the File in.
- **Every Diagnostic gets its chain in one place.** `Layout::finish` walks the
  collected `(position, Diagnostic)` pairs and appends the chain of each as
  related Locations, innermost first, with the message "included from
  `main.m68k`". No phase has to remember to do it, and the expansion's own
  Diagnostics — a missing File named by an included File — get it too.
- **The related message names the File the `include` line is in, and the
  Location says where.** Naming the line number in the message as well was the
  alternative; the message would then have had to choose between the 0-based
  number every Location in this crate carries and the 1-based number a person
  reads, which is a choice `tests/corpus/README.md` spent a paragraph on once
  already.
- **An assembled instruction carries its chain**, `AssembledInstruction::include_chain`,
  because a Location cannot answer "through which `include` line did this
  instruction get here" once a File may be included twice: the two copies share
  one Location and differ only in this. It is the design record's own
  requirement for phase 4, and `the_two_copies_of_a_file_differ_only_in_their_chain`
  is the test that shows why a Location alone would not do. A `MemoryRun`
  deliberately carries none: the editor shows memory by address, and no view of
  it asks where the bytes were written.
- **`AssembledInstruction` is serialised camelCase now**, `#[serde(rename_all =
  "camelCase")]`, which changes nothing but the new field's name
  (`includeChain`): every other field is one word. The rule at the top of
  `src/ts_types.rs` is that the Assembler's shapes are camelCase, and this is the
  first field of one that has two words in it.
- **A File included twice is named in the duplicate-symbol error.** When
  `symbol_already_defined` is raised and the previous definition is at another
  position of the *same* File, `Expansion::included_twice` answers the two
  `include` lines and the Diagnostic gets them as related Locations: "`lib.m68k`
  is included here" and "and included again here, so every name in `lib.m68k` is
  defined twice". It answers the **outermost** pair of chains that differ, not
  the innermost, because a File included once by a File that is itself included
  twice is the outer line's doing and the inner one would name the same line
  twice. Two definitions in one copy of one File say nothing about `include`.

#### `incbin`

- **`incbin` is a `dc.b` of the whole File**, and the test says so as a
  comparison: `incbin_is_a_dc_b_of_the_whole_file` assembles the same bytes
  written out by hand and asserts the same memory and the same Label. No
  alignment, the Label on the first byte, the bytes at the current address.
- **Pass 1 takes the room and pass 2 reads the bytes**, which is how `dc`
  already splits its length from its values; the File is resolved twice and the
  second time silently, since pass 1 has already said whatever there was to say.
  Storing the bytes in the `LinePlan` was the alternative and would have put a
  megabyte in a structure that is cloned once a line.
- **A text File contributes its Latin-1 bytes** (ADR 0004), through the new
  `source::latin1_bytes`. A character above 255 has no byte at all and is
  reported **where it is** — in the File that holds it, at its own line and
  column, through the new `SourceFile::location_of` — with the `incbin` line as
  a related Location. One message per `incbin`, however many such characters the
  File holds, and a `0` is written for each so that every byte after it keeps
  the address it will have once the character is fixed.
- **`incbin` inside an `offset` region is `no_bytes_in_an_offset_region`**, like
  a `dc`, and needed no code: the region rule is applied to the plan and an
  `incbin` plans `Item::Data`.
- **EASy68K's `incbin` "inserts the specified binary file into the S-Record
  output file"; s68k has no S-Record and puts the bytes in memory**, which is
  the same thing for a program that then reads them. Written down because the
  help's sentence is about a file format this assembler does not have.

#### The Diagnostics

- **`unreadable_file` grew to cover the whole family**, as phase 1 said it would
  ("the Entry file is missing, or holds bytes; phase 4's `include` reuses it").
  It carries `directive` (`include`, `incbin`, or `None` for the Entry file),
  `suggestions` (a list now, where it was one), `binary` and `alone`, and it has
  four sentences: a missing File, a missing File with somewhere to look, a File
  that holds bytes where `include` wanted source, and the Entry file itself
  holding bytes. One code, because the finding is one — a File that was asked
  for is not the File that was needed — and because a student's fix is in the
  message rather than in the code an editor matches on.
- **Three new kinds.** `include_cycle` (the chain written out, `main.m68k ->
  a.m68k -> main.m68k`), `include_too_deep` (the two backstops, one kind with a
  sentence each, the way `unreadable_file` has always had two) and
  `end_in_an_included_file` ("`end` belongs in the entry file, `main.m68k`"),
  which ADR 0001 has listed since phase 0 as the one place `include` is stricter
  than EASy68K. All three are errors.
- **`end` in an included File is refused and the line is then ignored.** EASy68K
  stops assembling there and drops the rest of the entry file; stopping here
  would answer one mistake with a file of consequences, so the lines below it
  are laid out where they would have been and the Entry point still comes from
  the Entry file. The Program is not built either way, since the line is an
  error.
- **Two backstops, and ADR 0001 now lists them**: eight Files of nesting, and
  200,000 lines in one assembly. A cycle is caught exactly, by path, so nesting
  is already bounded by the number of Files a Project holds and neither limit
  can be reached by a program anybody meant to write; they are there because
  this assembler runs in a browser on every keystroke, and a File that includes
  two others which each include two more *multiplies*. Eight is deep enough that
  a chain that reaches it is one nobody can read, and the message says to
  include the files side by side instead. The line budget is thirty times the
  longest program the editor ships. The budget is reported once however many
  `include` lines are left.
- **An `include` with no file name at all is `wrong_operand_count`**, not a kind
  of its own: `include` and `incbin` take exactly one file name, the count is the
  only thing wrong with the line, and the expansion says nothing about a line
  that names no File.
- **`unimplemented_reason` in `layout.rs` is down to `memory`, the macro
  Directives, conditional assembly and structured control**, and **nothing in
  the crate says "yet" any more**: `include` and `incbin` were the last two
  sentences that promised anything, which is what step 15 said they were.

#### The tests

- **`src/test/include.rs` is the rule-by-rule module**, 27 tests: the lines
  assembled where the `include` line is, the section and the address, the one
  Symbol namespace both ways, the Local label scopes across the boundary in both
  directions, `set` and `reg` by position, the Label on the `include` line, a
  File included twice and the duplicate-name error that names the two lines,
  `end`, an `include` after `end`, the chain on a Diagnostic and on an
  instruction, a breakpoint in an included File and one that stops at both
  copies, and nine on `incbin`. The rules about the *shape* of the sequence —
  resolution, cycles, the backstops, the chain itself — are tested beside the
  code that builds them, in `include.rs`'s own 16.
- **`editor_programs_split_across_files_assemble_the_same` is the corpus-scale
  version**, and it is a fixture test with no fixture file: it cuts three
  `editor/` programs into an Entry file and one or two included Files — the two
  strings of `hello-world-1`, the whole of `subroutine-with-register-arguments-1`'s
  `gcd`, and `max-of-an-array-1`'s constant and data — and asserts that the
  fixture of the split program is identical, serialised, to the fixture of the
  whole one, which `editor_programs` pins against the snapshot on disk. **The
  cut keeps every line number**: the lines that move out are replaced by one
  `include` line and then blank lines, and the File they move into is padded
  with as many blank lines as there are lines above them, so the `n`th line of
  the program is still the `n`th line of whichever File now holds it. That is
  what lets the comparison cover the `line` of every instruction and Label and
  not only the addresses and the bytes. The fixture format names no File, which
  is what makes the comparison possible at all, and `tests/corpus/README.md`
  says so.
- **A `tests/diagnostics` case may now be a directory**, which is a Project:
  every file under it is a File named by its path inside the directory,
  `main.asm` is the Entry file, and a `.bin` file is a binary one. No single
  File of source can name another File to fail to read, which is why
  `unreadable_file` had no case for three phases; it has one now, and so do the
  three new kinds. **`WITHOUT_A_CASE` in `src/test/diagnostics.rs` is empty**:
  every `DiagnosticKind` the Assembler can raise has a program that raises it
  and a snapshot to read.
- **The `unreadable_file` case holds three of its four sentences** — a missing
  File with a suggestion, a missing File with nothing to suggest, and an
  `include` of a binary File — and says in its own comment that the fourth, the
  Entry file itself, is `an_entry_file_that_is_missing_or_binary_is_the_one_failure`
  in `include.rs`, because a case is a Project whose Entry file is read by
  definition.
- **The smoke test grows the multi-file case the design record's "Tests" item 6
  asks for**: a Project of three Files across two directories, `include` and
  `incbin` and a binary `Uint8Array`, run to the end, the `includeChain` of an
  instruction of the included File read back, a mistake in an included File with
  its chain, and a missing File with its suggestion.

#### The twelve findings of the phase 3 review

Ten of the twelve are answered here; the two that are not are analyzer
semantics, and both say why.

1. **Not done: `move sr,sr` and `movep 0(a0),0(a1)` still get a per-position
   message that is false of the instruction.** The fix is the right one — a
   whole-shape check, as `addx` has — but it is not cheap: `invalid_operand_pair`
   hard-codes "two data registers or two predecrement operands" in its message,
   so it needs a `shapes` field filled from the `Family`, `check_operand_pair`
   needs a per-Family guard (`move` only when *both* Operands name a half of the
   register, or `move a0,sr` would stop being told which half is wrong), and
   `movep`'s existing good message for `movep.w d0,(a1)` moves too. It is a
   change to the analyzer's shape in a phase about Files, and it is carried:
   **the next person to open `analyzer.rs` should do it**, and this bullet is
   the design.
2. **Not done: `chk d0,#5` is still told that "nothing can be written to" an
   immediate.** The suggestion is right wherever the position is written and
   wrong wherever it is read, and the table has no "this position is written"
   flag to gate it on — `chk <ea>,Dn` reads its register and `tst <ea>` reads its
   operand, so it is not one Mnemonic's special case. Adding the flag is a
   `Form`-wide change to a 125-row table, which is not this phase's. Carried
   with the same note as 1.
3. **Done: ADR 0001's odd-`org` consequence is corrected in place** and marked as
   a correction — the rule is "an odd `org` that *moves* the address warns and
   rounds up", which step 12 established and which `docs/grammar.md` already had.
4. **Done: the "an `org` that moves nothing says nothing" rule is written down**
   in `docs/grammar.md` 2.6, with the two readings of `ORG $1001` side by side,
   and in `tests/corpus/README.md`. The rule is kept as it is rather than
   restricted to `org *`: it is about the address and not about the text, and an
   `org` that moves nothing has nothing to round up whatever it is written as.
5. **Done: the citation is `Reference/68ks6d.htm`**, the `EORI` page, which is
   the one carrying "can only work with word and byte"; `68ks6b.htm` stays for
   the address-method list.
6. **Done: the design record's "one typing slip in the help" is three**, and the
   bullet in step 14 now names all three rows (`68ks5q.htm`'s `NEGX`,
   `68ks5e.htm`'s `ADDX` table against its own prose, `68ks8f.htm`'s inverted
   `NBCD` sentence) and says which reading is implemented and why. It is marked
   as the correction it is.
7. **Done: "takes none or one operands" is gone.** `operand_count` has an arm
   for a list that starts with 0 — "no operand or one of them" — which `end` and
   `section` both reach. No snapshot held the old sentence.
8. **Done: `section` has its own message and its own hint.** "`section` with no
   number sets a name to the number of the section in force, and this line has
   no name", hinted "write the name in the first column, `here section`, or
   write the number, `section 1`" — where the generic hint offered `count
   section …`, an operand that would have removed the requirement. The golden
   case `directive_needs_a_label.asm` gained the bare `section` line, and its
   snapshot moved for it.
9. **Done, the honest half: the records say what the code does.** `docs/grammar.md`
   2.6 and step 11's bullet above now read "the Layout ignores `simhalt`'s
   Operand field" and say that the field is still tokenized, so a mistake inside
   it is still reported; the test is `simhalt_ignores_the_rest_of_its_line`. The
   other half of the finding — making it true by giving `simhalt` a
   `RawOperandField` — is a **grammar change** (ADR 0002: the document first) and
   would move what the parser accepts on `rts`, `nop`, `page` and `nolist` too if
   it were done consistently. It is not done, and it is the better student
   experience: whoever takes it should read this bullet and 2.5 together.
10. **Done: `invalid_address_width.asm`'s comment counts its own lines.**
11. **Done: `sr` and `ccr` are out of the corpus README's Operand table**, which
    mirrors `encoded::Operand` one row per variant, and into a paragraph under
    it saying they are part of the instruction and not operands of it.
12. **Done: the corpus README says what the `+2` of a PC-relative displacement
    is.** It is the hardware's number for every form but `movem`, whose mask word
    is the first extension word; s68k encodes no words, so the single constant is
    a deliberate simplification and a printed `movem` displacement is two less
    than a real assembler's.

**What the surface step must know.**

* **The public surface changed in exactly one place**: `InstructionLine` gained
  `includeChain: Location[]`, empty for an instruction of the Entry file. The
  input side did not change at all — `assemble(source, options?)` has taken
  `{ files, entry }` since step 9, and a binary File has always crossed as a
  `Uint8Array` — and neither did `Diagnostic`, whose `related` is where the
  chain arrives. `Breakpoint` is still `{ file, line }` and now stops in an
  included File; a File included twice has one line and two addresses, and
  `get_breakpoint_addresses` answers both.
* **What the editor can now ask.** Where an instruction came from (`location`),
  through which `include` lines (`includeChain`, innermost first), where a
  Diagnostic is and what it was reached through (`location` and `related`), and
  which Files a Project is made of, which it already knew because it wrote them.
  What it cannot ask, and what a later step would have to add, is the reverse
  direction — "which Files did this build read?" — since a Program carries no
  list of them; the chains on the instructions are the only trace.
* **Four codes are new or changed shape**: `unreadable_file` (now raised by
  `include` and `incbin` as well as by the Entry file), `include_cycle`,
  `include_too_deep` and `end_in_an_included_file`. An editor that lists codes
  has four more to know about; one that matches on `code` and shows `message`
  and `hint` needs nothing.
* **Nothing in the crate promises a feature any more.** The `unimplemented_*`
  sentences are `rte`, `stop`, `reset`, `move usp,an`, `memory`, macros,
  conditional assembly and structured control, and none of them says "yet".
  Real instruction sizes are the one decision the design record still leaves
  open, and `INSTRUCTION_SIZE` is still 4.
* **Two findings of the phase 3 review are carried** (1 and 2 above), both in
  `analyzer.rs`, both about a message that says something untrue of the
  instruction. Neither is about Files, and both have their fix written out.

### Step 17 — the 2.0 surface for a Project: the declarations, the wrapper, the command line

The last step of phase 4 and of the plan. It adds no field and no diagnostic:
step 16 built the Project and this step makes the three places that face
outwards say so — the WebAssembly declarations (`src/lib.rs`,
`src/ts_types.rs`), the TypeScript wrapper and its smoke test
(`ts-lib/src/index.ts`, `ts-lib/test/smoke.mjs`), and the command line
(`src/main.rs`) — and finishes the documents. `cargo test` is **476 green**
(4 new, all in `src/main.rs`, which had none: 472 in the library and 4 in the
binary), `cargo fmt --check` is clean, `cargo build --all-targets` raises no
warning, `cargo clippy --all-targets` no new one (the same 4, all older than
phase 1's step 10) and `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` is clean.
The chain was run from a rebuilt `pkg`: `wasm-pack build --out-dir
ts-lib/src/pkg --out-name s68k`, then `npm run build-lib` and `npm test` in
`ts-lib`. No `tests/corpus/` fixture and no `tests/diagnostics/` snapshot moved,
in either direction: nothing here changes what is assembled.

#### The boundary

- **The input side did not change and now says so.** `wasm_assemble`'s first
  argument was declared `any`, which was the last shape crossing the boundary
  that the generated `.d.ts` did not describe. It is now a `SourceFiles`: a
  `#[wasm_bindgen(typescript_type = "SourceFiles")]` extern type in
  `src/lib.rs` and the declaration itself — `Record<string, string |
  Uint8Array>` — beside the other custom sections in `src/ts_types.rs`, so
  `wasm_assemble(files: SourceFiles, entry: string)` is what the package ships.
  It is a name for what step 9 already accepted and not a change to it; the
  wrapper keeps its own `SourceFiles` alias, structurally the same type, so
  nothing imports across.
- **A `Uint8Array` is the only way `FileContent::Bytes` is built, and the smoke
  test is what proves it.** `files_from_js` reads a `string` into
  `FileContent::Text` and anything that `dyn_ref`s to a `Uint8Array` — a Node
  `Buffer` is one — into `FileContent::Bytes`, and it cannot be exercised by
  `cargo test`, because a native build has no JavaScript values to hand it. The
  `incbin` case of the smoke test assembles a `Uint8Array` and has the *program*
  read one of its bytes back, which is the whole path in one assertion.
- **Everything else this step was asked to check was already true.**
  `Diagnostic.related` crosses as `[{ location, message }]` and not as a tuple
  (`SerializedRelatedList` in `diagnostics.rs` is why, since phase 1), the
  Include chain crosses as `includeChain: Location[]` (step 16), and a Location
  is camelCase wherever it appears. The only thing that was wrong was a comment:
  see the corrections below.

#### The wrapper

- **One sentence deleted, and no type added.** `S68k.assemble`'s documentation
  still said that "`include` and `incbin` are not implemented yet, so a project
  of more than one file assembles only its entry file" — the last "yet" in the
  repository, and step 16's claim that there is none was true only of the crate.
  It is now the paragraph a caller needs: a project is assembled from its entry
  file down, a missing File is a diagnostic naming the closest one, and nothing
  reads a disk. `SourceFiles`, `AssemblySource`, `AssembleOptions`,
  `AssemblyResult`, `Location`, `RelatedLocation`, `Diagnostic` and
  `InstructionLine` were already the wrapper's exported types, which is what
  "add nothing the design record does not name" left to do.

#### The smoke test

- **Four cases, and they are the four questions an editor asks of a Project.** A
  Project of four Files — an entry file, a data File and a subroutine File it
  `include`s, and a `Uint8Array` it `incbin`s — assembled, run to the end, with
  `D2` holding the sum the included routine computed over the included data and
  `D3` a byte the program read out of the binary File; the `includeChain` of an
  instruction of the subroutine File, and the empty chain of one of the entry
  file; a breakpoint inside an included File; a mistake in an included File,
  whose Location names that File and whose `related` names the `include` line
  with "included from `main.x68`"; and a missing File, with its code, its
  Location, its columns and "did you mean `lib/io.x68`?" in the hint. The
  symbols of both included Files are read back through `getInfo`, one of them a
  Local label, which is where a wrong comment was caught.

#### The command line reads a directory as a Project

- **The Project is the directory the Entry file is in**, read as far down as it
  goes: every File under it is one File of the Project, named by its path inside
  it, with the Entry file's own path relative to it. `cargo run --
  dir/main.asm` assembles `dir/main.asm` as `main.asm` and resolves its
  `include 'lib/io.x68'` to `dir/lib/io.x68`. A File named `.asm`, `.x68`,
  `.m68k`, `.s` or `.inc` goes in as text and every other File as bytes, which
  is the only thing a directory says about which is which.
- **It replaces step 16's scan of the `include` lines, for two reasons that are
  both about the Assembler and not about disks.** A written name resolves
  *beside the including File first and at the project root second*, and a scan
  can follow only one of the two: `lib/a.asm` including `b.asm` that sits at the
  root read nothing, and the Assembler then said the File was missing. And
  `unreadable_file`'s suggestions are the closest paths **of the Project**, so a
  Project made of the names that failed can never offer the File that was meant
  — the message the whole diagnostic exists for. `cargo run` now prints "there
  is no file named `lib/oi.x68` in this project" with "hint: did you mean
  `lib/io.x68`?" under it, which the scan could not have said, because the File
  that was meant was never read.
- **What it costs is three rules of the walk's own**, because a directory on
  disk is not a Project and nothing promises that it is small: a directory whose
  name starts with `.` is not entered, `target` and `node_modules` are not
  entered, and no more than 1000 Files and 32 MiB are read, one line on standard
  error saying so when something is left out. The numbers are not arbitrary:
  this repository is 326 Files and 11.3 MiB with those two names skipped, which
  `cargo run` with no argument reads in 0.1 s, and `target/` alone is 3.6 GB —
  the default Entry file `code-to-run.asm` sits beside it, so a walk without
  that rule would read a disk image to assemble twelve lines.
- **Source Files are read first and everything else with what is left**, and a
  File that does not fit is left out while the walk goes on. The budget is a
  backstop and it should never decide which File a program gets: a directory of
  1100 assets beside `main.asm` and its `lib.asm` reads both of them, reports
  that some Files were left out, and assembles. Reading in the directory's own
  order would have spent the budget on the assets and then told the student
  that `lib.asm` is not in the project, which is a message about the walk
  pretending to be a message about the program.
- **The Entry file is read as source whatever it is called**, before the walk,
  so `cargo run -- notes.txt` assembles it rather than reporting that the Entry
  file holds bytes. The command line named it: it is the program.
- **A source File that is not UTF-8 is read as Latin-1** (ADR 0004), where step
  16's reader put it in as bytes. A File written by EASy68K holds bytes and not
  code points, so `dc.b 'é'` is the single byte `$E9` on disk; read as Latin-1
  it is the one character the Assembler then writes back as `$E9`, and read as
  UTF-8 (when it is UTF-8) it is the same character. The conversion is total, so
  no File on disk can stop the command line from assembling.
- **`src/main.rs` no longer resolves anything**, so `include::join`'s
  documentation no longer says it is public "because `src/main.rs` reads a
  Project ... with it"; it says the general reason instead, that a caller
  outside the Assembler which has to resolve a written file name has no second
  implementation of the rule to use. Marked below as the correction it is.
- **Four tests, in a binary that had none.** The walk over a directory of six
  Files (with `target/` and a dot directory among them), the Entry file's
  extension not mattering, `is_source` over nine names, and the Latin-1
  fallback. They write into the system's temporary directory under a name of
  their own, so they do not collide.

#### The three factual corrections

1. **A Local label's full name has no dot in it**, and three doc comments said
   it did: `src/ts_types.rs`'s `ProgramSymbol` (the one that is published, which
   an editor writing a symbol list would have believed), `program.rs`'s
   `ProgramSymbol` and `symbols.rs`'s `SymbolTable::resolve`. `qualify` builds
   `start:loop` — EASy68K's own rule, "replacing the dot with a colon", which
   step 7's bullet states correctly and `symbols.rs`'s own module comment
   repeats — and the smoke test now asserts `SUM:loop` through the whole chain,
   which is how the three comments were caught.
2. **Step 16's "The command line reads a Project from disk" bullet is marked
   superseded**, in place, with a sentence naming this step and the reason. The
   bullet itself is left as the record of what step 16 built, which is what the
   implementation notes are for.
3. **`tests/corpus/README.md`'s example of a diagnostics fixture** showed a
   `simhalt` "not implemented yet" entry, which phase 2 removed from the
   snapshots — the same README says so further down, where it records that the
   `unimplemented_operation` on `SIMHALT` is gone from `clockDigital-errors`.
   The example is now the first entry `graphicSound-errors.snap` actually
   holds, a `bare_comment`.

**What is left.**

* **The public surface is closed**, and the design record's "Public API" section
  now says so. Nothing in the repository — crate, wrapper or document — promises
  an unimplemented feature any more; the `unimplemented_*` sentences name
  `rte`, `stop`, `reset`, `move usp,an`, `memory`, macros, conditional assembly
  and structured control, and none of them says "yet".
* **What an editor gains, and what it does not.** `InstructionLine.includeChain`
  and `Diagnostic.related` are the two places a chain arrives; a Program still
  carries no list of the Files a build read, which is step 16's note and is
  still true. Four codes are new since 1.4.2's editor was written
  (`unreadable_file`, `include_cycle`, `include_too_deep`,
  `end_in_an_included_file`).
* **The two carried findings of the phase 3 review are still carried** (`move
  sr,sr` and `movep 0(a0),0(a1)`'s per-position message, and `chk d0,#5`'s
  "nothing can be written to it"), both in `analyzer.rs`, both with their fix
  written out in step 16's notes. They are the only work of phases 1 to 4 that
  is known and not done.
* **`web/`** still calls 1.4.2's `new S68k(code)` against a committed `pkg/`
  that no longer exists, and is in no CI job; `README.md` has said so since step
  9 and this step did not change it.

### Phase 4 review: nine minor findings left open (2026-09-08)

The s68k side of phase 4 was reviewed and passed with no blocker or major finding. The nine minor ones were not fixed, because the work was stopped before the asm-editor step to save usage; they are listed here for whoever continues.

- **`unreadable_file` loses its suggestions exactly when the miss is written from a subdirectory** — Project `main.m68k` -> `include lib/a.m68k`, `lib/a.m68k` -> `include oi.m68k`, with `lib/io.m68k` present: `error unreadable_file lib/a.m68k:0 | there is no file named `oi.m68k` in this project | hint: None`. The same misspelling written from the root (`include lib/oi.m68k`) does get `hint: did you mean `lib/io.m68k`?`. Cause: `include.rs` `resolve_in` throws the candidate list away and reports ` Fix: Hand `candidates(written, from)` (or the including File's directory) to `closest_paths` and take the best suggestion over all of the paths that were actually tried, rather than over `join("", written)` alone; that turns this case into `did you mean `lib/io.m68k`?` with no new rule.
- **An `include` after `end` can stop the build, which is a new strictness ADR 0001 does not list** — `    nop\n    end\n    include gone.m68k\n` gives `warning code_after_end main.m68k:2 | this line comes after `end` and is not assembled` immediately followed by `error unreadable_file main.m68k:2 | there is no file named `gone.m68k` in this project`, and no Program. EASy68K stops assembling at END and never reads the line, so this is a program that assembles there and not here. Step 16 records th Fix: Add one bullet to ADR 0001's stricter-than-EASy68K list naming this consequence; or, if the EASy68K behaviour is preferred, have `Expander::expand_file` stop following `include` lines after a line whose operation is `end` in the same File, and say so in step 16's bullet instead.
- **An unquoted file name containing a space gives a message about a fragment, with no hint** — `    include my lib.m68k` with `my lib.m68k` in the project: `error unreadable_file main.m68k:0 | there is no file named `my` in this project | hint: None`, plus a `bare_comment` suggestion on the trailing `lib.m68k`. `Directives/include.htm` states the rule this student broke ("must be enclosed in single (') or double (\") quotes if any part of the file path or name includes spaces"), so this is Fix: When the written name has no extension and the line's Comment field is bare, add the hint "quote a file name that holds spaces, `include 'my lib.m68k'`"; the two facts are already on hand in `plan_include`/`file_name_span`.
- **A suggestion can point at a File of the kind the Directive cannot read** — `    include io` with a binary `io.bin` in the project: `there is no file named `io` in this project | hint: did you mean `io.bin`?`; following the hint then gives `` `io.bin` holds bytes, not source ``. Symmetrically `    incbin sprite` with `sprite.m68k` present suggests `sprite.m68k`, which `incbin` can read, so only the `include` direction is a dead end. It comes from `closest_paths`'s stem ru Fix: Pass the Directive (already carried as `directive` on the kind) into `closest_paths` and, for `include`, rank or filter to text Files; a binary File is still worth naming last, since "it is there but it is bytes" is itself the answer.
- **`include ''` is reported as a line that writes no operand** — `    include ''` and `    incbin ''` give `` `include` takes one operand, and this line has none `` / `` `incbin` takes one operand, and this line has none ``. The line does write an operand — an empty quoted name — so the sentence describes a line the student did not write. `include_of` and `file_name_quietly` both map an empty `written_path` to `None`, and step 16 records only "an empty name is Fix: Either give the empty name its own sentence ("the file name between the quotes is empty"), or keep the reduction and add the rendered message to step 16's bullet so the record says what a student sees.
- **`UnreadableFile`'s `alone` and `suggestions` are hardcoded on the binary-`include` path** — `src/assembler/include.rs`, `Expander::follow`, the `Resolved::Bytes` arm builds `DiagnosticKind::UnreadableFile { suggestions: Vec::new(), binary: true, alone: false }` with `alone` written as a literal rather than `files.len() <= 1`. It is invisible today because `hint()`'s `(true, _)` arms read neither field, but it is a field of a public kind carrying a value that is not a fact about the Proje Fix: Build the kind through `missing_file`-style construction, or set `alone: files.len() <= 1` there too, so every `UnreadableFile` describes the Project it was raised against.
- **The command line prints a related `include` location's file twice, two different ways** — `cargo run -- /abs/path/proj/main.asm` on a diagnostic inside an included File prints `/abs/path/proj/main.asm:3:13: included from `main.asm``: the Location half is mapped back through `on_disk` to the path on disk, and the message half is the Project path the Assembler wrote. Both halves name the same file and neither is wrong, but the line reads as if two files were involved. Fix: In `print_diagnostic`, drop the file name from the related message when it equals the related Location's own file (print just "included from here"), or map the message's name through `on_disk` as well.
- **One unwrapped line in the rewritten `simhalt` bullet of docs/grammar.md** — §2.6's rewritten bullet ends `...and\n  no rule of the parser may consult the Directive's arity (ADR 0003). It is the one Directive that produces an executable item: four` — about 115 columns where every other line of the file wraps near 80. It is the seam where finding 9's replacement text was spliced onto the old sentence. Fix: Re-wrap that paragraph to the file's width.
- **The line budget still assembles 200,000 lines before it fires** — A deliberately multiplying Project (a chain of eight Files each including the next twice, bottoming out in a 3,000-line File) assembles for 1.86 s in a debug build before reporting one `include_too_deep` ("including `a7.m68k` would take this assembly past 200000 lines") and no Program. The backstop bounds the work, as ADR 0001 now says, but the bound is a full 200,000-line layout, and the stated m Fix: Nothing required. If it is ever measured to matter, the cheap half is to check the budget against the target's *transitive* size rather than its own line count, so a runaway is refused before the lines are laid out.

