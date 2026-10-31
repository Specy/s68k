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

`simhalt`, assembled as EASy68K's `$FFFF $FFFF`, makes an interpreter return
`InterpreterStatus.Paused` without terminating it. Calling `step()`, `run()`, `runWithLimit()` or
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

The result also says the processor the source is written for, `processor`
(`'68000'`, `'68010'` or `'68020'`, from its `opt p=` line, `'68000'` when it
has none) and `bitFields` (`opt bit`, or `opt p=68020`), whether or not it
built a program, so an editor can follow the processor while the code has an
error. With a program they are its own `getProcessor()` and `hasBitFields()`.

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
`DirectX` forms, `ControlSound` and `ControlSoundDirectX`. A host answers with
a `SoundOutcome`, what its player did, and the interpreter writes D0.W as
EASy68K does; `LoadSound` has no result. A host with nothing to play them on
answers `Terminate`.

```ts
switch (interrupt.type) {
    case 'PlaySound':               // task 70: the path
    case 'PlayLoadedSound':         // task 72: the index
    case 'PlaySoundDirectX':        // task 73: the path
    case 'LoadSoundDirectX':        // task 74: {path, index}
    case 'PlayLoadedSoundDirectX':  // task 75: the index
    case 'ControlSound':            // task 76: {index, control}
    case 'ControlSoundDirectX':     // task 77: {index, control}
        return {type: interrupt.type, value: 'Done'}   // 'Busy', 'Missing', 'Unplayable'
    case 'LoadSound':               // task 71: {path, index}
        return {type: 'LoadSound'}
}
```

| Outcome | It means | 70, 72 | 73 to 77 |
|---|---|---|---|
| `'Done'` | the task did what it asked: a sound started, a load loaded, a stop stopped | 1 | 1 |
| `'Busy'` | the standard player was still playing an earlier sound; the DirectX player mixes, so it is never busy | 0 | 0 |
| `'Missing'` | no file at the path, or no sound in the slot | D0 left as it was | 0 |
| `'Unplayable'` | a file that is there but cannot be played | 0 | 0 |

#### Errors, and why a program ended

Every method throws a `RuntimeError` object, `{type, value}`, and none of them
panics, so the interpreter stays usable after a bad call. A runtime error the
68000 has an exception for goes through its vector into the program's handler
(below, "Exceptions"), and ends the program only when the vector holds none:
the status is `TerminatedWithException`, the error is thrown, and
`getTermination()` answers it as the cause, with the vector. A `trap #15` task
adds two errors of its own, which end the program:

```ts
{type: 'UnsupportedTrapTask', value: {task: 10}}            // the printer, the network; a number that is no task goes through vector 47 first
{type: 'InvalidTrapArgument', value: {task: 15, reason: 'D2.B is 37, and a base is 2 to 36'}}

interpreter.getTermination()
// null while it runs, then {type: 'TerminateTask'} (task 9),
// {type: 'EndOfProgram', value: {address: 0x1012}} (it ran into memory it never placed),
// {type: 'TerminatedByHost'} (a Terminate answer),
// {type: 'Exception', value: {cause: {type: 'DivisionByZero'}, vector: 5}} (vector 5 holds no handler),
// {type: 'Exception', value: {cause: {type: 'UnsupportedTrapTask', value: {task: 10}}}} (no vector could take it)
// {type: 'Exception', value: {cause: {type: 'Irq', value: {level: 2}}, vector: 26}} (an IRQ whose autovector holds no handler)
// or {type: 'Halted', value: {cause, vector}} (a frame pushed on an odd supervisor stack pointer
// or into memory the memory map refuses, or a vector read from an invalid range)
```

A program ends where it runs into `$FFFF $FFFF` it never placed: EASy68K's
SIMHALT, outside the bytes of its code and data. Memory starts as `$FF`, so a
program that does not end itself ends one past its last instruction when
nothing follows it, and `EndOfProgram` says where. `$FFFF $FFFF` inside the
program's own bytes is a `simhalt`, which pauses. Anything else runs as what it
decodes to: a program that runs on into its data, or jumps there, runs the
data, a word that is no instruction is `IllegalInstruction`, and an instruction
fetched from an odd address, or a jump to one, is an `AddressError`.
`hasReachedBottom()` is true when the next step ends the program.

Undoing the step that ended the program brings it back, running. The tasks s68k
does not carry out are the printer (10), the text window's font and contents
(21, 22, 25), the hardware window (32 with 0 to 3 or 7 in D1.B: there is no
hardware window), the serial ports (40 to 43) and the network (100 to 107).

Where this departs from EASy68K:

- a base outside 2 to 36 for task 15, a D1.B other than 0 to 3 for task 16 and
  a D1.L other than 0 or 1 for task 58 end the program, where EASy68K does
  nothing;
- a D1.W of `$8000` or more for tasks 0 and 1 displays up to 255 characters, as
  above 255;
- overflow in `atoi` wraps, Borland's own being unverified;
- the characters typed after the 79th of a line are dropped;
- a read of no bytes is 2, where EASy68K says 1 once an earlier read reached the
  end of the file;
- a buffer whose count passes 4 GB is 2, where EASy68K's check wraps round, and
  task 58 checks all 256 bytes it writes.

### Debugging

```ts
interpreter.setBreakpoints([{file: 'lib/io.x68', line: 12}])  // kept for every run until replaced
interpreter.runWithBreakpoints()
interpreter.getCurrentLocation()      // {file, line, column, endColumn} | null
interpreter.getNextInstruction()      // {address, size, bytes, location, includeChain, source, assembledAs?} | null
interpreter.disassembleAt(0x1008)     // {address, size, bytes, text: 'ADD.W D1,D2'}, what memory holds now
interpreter.getCallStack()            // one frame per subroutine entered and per exception taken
interpreter.canUndo() && interpreter.undo()
```

A breakpoint is a line of a file, any file of the project: one on a comment, a
directive or a label alone stops nothing, and one on a line of a file included
twice stops at both copies. Undo needs a history, which `new Interpreter(program)` keeps by
default (`{keep_history: true, history_size: 100}`). A program that has not run yet has nothing
to undo: `canUndo()` is false until the first step, and undoing every step leaves the program
counter at the Entry point.

`program.getMemoryMap()` returns the ranges the program's `memory` lines
declare, at most one of each kind, in EASy68K's order: `[{kind: 'rom', start:
0x8000, end: 0x8fff, location}]`, `kind` being `'rom'`, `'read'`, `'protected'`
or `'invalid'` and both addresses in 24 bits, `end` included. The interpreter
enforces them as EASy68K does: a write to ROM is ignored, a write to a read
only range is a bus error, and so is any access to a protected range from user
mode and any access to an invalid one, an instruction's fetch included. A bus
error is vector 2, with the 14-byte frame, and with no handler ends the program
with `{type: 'BusError', value: {address, size, access, range}}`, `range`
`'read'`, `'protected'` or `'invalid'`. The `trap #15` tasks check their
buffers as EASy68K does. A poke, a testcase's starting memory, the host's reads
and undo go past the map.

`program.toSRecord()` returns the program as the `.S68` file EASy68K's
assembler writes, which EASy68K's simulator loads and runs: the `68KPROG`
header, the code and data in `S1`/`S2` records of up to 32 bytes, the memory map
as EASy68K's extra `S0` records and the entry point in an `S8` record, each
record ending with CR LF.

`getInstructionAt`, `getNextInstruction` and `getLastInstruction` answer the
line the program laid out at an address, as it was assembled, and
`assembledAs` says what it assembled as when the assembler chose something the
line did not write: another mnemonic (`add.l #1,d0` is `ADDQ.L #1,D0`, `move.l
#5,d0` is `MOVEQ #5,D0`, `move.l d0,a0` is `MOVEA.L D0,A0`), a branch size
(`bra loop` is `BRA.S loop`) or an address in a word (`lea $1000,a0` is `LEA
$1000.W,A0`). It is absent when the line is what it says; an alias, `dbra` for
`DBF`, is no other mnemonic. The text is EASy68K's, with the program's labels.

### Code is memory

The interpreter runs what memory holds, through a cache of the instructions it
has decoded, filled from the assembler's own when the interpreter is made.
Every write passes it, so a program can change its own code and the change
runs; `writeMemoryBytes` writes code like data, and undo writes the old bytes
back. The source does not move: a changed line keeps its location, its
breakpoint and its `getInstructionAt`, and `disassembleAt` reads what runs
there now.

A write of the program's own into an instruction, a store or a `trap #15`
task's answer, is a code write. The step says what it overwrote first, and
`getCodeWrite()` says it of the instruction that has just run; a poke, a
testcase's memory and undo never make one:

```ts
interpreter.getUndoHistory(1)[0].wroteCode  // {address: 0x1008, instructionIndex: 3}, or absent
interpreter.getCodeWrite()                  // the same, or null
interpreter.runWithBreakpoints(limit, {skipBreakpointAtPc: false, stopOnCodeWrite: true})
// InterpreterStatus.CodeWrite after the instruction that wrote into code; the next run goes on
```

`instructionIndex` indexes `getInstructionAddresses()`, and is absent for code
no line was laid out at, such as data the program jumped into. A stop for a code
write is like one for a watched write: the instruction is finished, the
program counter is on the next one, and any run or step goes on. A watched
write by the same instruction stops the run first, as `WatchedWrite`. `run()`,
`runWithLimit()`, `step()` and an answer to an interrupt record the code write
and do not stop.

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

### Watching a range of memory

A device mapped into memory, such as a sound chip's registers, has to see every
store into its registers as it happens and in order: a bit turned off and on
again is two writes, and a write that starts a copy has to be handled before the
program changes what it copies. `watchMemory(start, end)` watches the addresses
from `start` up to `end`, `end` not included. Each instruction that stores into
them ends the run or step that ran it with `InterpreterStatus.WatchedWrite`, and
`takeWatchedWrites()` answers what was stored:

```ts
interpreter.watchMemory(0xE00100, 0xE00300)
let status = interpreter.run()
while (status === InterpreterStatus.WatchedWrite) {
    for (const {address, size, value} of interpreter.takeWatchedWrites()) {
        device.write(address, size, value)   // {address: 0xE00110, size: 1, value: 1}
    }
    status = interpreter.run()
}
```

- A store counts when any of its bytes is in the range, so one that straddles
  an edge counts, at the address it began at. Each register of a `movem` is a
  write of its own, and so is the store of `addq`, `bset` or `clr` to memory and
  the long a `bsr`, `jsr` or `pea` pushes.
- A run of bytes written for the program, the answer to a read task, or by
  `writeMemoryBytes`, in a poke or not, is recorded a byte at a time, only the
  bytes in the range. An answer that writes into the range leaves the status at
  `WatchedWrite` as well; a write of the host's stops nothing.
- The instruction has finished and is in the history like any other. The
  program counter is on the next one, and any run or step goes on from it, as
  after `simhalt`; a store by the last instruction stops there, and the next call
  ends the program.
- An error comes before the stop: an instruction that fails after it stored
  ends the program with its error, and what it stored is in the log. Every
  other stop comes after it. A breakpoint on the next instruction is reported
  by the next call, made with `skipBreakpointAtPc: false` as after an
  interrupt, and a watched write on the last instruction a `limit` allows
  answers `WatchedWrite`, as any stop there does, rather than throwing
  `ExecutionLimit`.
- Writes wait in order until they are taken. `undo()` empties the log, so a
  device reads the range back after an undo; `unwatchMemory()` and another
  `watchMemory()` forget what was not taken. The program's initial contents are
  not stores, and memory starts filled with `$FF`.
- An interpreter that watches nothing pays one comparison a store.
- A run that stops at every write says nothing of how many instructions it ran
  before the stop. `getExecutedInstructions()` counts every instruction run, so
  a host that runs in slices takes the difference of two readings, and keeps to
  its own limit across the stops.

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

### The instruction table

`S68k.instructionTable()` lists every mnemonic the assembler accepts with the
forms it may be written in: per form, the addressing modes of each operand
position, by the names a diagnostic uses, the sizes, written as `parseLine`
writes one, and whether the 68000 runs the form only in supervisor mode.

```ts
S68k.instructionTable().find(({mnemonic}) => mnemonic === 'andi')
// {
//   mnemonic: "andi",
//   forms: [
//     {operands: [["Im"], ["Dn", "(An)", "(An)+", "-(An)", "d(An)", "d(An,Xn)", "Ea/<label>"]], sizes: ["byte", "word", "long"], privileged: false, processor: "68000"},
//     {operands: [["Im"], ["ccr"]], sizes: ["byte"], privileged: false, processor: "68000"},
//     {operands: [["Im"], ["sr"]], sizes: ["word"], privileged: true, privilegedFrom: "68000", processor: "68000"}
//   ]
// }
```

The privileged forms are `move <ea>,sr`, `andi`, `ori` and `eori` into `sr`,
`rte`, `stop`, `reset`, and `move usp,An` and `move An,usp`, and the 68010's
`movec` and `moves`. They assemble like any other, and raise a privilege
violation when they run in user mode. `privilegedFrom` names the first
processor that runs a form only in supervisor mode, and is absent when none
does: it is `'68010'` for `move sr,<ea>`, which a 68000 runs in user mode and
the 68010 made privileged, where `privileged`, which is about the form's own
first processor, is `false`. `processor` is the first processor that has the
form: `'68010'` for `move ccr,<ea>`, `rtd`, `bkpt`, `movec` and `moves`, whose
control registers are the operand mode `Rc`.

### Exceptions

Every exception a 68000 takes is taken through the vector table at `$000` to
`$3FF`: the bus error (2) of the memory map, the address error (3), an illegal instruction (4), a division by zero
(5), `chk` (6), `trapv` (7), a privilege violation (8), trace (9), line A and
line F (10, 11), `trap #0` to `#14` (32 to 46) and a `trap #15` task that is
none of EASy68K's (47). The processor pushes the status register and the
program counter on the supervisor stack, 14 bytes for a bus or address error, sets S,
clears T and jumps to the handler; `rte` returns. A vector holding an odd
address has no handler, and the program ends as it would have before, the
vector named in the termination: memory starts as `$FF`, so every vector is
unset until the program writes it.

```ts
interpreter.getSystem()               // {sr: 0x2000, usp: 0x00ff0000, ssp: 0x01000000} at the start
interpreter.getSp()                   // A7: the SSP in supervisor mode, the USP in user mode
interpreter.beginPoke()
interpreter.setSr(0x0000)             // user mode: A7 is the USP now
interpreter.setUsp(0x8000)
interpreter.setSsp(0x00f00000)
interpreter.endPoke()                 // one step, which one undo takes back, the stack swap included
interpreter.getCallStack()[0]         // {kind: 'exception', vector: 5, address: handler, source_address: the stacked PC, ...}
```

In a program for the 68010 (`opt p=68010`) the System file has the control
registers too, and the frames are the 68010's:

```ts
interpreter.getSystem()               // {sr: 0x2000, usp: 0x00ff0000, ssp: 0x01000000, vbr: 0, sfc: 0, dfc: 0}
interpreter.beginPoke()
interpreter.setVbr(0x8000)            // the vector table: every exception reads its vector at VBR + 4 * vector
interpreter.setSfc(1)                 // three bits; s68k has one address space, so `moves` reaches the same memory
interpreter.setDfc(5)
interpreter.endPoke()                 // one step, which one undo takes back; on a 68000 these throw InvalidArgument
```

- **Format `$0`**, every exception but the bus and address errors: eight
  bytes, the status register, the program counter, and the format/vector-offset
  word, `0x0000` plus four times the vector. A handler that pops the frame by
  hand pops eight bytes, not six.
- **Format `$8`**, the bus and address errors: 58 bytes. Above the format word
  `0x8000` plus the offset, the special status word (bit 13 an instruction
  fetch, bit 12 a data read, bit 9 a byte, bit 8 a read, and the function code
  in bits 2 to 0), the fault address, then the three buffers and the sixteen
  internal words as zeros, the version among them, and three reserved words
  that are not written. The program counter is the faulting instruction's, and
  `rte` runs it again.
- **`rte`** pops the frame its format word says, and a format other than `$0`
  and `$8`, or a format `$8` frame whose version is not 0, is the format error,
  vector 14, `{type: 'FormatError', value: {format}}` when it has no handler.
- `movec` journals `WriteControlRegister {register: 'vbr' | 'sfc' | 'dfc', old,
  new}`, and `movec An,usp` the stack pointers, as `move An,usp` does.
- `move sr,<ea>` is privileged.

In a program for the 68020 (`opt p=68020`) the System file has the 68010's and
the 68020's: its interrupt and master stack pointers, which M chooses between
in supervisor mode, and its cache registers, with no cache behind them. `ssp`
is the 68020's ISP, which `isp` names too:

```ts
interpreter.getSystem()               // {sr: 0x2000, usp: 0x00ff0000, ssp: 0x01000000, vbr: 0, sfc: 0, dfc: 0,
                                      //  isp: 0x01000000, msp: 0, cacr: 0, caar: 0}
interpreter.beginPoke()
interpreter.setMsp(0x8000)            // A7 in supervisor mode while M is set
interpreter.setIsp(0x00f00000)        // the SSP: A7 in supervisor mode while M is clear
interpreter.setCacr(0xff)             // keeps E and F, bits 0 and 1: 3
interpreter.setCaar(0x1234)
interpreter.setSr(0x3000)             // M set: A7 is the MSP
interpreter.endPoke()                 // one step; before the 68020 these throw InvalidArgument
```

- **The status register** reads back `0xF71F` for `0xFFFF`: T1 (bit 15)
  traces every instruction, and T0 (bit 14), with T1 clear, only a branch
  taken, a jump, a return and a write of SR or of CCR by `andi`, `ori` or
  `eori`; M (bit 12) chooses the stack pointer.
- **Frames.** Format `$0`, eight bytes, and format `$2`, twelve, with the
  instruction's address at `+8`, for `chk`, `chk2`, `trapcc`, `trapv`, a
  division by zero and trace. A bus or address error pushes format `$B`, 92
  bytes, or `$A`, 32, for a bus error on fetching an instruction none of which
  ran: the special status word at `+$0A` and the fault address at `+$10`,
  every internal word 0, and `rte` runs the instruction again. An interrupt
  taken with M set pushes its frame on the MSP and the throwaway frame, format
  `$1`, on the ISP; `rte` pops both. Any other format is the format error,
  with no change of flags.
- **Odd addresses.** A word or a long at an odd address is read and written;
  only an instruction fetched from one is an address error.
- `movec` journals `WriteControlRegister` with `cacr` and `caar` too, and a
  `movec` of `msp` or `isp` the stack pointers.

A call frame is `kind: 'call'` and has no `vector`. An exception frame says
what raised it, `cause`, the `type` of its error, which tells the two causes of
a vector apart: `'Chk2OutOfBounds'` from `'ChkOutOfBounds'` through vector 6
and `'ConditionalTrap'` (`trapcc`) from `'OverflowException'` (`trapv`) through
vector 7; and `vbr`, the vector base register it was taken with, so its vector
was read at `vbr + 4 * vector` even after `movec` has moved the table. The
history journals an exception taken as
`PushException {vector, from, handler, cause, vbr}`, `rte` as
`PopException {to, from}`, and every change of S, or of the 68020's M,
`move An,usp`, `movec` and a poke of a stack pointer as
`WriteStackPointers {old: {usp, ssp, msp}, new: {usp, ssp, msp}, a7}`, `msp`
being 0 before the 68020, and `a7` which of the three A7 is after the change,
`'usp'`, `'ssp'` or `'msp'`: an `rte` through the 68020's throwaway frame swaps
twice, to the master stack and then to the stack of the frame there, and says
which each swap made it.

### Interrupts

The 68000's interrupt requests, IRQs, at levels 1 to 7, all autovectored: a
request is latched until it is taken, between two instructions, the highest
pending level above the status register's mask first and level 7 whatever the
mask, through vector 24 plus the level (`$64` to `$7C`). The mask rises to the
level, and `rte` puts it back. Taking one is a step of its own, `kind: 'irq'`,
with no `location`; its `PushException` names the vector and its
`SetMachineState` the request it took. An autovector with no handler ends the
program with `{type: 'Irq', value: {level}}` as the cause.

The program sets up its sources with `trap #15`: task 32 with 6 in D1.B sets a
timer per level (D2.B `$81` to `$87` and D3.L milliseconds, 1 to 7 turning
one off, 0 all of them), task 60 the mouse's levels and task 62 the keyboard's
(D1 bits 15 to 8 the level, bits 0 to 2 the events). The host runs the timers
on the program's time and reports what happens:

```ts
interpreter.getMachineState()
// {pendingIrqs: [], stopped: true, autoIrqs: [{level: 2, intervalMs: 500}],
//  keyboardLevels: {keyDown: 5, keyUp: 0}, mouseLevels: {down: 0, up: 0, move: 0}}
interpreter.raiseIrq(2)               // a timer fired: level 2 is pending
interpreter.deviceEvent('keyDown')    // 5: the level task 62 set for it, latched; null when none is set
```

`stop #n` loads the status register and waits: the status
`InterpreterStatus.Stopped`, with `step()` and every run answering it at once
and running nothing. A request that can wake the processor makes the status
`Running` at once, and the next step takes it; `getMachineState().stopped` is
true until then. A request the program has not taken yet is the host's input:
undoing the step that took one makes it pending again, but undoing an earlier
step leaves a later request pending. `reset` is privileged and changes nothing.

A run takes an IRQ that is ready before it looks at the breakpoint on the
program counter, as the handler's instruction is the next to run, so
continuing from a breakpoint with a request pending runs the handler and stops
at that breakpoint again.

### Cycles

The interpreter counts the clock cycles a 68000 takes: each instruction's own
time from the 68000 User's Manual, with Yacht's corrections, checked against
the conformance suite's length for every test, the data included (a multiply by
the bits of its source, a division by its loop, a shift by its count, a branch
taken or not, `movem` by its registers); each exception's, which is the step's
whole time (34 for `trap #0` to `#14`, an illegal instruction or a privilege
violation, 38 and the source's for a division by zero, an address error 58
after what the instruction had done); 34 more for a trace exception; and 44
for an IRQ. `trap #15`'s tasks take none, as in EASy68K, `stop` takes 4 and
`reset` 132. There are no wait states. A program for the 68010 counts the
68010's own time, section 9 of the manual and Moira's where the manual gives a
maximum (an exception 38, an IRQ 46, `rte` 24, `mulu` 40), with its loop mode:
a `DBcc` that branches back by −4 to a one-word instruction the 68010 can loop
makes the loop's next turns fetch nothing, `move.w (a0)+,(a1)+` taking 8 and
`dbra` 6 where they took 12 and 10, which the history journals as
`SetLoopMode {old, new}`.

A program for the 68020 counts an estimate: the cache case of the M68020
User's Manual's timing tables, the instruction in the cache and no overlap
with the instructions around it. The manual gives a best, a cache and a worst
case for every instruction and says that exact times depend on the cache and
on that overlap, so no exact count exists; the cache case is the middle one,
and Musashi's. Its data counts only where the manual's times depend on it (a
branch taken, a `DBcc` that runs out, a bit field in five bytes, `cas` that
writes, the frame `rte` pops), so a shift by any count and a multiply by any
operand take the same; an exception 20, `trapcc` and `trapv` 25, an IRQ 26,
or 41 with the throwaway frame of the master state. The program says which
kind of count it gets: `cyclesExact` in `getInfo()`, or `hasExactCycles()`,
is `true` for the 68000 and the 68010 and `false` for the 68020, whose count a
host shows as an estimate. It is `false` too for a program for the 68000 or the
68010 under `opt bit` that has a bit-field instruction: its bit fields take
EASy68K's time, the 68020's cache case, which no 68000 or 68010 has, and every
other instruction its exact time.

```ts
interpreter.getCycles()               // 1234n: a bigint, since the start or task 30
interpreter.getUndoHistory(1)[0].cycles // 12: what the newest step took, 0 for a poke
program.hasExactCycles()              // false for a program for the 68020: an estimate
```

Task 30 clears the counter and task 31 reads it into D1.L, 0 once it is past
32 bits, as in EASy68K. Undo takes a step's cycles back, so the counter is
always what the steps in force took. Where EASy68K counts differently, the
68000 wins: a `trap` is 34 here and 38 there, a division by zero 38 and the
source's effective address and not 42, `chk` 38 or 40 and the source's and not
44, an IRQ 44 once and not 34 and 44 more when requested, an address error
what the instruction had done and 58 and not 50, and `tst` of memory 4 and the
effective address and not 4.

### Memory

`Program` and `Interpreter` hold memory on the WebAssembly side, which garbage
collection cannot reach. Call `dispose()` on both when a program is done with.
`S68k.assemble` frees the assembly itself when the source did not build one, so
live checking on every keystroke leaks nothing.

## Migrating: what the history says about exceptions and stacks

The history and the call stack say what a host used to work out, at no cost to
an interpreter that keeps no history:

| before | after |
|---|---|
| `PushException {vector, from, handler}` | and `cause`, the `type` of the error that raised it, and `vbr`, the vector base register it was taken with |
| an exception's frame of `getCallStack()` `{kind: 'exception', vector, ...}` | and the same `cause` and `vbr` |
| `WriteStackPointers {old, new}`, which of them A7 became left to `new_sr` | and `a7`, `'usp'`, `'ssp'` or `'msp'`, after each change, in order within a step |

## Migrating: the 68020's cycles

The 68020's cycles (above, "Cycles") change what a program for the 68020
counts, and add one field a host reads:

| before | after |
|---|---|
| a 68020 program counted as a 68010: its own instructions and modes a placeholder, an exception 38, an IRQ 46 | the manual's cache case, an estimate: `moveq` 2, `mulu.w` 27, a shift 4 to 12 by any count, an exception 20, an IRQ 26 |
| | `cyclesExact` in `ProgramInfo`, and `Program.hasExactCycles()`, `false` for the 68020 and for a program under `opt bit` with a bit-field instruction |

## Migrating: the 68020's supervisor model

The 68020's supervisor model (the README's "The 68020's supervisor model")
changes what a program for the 68020 does, and two shapes every host may read:

| before | after |
|---|---|
| a 68020 program took its exceptions with the 68010's frames, `$0` and `$8` | the 68020's: `$0`, `$2` for `chk`, `chk2`, `trapcc`, `trapv`, a division by zero and trace, `$A` and `$B` for the bus and address errors, and `$1`, the throwaway frame of an interrupt with M set; `rte` takes those and raises the format error on `$8` |
| a 68020 program's word or long at an odd address was an address error | read and written; only a fetch from an odd address is an address error |
| a 68020 program's status register read `0xA71F` at most | `0xF71F`: T0 and M |
| `getSystem()` of a 68020 program as a 68010's | and `isp`, `msp`, `cacr` and `caar`; `setIsp`, `setMsp`, `setCacr`, `setCaar` |
| `WriteStackPointers {old: {usp, ssp}, new: {usp, ssp}}` | `{usp, ssp, msp}` on every processor, `msp` 0 before the 68020 |
| `WriteControlRegister`'s `register` was `'vbr'`, `'sfc'` or `'dfc'` | and `'cacr'` or `'caar'` |
| `cacr`, `caar`, `msp` and `isp` were names a symbol could take | reserved in every program, as `vbr` is; `movec` takes them under `opt p=68020` |
| the hint of `(d3)` in a 68000 program named `(a0)` | it names `movea.w d3,a3` and `(a3)` |

## Migrating: the 68020's instructions

The 68020's integer instructions (the README's "The 68020's instructions")
change what a program for the 68020, or a 68000 one with `opt bit`, can
write, and what a host may be told:

| before | after |
|---|---|
| `chk2`, `cmp2`, `cas`, `cas2`, `pack`, `unpk`, `divsl`, `divul`, `trapcc` and the bit fields refused, `unimplemented_in_dialect`; `muls.l`, `chk.l`, `tst a0` and the like refused under `opt p=68020` | assembled under `opt p=68020`, and the bit fields under `opt bit`; `needs_a_later_processor` on an earlier processor, a bit field's second fix writing `opt bit` |
| the code `unimplemented_in_dialect` | gone: no instruction is still to come |
| | the causes `ConditionalTrap`, vector 7, and `Chk2OutOfBounds {value, lower, upper}`, vector 6 |
| `link.w` refused on every processor, `invalid_size` | assembled under `opt p=68020` as `link`; `needs_a_later_processor` before it, as EASy68K refuses it, its second fix writing `link` without the size |
| `(za0)`, `(zd3)` and `(zpc)` read as the 68020's suppressed registers in every program, so a 68000 program's Symbol of that name was refused there | a Symbol of the name keeps its meaning wherever it can stand, `(za0)` its address; a 68020 program is warned, the new code `symbol_named_like_a_register` |
| | `parseLine`'s modes `register_pair`, `indirect_pair` and `bit_field`; `instructionTable`'s modes `Dx:Dy`, `(Rx):(Ry)` and `{offset:width}` |

## Migrating past 4.0.0: breakpoints

| Before | Now |
| --- | --- |
| `runWithBreakpoints(breakpoints, limit, options)` looked every breakpoint's line up in the whole program on each call | `setBreakpoints(breakpoints)` looks them up once and keeps them; `runWithBreakpoints(limit, options)` stops at the ones set, none until it is called |
| a malformed breakpoint list made `runWithBreakpoints` throw an `InvalidArgument` | `setBreakpoints` throws it |
| `takeWatchedWrites()` copied an empty array across the boundary when nothing had been stored | it answers `[]` without one |

## Migrating past 4.0.0: the 68010

The release with the complete 68010 dialect (the README's "Processor
dialects") changes only what a program for the 68010 or the 68020 does, and
what the instruction table and the history can say:

| before | after |
|---|---|
| `rtd`, `bkpt`, `movec` and `moves` refused, `unimplemented_in_dialect` | assembled under `opt p=68010` and `p=68020`, `needs_a_later_processor` on a 68000 |
| the 68010's frames were the 68000's | format `$0`, eight bytes, and format `$8`, 58, for the bus and address errors; `rte` checks the format, and the cause `FormatError {format}`, vector 14 |
| `getSystem()` `{sr, usp, ssp}` | and `vbr`, `sfc` and `dfc` in a program for the 68010 or the 68020; `setVbr`, `setSfc`, `setDfc` |
| `move sr,<ea>` ran in user mode on every processor | privileged from the 68010 on; the table's `privilegedFrom` |
| `sfc`, `dfc` and `vbr` were names a symbol could take | reserved, as `usp` is |
| the 68010 and the 68020 counted as a 68000 | the 68010's own cycles, with loop mode; the 68020 counts as a 68010 |
| | the mutations `WriteControlRegister` and `SetLoopMode` |

## Migrating past 4.0.0: cycles

The release with cycles (above, "Cycles") counts them, and carries out tasks
30 and 31:

| before | after |
|---|---|
| `UnsupportedTrapTask` for tasks 30 and 31 | 30 clears the counter, 31 reads it into D1.L |
| | `getCycles(): bigint`, and each step's `cycles` |

## Migrating past 4.0.0: interrupts

The release with interrupts (above, "Interrupts") assembles and runs `stop` and
`reset`, and carries out tasks 60, 62 and the parts 4 and 6 of task 32:

| before | after |
|---|---|
| `stop` and `reset` refused by the assembler (`unimplemented_operation`) | assembled and run, privileged |
| `{type: 'Unimplemented', value}` for a decoded `stop` or `reset` | gone: nothing raises it |
| `UnsupportedTrapTask` for tasks 60 and 62, and for task 32 but its 5 | 60 and 62 set the mouse's and keyboard's levels; 32 answers the version for 4 and sets the timers for 6, and only 0 to 3 and 7 are refused |
| `ExecutionStepKind` `'instruction' \| 'poke'` | and `'irq'` |
| | `InterpreterStatus.Stopped`, numbered 7; `getMachineState()`, `raiseIrq(level)`, `deviceEvent(kind)`; the cause `Irq {level}` and the mutation `SetMachineState {old, new}` |
| | the `limit` of `runWithLimit` and `runWithBreakpoints` counts instructions, as `getExecutedInstructions()` does, and an IRQ taken on the way is not one of them |

## Migrating past 4.0.0: exceptions

The release after 4.0.0 takes the 68000's exceptions (above, "Exceptions"). A
program that installs no handler ends as it did, and what reads the end of one
changes:

| 4.0.0 | after |
|---|---|
| `{type: 'Exception', value: error}` | `{type: 'Exception', value: {cause: error, vector?}}`, `vector` the one that held no handler |
| | `{type: 'Halted', value: {cause, vector}}`, a double fault |
| SR `0x2700` at the start, its high byte of no effect | SR `0x2000`, EASy68K's: S, T and the mask are the processor's, and A7 is the SSP or the USP by S |
| one stack pointer, A7 at `0x01000000` | the SSP at `0x01000000`, A7 at the start, and the USP at `0x00FF0000`: `getSystem()`, `setSr`, `setUsp`, `setSsp` |
| `rte`, `move usp` and `trap #0` to `#14` refused by the assembler (`unimplemented_operation`, `unimplemented_addressing_mode`) | assembled and run; the code `unimplemented_addressing_mode` is gone |
| `{type: 'Unimplemented', value: '`rte`'}` and the line A and F words for decoded words | `rte` runs, `LineA`/`LineF` take vectors 10 and 11; `Unimplemented` is `stop` and `reset` |
| `AddressError {address, size}`, the address in 24 bits | `AddressError {address, size, access}`, `access` `'Read'`, `'Write'` or `'Program'` and the address all 32 bits; a jump to an odd address raises it in the jump |
| a call stack of calls | `kind: 'call' \| 'exception'`, and an exception frame's `vector` |
| | the errors `PrivilegeViolation`, `Trace`, `LineA`, `LineF` and `UnhandledTrap {vector}`, and the mutations `WriteStackPointers`, `PushException` and `PopException` |
| the memory map carried and not enforced | enforced: `BusError {address, size, access, range}`, vector 2 |
| | each step's `ran`, what it ran, `{address, size, bytes, text}` |

## Migrating to 4.0.0

4.0.0 answers a sound task with what the player did, and the interpreter writes
D0.W as EASy68K does, and a run whose limit runs out on a stop answers the
stop.

| 3.2.0 | 4.0.0 |
|---|---|
| `{type: 'PlaySound', value: true}`, and the other sound answers but `LoadSound`, with a boolean | a `SoundOutcome`: `'Done'`, `'Busy'`, `'Missing'` or `'Unplayable'` |
| tasks 70 and 72 wrote 0 to D0.W for a missing file or sound | `'Missing'` leaves D0 as it was, as EASy68K does |
| a trap, `simhalt` or the program's end on the last instruction a `runWithLimit` or `runWithBreakpoints` limit allowed threw `ExecutionLimit`, with the interrupt left pending | the status is answered, `Interrupt`, `Paused` or `Terminated`; `ExecutionLimit` only when the program would run on |
| a load or store into code, and a host write there, threw `InstructionAccess` | the code is memory, and the write runs; `InstructionAccess` is gone |
| `{type: 'EndOfProgram'}` when the program counter left the last instruction | `{type: 'EndOfProgram', value: {address}}` when the program runs into `$FFFF $FFFF` it never placed, which is still one past the last instruction when nothing follows it |
| a program that ran off its last instruction into its data ended there | it runs the data, as a 68000 does, so it should end itself (`simhalt`, or task 9) |
| a jump between two instructions threw `OutOfBounds` ("Invalid instruction address") | it runs what is there: an instruction decoded from those bytes, the end in memory the program never placed, or an `AddressError` at an odd address |

It also adds a watched range of memory: `watchMemory`, `unwatchMemory`,
`takeWatchedWrites` and the status `InterpreterStatus.WatchedWrite`, numbered 5
after `Paused`, which only an interpreter that watches a range ever answers, and
`getExecutedInstructions()`, the count of instructions run.

It adds what code in memory needs: `disassembleAt(address)`, `getCodeWrite()`,
`wroteCode` on a step, `assembledAs` on an instruction line, the
`stopOnCodeWrite` option of `runWithBreakpoints` and the status
`InterpreterStatus.CodeWrite`, numbered 6, which only a run asked to stop
answers.

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
| a division by zero or an address error left the program running | every runtime error ends the program, `TerminatedWithException`, unless a vector holds a handler for it |
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
| `getInstructionAt(address)` → an instruction with a parsed line | → `{address, size, bytes, location, includeChain, source}` |
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
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo, `move`/`andi`/`ori`/`eori` with `sr` or `ccr` as an operand, and `move usp,an` and `move an,usp`              |
| Bitwise                | not, or, ori, and, andi, eor, eori, lsl, lsr, asr, asl, rol, ror, roxl, roxr, btst, bclr, bchg, bset                                                                                                              |
| Binary coded decimal   | abcd, sbcd, nbcd                                                                                                                                                                                                 |
| Other                  | clr, exg, neg, ext, extb, swap, move, link, unlk, lea, pea, moveq, movea, movem, movep, tas, nop                                                                                                                  |
| Exceptions             | chk, trapv, illegal, trap #0 to #14, rte, and rtr, which returns and restores the condition codes                                                                                                                  |
| Interrupts and control | stop and reset                                                                                                                                                                                                   |
| Interrupt              | trap #15, with the I/O, mouse, graphics and interrupt-request tasks of EASy68K                                                                                                                                   |

## Supported directives
`org`, `equ`, `set`, `dc`, `dcb`, `ds`, `end`, `reg`, `fail`, `simhalt`,
`section`, `offset`, `include`, `incbin` and `memory`, and `opt`, `list`,
`nolist` and `page`, which are accepted and ignored. Without `end`, the entry point is a
label named `START`, and failing that the first instruction.

The macro and conditional-assembly directives and EASy68K's structured control
are recognised and refused with a diagnostic naming the feature.

## Known limitations
1. Characters are one byte, read and written as Windows-1252, as in EASy68K; a source character with no byte of its own is an assembly error.
2. The 68020's cycle count is an estimate, the manual's cache case (above, "Cycles"); the 68000's and the 68010's are exact, with no wait states.
3. The timers of task 32 are the host's to run; the interpreter keeps their settings and takes the requests the host raises.

# How to build
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the whole package by running `npm run build-all` in the `ts-lib` folder of the project: it builds the wasm package into `ts-lib/src/pkg` and then the TypeScript library into `ts-lib/dist`. `npm test` runs a smoke test over the built `dist/`.

## Memory layout and stack bounds

`Program.getLayoutItems()` returns a `Uint32Array` after assembly, with five
values per item: address, byte length, kind (`0` code, `1` data, `2` reserved),
numbered section, and byte alignment, which is 2 for every instruction. `org` gaps remain gaps; `offset` regions
and zero-length reservations emit no item. `ds` remains reserved even with a fill.
The existing `Program.labels` supplies the symbols, including qualified locals.

`Interpreter.getStackTop()` is the top of the stack A7 is on, and each stack
pointer, the USP and the SSP, keeps its own: they start at EASy68K's stack
pointers, and a change of S swaps them with the stack pointers, so a handler
entered from user mode has the supervisor stack's top, and its `rte` finds the
user stack's where the program left it. An instruction moving A7 by more than
4096 bytes starts a new stack, as does a `move An,usp` or a poke that moves the
other stack pointer so; raising A7 above its top raises the top. Undo restores
both with the step, a poke's included. This internal history field does not
change the serialized public execution history.
