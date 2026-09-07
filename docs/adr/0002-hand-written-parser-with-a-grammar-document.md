---
status: accepted
date: 2026-09-07
---

# The parser is hand written; the grammar is a document, not an executable artifact

The regex lexer could not tell a label from an instruction, rewrote EQU names inside other words, and had no notion of where a token sat on the line, so the parser is being rewritten. We decided on a hand-written tokenizer and recursive-descent parser, with a Pratt loop for expressions and a source span on every token and node, specified by an EBNF in `docs/grammar.md` that the parser follows rule by rule and the tests are named after. The diagnostics are the product of this crate, and only a hand-written parser lets each message be composed at the point where the parser knows what it expected and why; the language is line oriented, so recovery is "report, skip to the end of the line, continue"; and there is no code generation step or heavy dependency in a crate that ships as WebAssembly.

## Considered options

- **Parser generator (`pest`, `lalrpop`)**: the grammar file would be authoritative, but its errors are lists of expected tokens, recovery is weak, and teaching-grade messages would be bolted on around the generated parser anyway.
- **Parser combinators with recovery (`chumsky`)**: good errors, at the cost of heavy generic code, long compile times, an API that still churns and a larger binary.

## Consequences

- `docs/grammar.md` is the specification; a change to what the parser accepts is a change to that document first.
- Macro expansion, when it arrives, is a token-level operation on the tokenizer's output rather than a text substitution.
