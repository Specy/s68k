---
status: accepted
date: 2026-09-07
---

# EASy68K is the reference dialect, and s68k is a lenient superset of it

Students bring programs written for EASy68K, the assembler their courses and textbooks use, and the asm-editor's documentation already points at it as the reference. We decided that s68k follows EASy68K's grammar and meaning wherever a feature is implemented, so that an EASy68K program assembles here unchanged, and that s68k keeps the leniencies it already had rather than adopting EASy68K's stricter column rules, so that the programs saved in the editor keep assembling too. Features s68k lacks are reported as "not implemented", never given a different meaning.

## Considered options

- **Strict EASy68K/Motorola MASM rules**: a word in column 1 is always a label and the operation must be indented. Rejected because every program that starts its instructions in column 1, which the editor has always allowed, would stop assembling.
- **Own dialect, EASy68K inspired**: free to diverge where it teaches better. Rejected because "my class program does not assemble" costs more than any single improvement would gain, and the educational choices can be made as diagnostics instead.

## Consequences

- A label is recognised by position or by colon: an identifier in column 1 that is not a mnemonic or directive name, or an identifier followed by a colon anywhere. A label named exactly like a mnemonic must carry the colon; that is the one case EASy68K users never hit, since their labels sit in column 1 and their instructions are indented.
- The operand field ends at whitespace not adjacent to a comma, so EASy68K's bare comment field (`move.b #23,d0   trap task 23`) is accepted, while `d0, d1` keeps working. A bare comment is flagged once per file as a `suggestion` to use `;`, which is also what introduces non-error severities into s68k. A comment field beginning with a comma gets its own hint, since it is almost always a broken operand list.
- `*` is the current address, or multiplication, wherever an expression term can stand; it is a comment marker only at the start of a line or after a complete operand field.
- Where s68k is stricter than EASy68K it is on purpose and listed here: two lines placing bytes or an instruction at the same address is an error (EASy68K does not check), because instructions do not live in memory here and an overlap would be silently incoherent.
- `org` may move the current address anywhere, including backwards, as in EASy68K; word and long data and instructions are aligned to even addresses as EASy68K does; an odd `org` gets EASy68K's warning and is rounded up. Instructions stay 4 bytes each for now (the interpreter has never loaded real encodings), with the size stored per instruction so that real sizes can come later without touching the front end.
- `end` inside an included file is an error rather than, as in EASy68K, the point where assembly silently stops and the rest of the entry file is dropped.
- One old leniency is dropped: `equ` no longer aliases arbitrary text (`ten equ #10`, `reg equ d1`); it defines a value, as in EASy68K, and the error for the old forms says how to rewrite them. Keeping it would have meant keeping text substitution, which is what corrupted other names.
