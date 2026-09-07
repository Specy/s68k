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
- **Comment**: a line starting with `*` or `;`; `;` anywhere; `*` after a complete operand field; and EASy68K's bare comment after the operand field, flagged once per file as a `suggestion` to use `;`. A comment field starting with a comma gets its own hint (almost always a broken operand list). `*` where an expression term can stand is the current address or multiplication, so `lea *,a0` and `org (*+1)&-2` work; `#2 * 3` ends the operand at `#2` and the error says that expressions cannot contain spaces.
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
- `warning`: odd `org` (rounded up), character literal over four characters, constant over 32 bits, code after `end`, `end` without an address. `suggestion`: bare comment (once per file), comment field starting with a comma, a bare number below the program's origin where `#` was probably meant. Runtime errors stay separate.

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
- Addressing modes: PC-relative displacement and index; `.w`/`.l` on absolute addresses; `.s`/`.w`/`.l` on branches, accepted without a range check until real sizes exist.
- One instruction table for mnemonics, operand rules, sizes and defaults, shared by parser, analyzer and encoder.

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

### Tests

1. Golden fixtures of 1.4.2's output for the 30 editor programs (25 lecture playgrounds, 5 runnable `.x68`; all 30 assemble on 1.4.2, baseline run on 2026-09-07), updated only on purpose with a note.
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
