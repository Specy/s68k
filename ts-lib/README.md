# @specy/s68k

A rust assembler and interpreter for m68k, written to explain what is wrong with a program as much as to run it.
It compiles to WASM to be used with javascript and node.js, [available on npm](https://www.npmjs.com/package/@specy/s68k).

[Typescript library documentation](https://github.com/Specy/s68k/wiki)
It is part of a family of javascript assembly interpreters/simulators:

- MIPS: [git repo](https://github.com/Specy/mars),  [npm package](https://www.npmjs.com/package/@specy/mips)
- RISC-V: [git repo](https://github.com/Specy/rars), [npm package](https://www.npmjs.com/package/@specy/risc-v)
- X86: [git repo](https://github.com/Specy/x86-js), [npm package](https://www.npmjs.com/package/@specy/x86)
- M68K: [git repo](https://github.com/Specy/s68k), [npm package](https://www.npmjs.com/package/@specy/s68k)

## Purpose
The purpose of this interpreter is to help people learn the basics of assembly, in this case m68k, by providing useful errors and hints like a modern language, in the hope to help understand the addressing modes and learn the different instructions/directives.
**WARNING**
It wasn't made to assemble or make actual programs, but marely as a learning tool, don't expect 100% accuracy.

## Usage

```bash
npm install @specy/s68k
```

Everything starts at `S68k.assemble`. It answers the diagnostics it found and,
when none of them is an error, the program to run.

```ts
import {S68k, Interpreter, InterpreterStatus} from '@specy/s68k'

const {diagnostics, program} = S68k.assemble(`
    ORG $1000
START:
    MOVE.L  #5,D1
    ADD.L   #3,D1
    MOVE.B  #9,D0
    TRAP    #15
`)

for (const d of diagnostics) {
    // severity: "error" | "warning" | "suggestion"
    console.log(`${d.location.file}:${d.location.line + 1}: ${d.severity}: ${d.message}`)
    if (d.hint) console.log(`  ${d.hint}`)
}

if (program) {
    const interpreter = new Interpreter(program)
    while (interpreter.getStatus() === InterpreterStatus.Running) interpreter.step()
    console.log(interpreter.getCpuSnapshot().getRegistersValues())
    interpreter.dispose()
    program.dispose()
}
```

A diagnostic is a plain object, and only an `error` stops the program from being
built — a `warning` or a `suggestion` comes back with a program beside it:

```ts
{
    severity: "error",
    code: "invalid_addressing_mode",          // stable, snake_case, safe to match on
    message: "the second operand of `move` cannot be an immediate",
    hint: "an immediate is a value, and nothing can be written to it; there it takes Dn, An, (An), ...",
    location: {file: "main.m68k", line: 1, column: 15, endColumn: 17},
    related: []                               // [{location, message}], e.g. the first definition of a name
}
```

Lines and columns are 0-based, and every location names its file. To assemble a
project of several files, or to give the source the name the editor knows it by:

```ts
S68k.assemble({files: {'main.x68': source, 'lib/io.x68': library}, entry: 'main.x68'})
S68k.assemble(source, {entry: 'lecture-1.x68'})
```

`include` and `incbin` are not implemented yet, so only the entry file is read
and the directives are reported; a project of one file works today.

### Running, stepping and interrupts

`TRAP #15` asks the host to do something — print a string, read a number, draw a
pixel. The interpreter stops with the status `Interrupt` and waits for an answer:

```ts
const status = await interpreter.runWithInterruptHandler(async (interrupt) => {
    switch (interrupt.type) {
        case 'DisplayStringWithCRLF':
            console.log(interrupt.value)
            return {type: 'DisplayStringWithCRLF'}
        case 'ReadNumber':
            return {type: 'ReadNumber', value: 42}
        default:
            throw new Error(`unhandled interrupt ${interrupt.type}`)
    }
})
```

### Debugging

```ts
interpreter.runWithBreakpoints([{file: 'main.m68k', line: 12}])
interpreter.getCurrentLocation()      // {file, line, column, endColumn} | null
interpreter.getNextInstruction()      // {address, size, location, source} | null
interpreter.getCallStack()            // one frame per subroutine entered
interpreter.canUndo() && interpreter.undo()
```

A breakpoint is a line of a file: one on a comment, a directive or a label alone
stops nothing. Undo needs a history, which `new Interpreter(program)` keeps by
default (`{keep_history: true, history_size: 100}`).

### Reading one line

`S68k.parseLine(text)` reads a single line into its four fields, for hover and
highlighting. It never throws and reports nothing — a line out of its file
cannot know a symbol, an address or an instruction's operand rules.

```ts
S68k.parseLine('start:  move.w #$10,(a0)+  ; go')
// {
//   kind: "instruction",
//   label: {name: "start", colon: true, span: {start: 0, end: 5}},
//   operation: {
//     name: "move", size: "word", nameSpan: {...}, span: {...},
//     operands: [
//       {mode: "immediate", description: "an immediate", text: "#$10", span: {start: 15, end: 19}},
//       {mode: "postincrement", description: "a postincrement operand", text: "(a0)+", span: {start: 20, end: 25}}
//     ]
//   },
//   comment: {kind: "explicit", text: "; go", span: {start: 27, end: 31}}
// }
```

Spans are character columns of the line, `end` exclusive, so they line up with a
diagnostic's columns.

### Memory

`Program` and `Interpreter` hold memory on the WebAssembly side, which garbage
collection cannot reach. Call `dispose()` on both when a program is done with.
`S68k.assemble` frees the assembly itself when the source did not build one, so
live checking on every keystroke leaks nothing.

## Migrating from 1.4.2

| 1.4.2 | 2.0 |
|---|---|
| `new S68k(code)`, `s68k.semanticCheck()` | `S68k.assemble(code).diagnostics` |
| `S68k.compile(code)` → `{ok, errors, interpreter}` | `S68k.assemble(code)` → `{diagnostics, program?}` then `new Interpreter(program)` |
| `SemanticError` class, `getMessage()`, `getLineIndex()` | a plain `Diagnostic` object with `code`, `message`, `hint`, `location` |
| `S68k.lex`, `S68k.lexOne`, `LexedLine`, `ParsedLine` | `S68k.parseLine(text)` |
| `getCurrentLineIndex(): number` | `getCurrentLocation(): Location \| null` |
| `runWithBreakpoints(Uint32Array)` (line indexes) | `runWithBreakpoints([{file, line}])` |
| `getInstructionAt(address)` → an instruction with a parsed line | → `{address, size, location, source}` |
| `stepGetStatus()` | `step()`, which answers the status (`stepGetStatus` still works) |

Diagnostics are reported where 1.4.2 was silent, and a few programs that
assembled by accident no longer do; `equ` names a value rather than substituting
text.

## Supported instructions
| Type                   | Instructions                                                                                                                                                                                                      |
|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Arithmetic             | add, sub, suba, adda, divs, divu, muls, mulu, addq, subq, addi, subi                                                                                                                                              |
| Comparison             | tst, cmp, cmpi, cmpa, cmpm                                                                                                                                                                                        |
| Branching and jumping  | bcc, bcs, beq, bne, blt, ble, bgt, bge, bls, bhi, bpl, bmi, blo, bhs, bvc, bvs, bsr, bra, jsr, rts, dbcc, dbcs, dbeq, dbne, dbge, dbgt, dble, dbls, dblt, dbhi, dbmi, dbpl, dbvc, dbvs, dbf, dbt, dbhs, dblo dbra |
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo                                                                                                                            |
| Bitwise                | not, or, and, eor, lsl, lsr, asr, asl, rol, ror, btst, bclr, bchg, bset                                                                                                                                           |
| Other                  | clr, exg, neg, ext, swap, move, link, unl, lea, pea, moveq, movea, movem                                                                                                                                          |
| Interrupt              | trap #15, with implemented interrupts from 0 to 7                                                                                                                                                                 |

## Supported directives
`org`, `equ`, `set`, `dc`, `dcb`, `ds`, `end`, and `opt`, `list`, `nolist` and
`page`, which are accepted and ignored. Without `end`, the entry point is a
label named `START`, and failing that the first instruction.

`include`, `incbin`, `reg`, `fail`, `simhalt`, `offset` and `section` are
recognised and refused with a diagnostic naming the feature; so are the macro and
conditional-assembly directives and EASy68K's structured control.

## Known limitations
1. Characters are one byte, read and written as Latin-1; a source character with no byte of its own is an assembly error.
2. Every instruction is four bytes wide whatever it encodes to on a real 68000, so an address computed from instruction sizes will not match the hardware.
3. The program runs as supervisor, always.

# How to build
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the whole package by running `npm run build-all` in the `ts-lib` folder of the project: it builds the wasm package into `ts-lib/src/pkg` and then the TypeScript library into `ts-lib/dist`. `npm test` runs a smoke test over the built `dist/`.
