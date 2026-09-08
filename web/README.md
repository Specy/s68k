# The `web` demo — not built, not ported to 2.0

A small webpack and Monaco page that drove the library's raw WebAssembly
surface. **It does not build against 2.0 and is in no CI job.**

`web/src/index.ts` imports `S68k`, `SemanticError` and the interpreter from
`../../pkg/s68k` and calls `new S68k(text)`, `wasm_semantic_check()`,
`wasm_compile()` and `wasm_create_interpreter(...)`. The assembler rewrite
removed all four (the design record, "Public API, `@specy/s68k` 2.0"): a program
is assembled once with `S68k.assemble(source)`, which answers
`{ diagnostics, program? }`, and an interpreter is `new Interpreter(program)`.
The `pkg/` folder it imports is no longer committed either; it is what
`wasm-pack build` writes at the root, and it is ignored.

Porting it means, in `web/src/index.ts`:

* build the page against `ts-lib` (`@specy/s68k`) rather than against the raw
  `pkg/` bindings, so that the demo uses the same wrapper the asm-editor does;
* replace the compile button's body with one `S68k.assemble(text)`, and render
  `assembly.diagnostics` — each one has a `severity`, a `code`, a `message`, an
  optional `hint` and a `location` with a line and columns, which is more than
  the old `wasm_get_message()` string the page prints today;
* replace `wasm_get_current_line_index()` with `getCurrentLocation()`, which
  answers `{ file, line }`, and read a step's instruction from
  `getNextInstruction()`, whose shape is `{ address, size, location, source }`;
* call `dispose()` on the program and the interpreter when the page drops them.

`ts-lib/test/smoke.mjs` is the whole 2.0 API exercised in one file and is the
example to copy from. Until someone does that, this folder is kept only so that
the page's layout and its memory table are not lost.
