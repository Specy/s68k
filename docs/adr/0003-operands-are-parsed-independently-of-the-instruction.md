---
status: accepted
date: 2026-09-07
---

# Operands are parsed independently of the instruction, so that every rejection can name the alternatives

EASy68K answers a wrong operand with "Invalid addressing mode" or "Invalid syntax", which tells a student nothing about what would have been right. We decided that the parser accepts any well-formed operand anywhere, the full set of 68000 addressing modes plus register lists, SR, CCR and USP, and that the analyzer alone judges it against the instruction table: which modes the instruction allows in that position, which sizes, which value ranges. A rejection can then say what was found, what is allowed, and what the student probably meant (`(a0)` for `(d0)`, `#5` for a bare `5` below the program's origin, `.w` or `.l` for a byte move into an address register), and an unknown mnemonic gets a did-you-mean from the same table, the "did you mean a label" hint, or the reason the feature is not implemented. Even a malformed operand is reported as what it tried to be, never as a syntax error.

## Consequences

- The instruction table is the single source of truth for mnemonics, operand rules, sizes and defaults; the parser only uses it to recognise mnemonics for the label rule.
- A grammar rule that rejects an operand shape early is a regression, even when the shape is never valid for any instruction.
- Heuristic hints ("did you mean `#5`") are `suggestion` diagnostics when the code is legal but probably wrong, never errors.
