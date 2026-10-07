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

`simhalt` makes an interpreter return `InterpreterStatus.Paused` without
terminating it. Calling `step()`, `run()`, `runWithLimit()` or
`runWithBreakpoints()` again resumes at the following instruction.

A diagnostic is a plain object, and only an `error` stops the program from being
built — a `warning` or a `suggestion` comes back with a program beside it:

```ts
{
    severity: "error",
    code: "invalid_addressing_mode",          // stable, snake_case, safe to match on
    message: "The second operand of `move` cannot be an immediate.",
    hint: "The operand should be Dn, An, (An), ... . An immediate is a value, and nothing can be written to it",
    location: {file: "main.m68k", line: 1, column: 15, endColumn: 17},
    related: []                               // [{location, message}], e.g. the first definition of a name
}
```

Lines and columns are 0-based, and every location names its file.

### A project of several files

A project is a map from a root-relative path to a file's text, or to the bytes
of a binary one, plus the path of the entry file to start from. Every other file
is reached from the entry file through `include` or `incbin`, or is not read at
all: nothing here opens a disk, so the editor hands over the buffers it has.

```ts
const {diagnostics, program} = S68k.assemble({
    files: {
        'main.x68': main,                          // the entry file
        'lib/io.x68': library,                     //   INCLUDE 'lib/io.x68'
        'data/sprite.bin': new Uint8Array(bytes)   //   INCBIN  'data/sprite.bin'
    },
    entry: 'main.x68'
})

// or one buffer under the name the editor knows it by
S68k.assemble(source, {entry: 'lecture-1.x68'})
```

`include` pastes a file's lines in where the line is — same section, same
address, one namespace — and `incbin` puts a file's bytes in memory as a `dc.b`
of the whole file would. A path is looked for beside the file that wrote it
first and at the project root second, and a file the project has not got is an
error naming the closest one it has:

```ts
{
    severity: "error",
    code: "unreadable_file",
    message: "there is no file named `io.x68` in this project",
    hint: "did you mean `lib/io.x68`?",
    location: {file: "main.x68", line: 0, column: 12, endColumn: 20},
    related: []
}
```

A diagnostic raised in an included file is reported where it is written, and
`related` carries the `include` lines it was reached through:
`{location: {file: 'main.x68', line: 1, ...}, message: 'included from `main.x68`'}`.
An assembled instruction carries the same chain as `includeChain`, innermost
first and empty for the entry file — a file included twice has one location per
line and two addresses, and the chain is what tells the two copies apart.

### Running, stepping and interrupts

`TRAP #15` asks the host to do something — print a string, read a number, draw a
pixel. The interpreter stops with the status `Interrupt` and waits for an answer:

```ts
const status = await interpreter.runWithInterruptHandler(async (interrupt) => {
    switch (interrupt.type) {
        case 'DisplayStringWithCRLF':
            console.log(interrupt.value)
            return {type: 'DisplayStringWithCRLF'}
        case 'DisplayNumberInBase':          // task 15: 'FF', already formatted
            process.stdout.write(interrupt.value)
            return {type: 'DisplayNumberInBase'}
        case 'ReadNumber':                   // task 4: the line as it was typed
            return {type: 'ReadNumber', value: '42'}
        case 'ReadChar':                     // task 5: one key; Enter is '\r' or '\n'
            return {type: 'ReadChar', value: '\n'}
        default:
            throw new Error(`unhandled interrupt ${interrupt.type}`)
    }
})
```

The interpreter owns what a task means, as EASy68K defines it, and the host
only carries text. A display task's `value` is the text to display: decoded
from Windows-1252 and, for a number, formatted the way EASy68K formats it — task
3 is signed decimal, task 15 an unsigned number in upper case, task 20 a number
in a field whose width is a signed byte, so a negative one pads on the right. A
read task is answered with what was typed, unparsed: the line for
`ReadKeyboardString`, `ReadNumber` and `DisplayStringAndReadNumber`, the key for
`ReadChar`.

- **A line** keeps its first 79 characters, as EASy68K does, and ends at its
  first line terminator. Task 2 stores it at (A1) with a NUL and puts the count
  in the whole of D1.L. Tasks 4 and 18 read it with `atoi`: `'12abc'` is 12,
  `'abc'` and `''` are 0, never an error, and a number too long for 32 bits
  wraps, so `'4294967295'` is `$FFFFFFFF`.
- **A key** goes into D1.B. Enter is `$0D`, EASy68K's code, whether the host
  sends `'\r'` or `'\n'`.
- A typed character Windows-1252 has no byte for is stored as `?`.

Every interrupt is answered by the `InterruptResult` of its own `type`, with a
value the task can take, or by `{type: 'Terminate'}`, which answers any task and
ends the program. Anything else throws a `RuntimeError` object and changes
nothing, and the interrupt still waits:

```ts
try {
    interpreter.answerInterrupt({type: 'ReadNumber', value: 42})
} catch (error) {
    // {type: 'InvalidAnswer', value: {interrupt: 'ReadNumber', reason: '...'}}
}
```

`NoPendingInterrupt` is an answer with nothing waiting, `InvalidAnswer` an
answer of another type, a value that is not an answer, a key state of the other
form, more bytes than a read asked for or a file number past 7, and
`OutOfBounds` a line that does not fit in memory.

#### Input settings

Task 12 turns the echo of what is typed off and on, and task 16 the input prompt
(EASy68K's flashing cursor) and the line feed a key read of Enter echoes after
the carriage return. They raise no interrupt; a host reads them while a read
task waits:

```ts
interpreter.getInputSettings()   // {echo: true, prompt: true, line_feed: true} when a program starts
```

Undo puts them back: a task that changed them carries a `SetInputSettings`
mutation with the old and the new settings. With the echo off, Enter still ends
a line read on a new line, as in EASy68K.

#### Files

Tasks 50 to 59 are done by the host's file system. Each interrupt carries its
arguments decoded, the answer says what the file system did, and the interpreter
writes EASy68K's result to D0.W: 0 success, 1 end of file, 2 error, 3 read only.

```ts
switch (interrupt.type) {
    case 'OpenFile':     // task 51: read and write an existing file, or read it only
        return {type: 'OpenFile', value: {handle: 0, read_only: false}}   // or null
    case 'NewFile':      // task 52: create or empty the file
        return {type: 'NewFile', value: 0}                                // or null
    case 'ReadFile':     // task 53: {handle, count}
        return {type: 'ReadFile', value: fileBytes.slice(position, position + interrupt.value.count)}
    case 'WriteFile':    // task 54: {handle, bytes}, a Uint8Array
        return {type: 'WriteFile', value: true}
    case 'PositionFile': // task 55: {handle, offset} from the start
    case 'CloseFile':    // task 56: the handle
    case 'DeleteFile':   // task 57: the path
    case 'CloseAllFiles':// task 50
        return {type: interrupt.type, value: true}                        // false when it failed
    case 'FileExists':   // task 59: the path
        return {type: 'FileExists', value: 'Writable'}                    // 'ReadOnly', 'Missing'
    case 'FileDialog':   // task 58: {mode: 'Open' | 'Save', title, filter, path}
        return {type: 'FileDialog', value: 'scores.txt'}                  // null for a cancel
}
```

- **A path** is the NUL terminated string at the address, at most 255
  characters, decoded from Windows-1252, with `\` written as `/`.
- **A file number** is 0 to 7, the lowest free one: a host keeps at most eight
  files open and answers `null` to a ninth open. Task 51 or 52 then puts -1 in
  D1.L and 2 in D0.W, and the file number in D1.L when it opened.
- **A read** of some bytes, fewer than asked for included, writes them to (A1),
  their count to D2.L and 0 to D0.W; no bytes is the end of the file, 1 in D0.W
  with D2.L left as it was; `null` is 2. The bytes are a `Uint8Array` (an array
  of numbers is taken too), and undo puts back what they overwrote.
- **A file that cannot be written** opens for reading only with 3, and a write
  to it fails with 2.
- **The dialog** writes the path chosen to (A3), at most 255 characters and NULs
  to 256 bytes, and 1 to D1.L; a cancel puts 0 in D1.L.
- **No interrupt** is raised where the interpreter can decide alone, and D0.W is
  2: a file number outside 0 to 7 for tasks 53 to 56, a read or a write of no
  bytes, a buffer past the end of memory, a negative position.

#### Sound

Tasks 70 to 77 carry their arguments, the file name or the sound's index in
D1.B and the control in D2.L: `PlaySound`, `LoadSound`, `PlayLoadedSound`, their
`DirectX` forms, `ControlSound` and `ControlSoundDirectX`. A host answers
whether it played, 1 or 0 in D0.W (`LoadSound` has no result), or `Terminate`
when it has nothing to play them on.

#### Errors, and why a program ended

Every method throws a `RuntimeError` object, `{type, value}`, and none of them
panics, so the interpreter stays usable after a bad call. A runtime error an
instruction raises ends the program: the status is `TerminatedWithException`,
and `getTermination()` answers the error as the cause. A `trap #15` task adds
two errors of its own:

```ts
{type: 'UnsupportedTrapTask', value: {task: 30}}            // the cycle counter, the network, a number that is no task
{type: 'InvalidTrapArgument', value: {task: 15, reason: 'D2.B is 37, and a base is 2 to 36'}}

interpreter.getTermination()
// null while it runs, then {type: 'TerminateTask'} (task 9), {type: 'EndOfProgram'},
// {type: 'TerminatedByHost'} (a Terminate answer) or {type: 'Exception', value: error}
```

Undoing the step that ended the program brings it back, running. The tasks s68k
does not carry out are the printer (10), the text window's font and contents
(21, 22, 25), the cycle counter (30, 31), the hardware window (32), the serial
ports (40 to 43), the interrupt requests (60, 62) and the network (100 to 107).

Where this departs from EASy68K:

- a base outside 2 to 36 for task 15, a D1.B other than 0 to 3 for task 16 and
  a D1.L other than 0 or 1 for task 58 end the program, where EASy68K does
  nothing;
- a number that is no task ends the program, where EASy68K takes the trap
  exception;
- a D1.W of `$8000` or more for tasks 0 and 1 displays up to 255 characters, as
  above 255;
- overflow in `atoi` wraps, Borland's own being unverified;
- the characters typed after the 79th of a line are dropped;
- a read of no bytes is 2, where EASy68K says 1 once an earlier read reached the
  end of the file;
- a buffer whose count passes 4 GB is 2, where EASy68K's check wraps round, and
  task 58 checks all 256 bytes it writes;
- tasks 70 and 72 write the answer to D0.W, where EASy68K leaves it when the
  sound is missing.

### Debugging

```ts
interpreter.runWithBreakpoints([{file: 'lib/io.x68', line: 12}])
interpreter.getCurrentLocation()      // {file, line, column, endColumn} | null
interpreter.getNextInstruction()      // {address, size, location, includeChain, source} | null
interpreter.getCallStack()            // one frame per subroutine entered
interpreter.canUndo() && interpreter.undo()
```

A breakpoint is a line of a file, any file of the project: one on a comment, a
directive or a label alone stops nothing, and one on a line of a file included
twice stops at both copies. Undo needs a history, which `new Interpreter(program)` keeps by
default (`{keep_history: true, history_size: 100}`).

`program.getInstructionAddresses()` returns the assembled instruction addresses
in ascending order. It is intended for whole-build editor annotations: use
`interpreter.getInstructionAt(address)` to obtain each instruction's source
location without copying every full instruction across the WebAssembly boundary
up front.

### Pokes: changing a value between two instructions

A poke is a register or memory value the host changes while debugging, recorded
in the same history as the instructions as a step of its own:

```ts
interpreter.beginPoke()
interpreter.setRegisterValue({type: 'Data', value: 0}, 0x99)
interpreter.writeMemoryBytes(0x2000, new Uint8Array([9, 9]))
const recorded = interpreter.endPoke()   // true: it wrote something

const [step] = interpreter.getUndoHistory(1)
step.kind      // 'poke', where an instruction's is 'instruction'
step.writes    // [{type: 'register', name: 'd0', old: 1, new: 0x99},
               //  {type: 'memory', address: 0x2000, old: [...], new: [9, 9]}]
interpreter.undo()                       // puts every one of them back
```

Everything written between `beginPoke()` and `endPoke()` is one step, whatever
it wrote; a poke that wrote nothing, or only values that were already there,
records none and `endPoke()` answers `false`. Outside a transaction the same
setters are direct and record nothing, which is what preset starting values
need. `beginPoke()` throws when a poke is already open and when an instruction
is executing — an interrupt waiting for its answer included — and `endPoke()`
throws when no poke is open. Undoing a poke puts back every value it wrote and
touches nothing else: not the program counter, not the flags, not the call
stack.

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

## Migrating to 3.0.0

3.0.0 makes every `trap #15` task follow EASy68K and types every error.

| 2.5.0 | 3.0.0 |
|---|---|
| `DisplayNumber`'s value a number | the text, `'-5'` |
| `DisplayNumberInBase` → `{value, base}` | the text in upper case, `'FF'` |
| `DisplaySignedNumberInField` → `{value, width}` | the text padded to a signed width, `'-5    '` for -6 |
| `DisplayStringAndNumber` → `{string, number}` | one text |
| `{type: 'ReadNumber', value: 42}`, and `DisplayStringAndReadNumber` with a number | the line typed, `'42'`, read by `atoi` |
| task 2 stored 80 UTF-8 bytes, its count in D1.W | 79 characters in Windows-1252, the count in D1.L |
| task 5 stored the key's code as given, so Enter sent as `'\n'` was `$0A` | Enter is `$0D`, whether it is sent as `'\r'` or `'\n'` |
| strings read and written as UTF-8 (an `é` failed at run time) | Windows-1252, in the tasks and in the assembler: `€` is `$80` |
| tasks 0 and 1 refused a D1.W above 255; task 1 displayed past a NUL | at most 255 characters, stopping at a NUL |
| tasks 12, 16, 50 to 59 and 70 to 77 failed as unknown | the settings are carried out, the files and the sound are interrupts |
| an answer of most other types was taken, a malformed one panicked | only the pending interrupt's own answer, or `Terminate` |
| `answerInterrupt` threw a string | a `RuntimeError` object: `NoPendingInterrupt`, `InvalidAnswer`, `OutOfBounds` |
| answering `Terminate` left the program running | it ends the program |
| a task error was `{type: 'Raw', value: 'Unknown interrupt: 30'}` and left the program running | `UnsupportedTrapTask` or `InvalidTrapArgument`, and the program ends |
| a division by zero or an address error left the program running | every runtime error ends the program, `TerminatedWithException` |
| a register number past 7 panicked | `InvalidArgument` is thrown |
| `runWithBreakpoints` threw a string for a malformed list | `InvalidArgument` |
| no way to ask why a program ended, or how input is shown | `getTermination()`, `getInputSettings()`, and the `SetInputSettings` mutation undo replays |

The diagnostic `character_above_latin1` now means a character Windows-1252 has
no byte for, and a typographic quote outside a string is `unexpected_character`.
A `switch` over `Interrupt`, `InterruptResult`, `RuntimeError` or
`MutationOperation` needs the new types.

## Migrating from 1.4.2

| 1.4.2 | 2.0 |
|---|---|
| `new S68k(code)`, `s68k.semanticCheck()` | `S68k.assemble(code).diagnostics` |
| `S68k.compile(code)` → `{ok, errors, interpreter}` | `S68k.assemble(code)` → `{diagnostics, program?}` then `new Interpreter(program)` |
| `SemanticError` class, `getMessage()`, `getLineIndex()` | a plain `Diagnostic` object with `code`, `message`, `hint`, `location` |
| `S68k.lex`, `S68k.lexOne`, `LexedLine`, `ParsedLine` | `S68k.parseLine(text)` |
| `getCurrentLineIndex(): number` | `getCurrentLocation(): Location \| null` |
| `runWithBreakpoints(Uint32Array)` (line indexes) | `runWithBreakpoints([{file, line}])` |
| `getInstructionAt(address)` → an instruction with a parsed line | → `{address, size, location, includeChain, source}` |
| `stepGetStatus()` | `step()`, which answers the status (`stepGetStatus` still works) |

Diagnostics are reported where 1.4.2 was silent, and a few programs that
assembled by accident no longer do; `equ` names a value rather than substituting
text.

## Supported instructions
| Type                   | Instructions                                                                                                                                                                                                      |
|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Arithmetic             | add, sub, suba, adda, divs, divu, muls, mulu, addq, subq, addi, subi, addx, subx, negx                                                                                                                            |
| Comparison             | tst, cmp, cmpi, cmpa, cmpm                                                                                                                                                                                        |
| Branching and jumping  | bcc, bcs, beq, bne, blt, ble, bgt, bge, bls, bhi, bpl, bmi, blo, bhs, bvc, bvs, bsr, bra, jmp, jsr, rts, dbcc, dbcs, dbeq, dbne, dbge, dbgt, dble, dbls, dblt, dbhi, dbmi, dbpl, dbvc, dbvs, dbf, dbt, dbhs, dblo, dbra |
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo, and `move`/`andi`/`ori`/`eori` with `sr` or `ccr` as an operand                                                           |
| Bitwise                | not, or, ori, and, andi, eor, eori, lsl, lsr, asr, asl, rol, ror, roxl, roxr, btst, bclr, bchg, bset                                                                                                              |
| Binary coded decimal   | abcd, sbcd, nbcd                                                                                                                                                                                                 |
| Other                  | clr, exg, neg, ext, extb, swap, move, link, unlk, lea, pea, moveq, movea, movem, movep, tas, nop                                                                                                                  |
| Exceptions             | chk, trapv, illegal, and rtr, which returns and restores the condition codes                                                                                                                                      |
| Interrupt              | trap #15, with the I/O, mouse and graphics tasks of EASy68K                                                                                                                                                      |

## Supported directives
`org`, `equ`, `set`, `dc`, `dcb`, `ds`, `end`, `reg`, `fail`, `simhalt`,
`section`, `offset`, `include` and `incbin`, and `opt`, `list`, `nolist` and
`page`, which are accepted and ignored. Without `end`, the entry point is a
label named `START`, and failing that the first instruction.

`memory`, the macro and conditional-assembly directives and EASy68K's structured
control are recognised and refused with a diagnostic naming the feature.

## Known limitations
1. Characters are one byte, read and written as Windows-1252, as in EASy68K; a source character with no byte of its own is an assembly error.
2. Every instruction is four bytes wide whatever it encodes to on a real 68000, so an address computed from instruction sizes will not match the hardware.
3. The program runs as supervisor, always.

# How to build
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the whole package by running `npm run build-all` in the `ts-lib` folder of the project: it builds the wasm package into `ts-lib/src/pkg` and then the TypeScript library into `ts-lib/dist`. `npm test` runs a smoke test over the built `dist/`.
