# s68k

s68k is an M68K assembler and interpreter written in Rust. The assembler reports useful errors and warnings to help with learning assembly, and encodes each instruction as EASy68K does, into memory. The emulator runs the assembled program and implements useful debugging features to step, undo, add breakpoints through the program

The library compiles to WebAssembly for use from JavaScript and Node.js. It is
[available on npm](https://www.npmjs.com/package/@specy/s68k).

It is part of a family of JavaScript assembly interpreters and simulators:

- MIPS: [git repo](https://github.com/Specy/mars),  [npm package](https://www.npmjs.com/package/@specy/mips)
- RISC-V: [git repo](https://github.com/Specy/rars), [npm package](https://www.npmjs.com/package/@specy/risc-v)
- X86: [git repo](https://github.com/Specy/x86-js), [npm package](https://www.npmjs.com/package/@specy/x86)
- M68K: [git repo](https://github.com/Specy/s68k), [npm package](https://www.npmjs.com/package/@specy/s68k)
- Z80: [git repo](https://github.com/Specy/trs80), [npm package](https://www.npmjs.com/package/@specy/z80)

## Purpose
s68k is a teaching tool for M68K assembly. Its diagnostics explain invalid
syntax, addressing modes, and directives. It writes a program out as the
S-record file EASy68K's assembler writes, which EASy68K's simulator loads, but
it is not intended to produce binaries for real 68000 systems.

## Architecture

The library has three parts:

- **Assembler** (`src/assembler/`): reads the source files of a project, starting from the entry file, and returns diagnostics and, when there are no errors, a program. It contains a tokenizer, a parser, a symbol table, an expression evaluator, the layout pass, the instruction table, and the analyzer. Assembly reports all of the diagnostics it finds.

- **Program**: the assembler's output, a program containing assembled instructions with their addresses, sizes, encodings, and source locations; the initial memory; the symbols; and the entry point.

- **Interpreter** (`src/interpreter.rs`): runs a program from memory using registers and the status register. It supports stepping, running to the end, undo history, breakpoints, a call stack, and a watched range of memory whose every store ends the run, for a device mapped into memory.

- **Machine** (`src/machine/`): the 68000's encoding. A decoder reads the words of an instruction for a 68000, a 68010 or a 68020, an encoder writes them, a disassembler prints an instruction in EASy68K's syntax, and the decode cache keeps the instructions the interpreter runs.

The assembler chooses each instruction's 68000 encoding as EASy68K does (below) and writes it into the program's memory at its real size, so every address is the one a 68000 and EASy68K's listing give. The interpreter runs those bytes (below, "Code is memory").

## Code is memory

The interpreter runs what memory holds, through a cache of decoded instructions: 4 KB pages of one slot per word, a page made only where code is, filled from the assembler's own instructions when the interpreter is made, so the program's code is never decoded while it runs. Every write passes the cache, the program's, a trap task's, the host's and undo's, and clears the instructions it covers, which are decoded again from their new bytes before they run. So a program can read and change its own code, and the change runs, as on a 68000; undo writes the old bytes back, and the old instruction runs again.

- **A code write** is a write of the program's own, its stores and a trap task's answer, into an instruction it had decoded. Each step says which instruction it overwrote first (`wroteCode` on the step, `getCodeWrite()`), as the instruction's address and the index of its line. A run stops after one when asked to (`runWithBreakpoints(…, {stopOnCodeWrite: true})`, the status `CodeWrite`), and the next run goes on. A Poke, a testcase's starting memory and undo never make one.
- **The source** is read through the program's own map of the lines it laid out, which does not move when bytes change: a changed line keeps its location, its breakpoint and its `getInstructionAt` as assembled, and `disassembleAt(address)` prints what memory holds there now. `getInstructionAt` also says what the line assembled as when the assembler chose something it did not write (`assembledAs`: `add.l #1,d0` is `ADDQ.L #1,D0`, `bra loop` is `BRA.S loop`, `lea $1000,a0` is `LEA $1000.W,A0`).
- **The end.** `$FFFF $FFFF` is EASy68K's SIMHALT wherever it is. Inside the bytes the program placed, its code and its data, it pauses, as `simhalt` does. Anywhere else it ends the program, `EndOfProgram` with the address: memory starts as `$FF`, so that is where a program that does not end itself ends, one past its last instruction when nothing follows it (ADR 0009). Any other bytes run as what they decode to, so a program that runs on into its data, or jumps there, runs the data, and a word that is no instruction is an illegal instruction. An instruction fetched from an odd address is an address error.

## Future work

- Exceptions through the vector table, supervisor and user mode, the memory map's bus errors, interrupts and cycles (the asm-editor's `docs/design/m68k-machine.md`)
- Macros and conditional assembly


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
| Interrupt              | trap #15, with the I/O tasks 0-9, 11-20, 23, 24 and 33 in `d0`, the file tasks 50-59, the mouse task 61, the sound tasks 70-77 and the graphics tasks 80-96 (the `Interrupt` enum of `src/instructions.rs` is the list) |

## Encodings

Where the 68000 has several encodings of a line, s68k picks the one EASy68K picks:

- The first of EASy68K's forms that fits: `cmp #n,dn` is `cmp`, and `cmp #n,(a0)` is `cmpi`; `and #n,dn` and `or #n,dn` are `and` and `or`, and into memory `andi` and `ori`; `eor #n` is `eori`; `add #n,an` and `sub #n,an` are `adda` and `suba`, and `add #n,dn` is `addi`.
- The optimisations EASy68K always makes: `move.l #n,dn` of -128 to 127 is `moveq`, and an `add`, `addi`, `adda`, `sub`, `subi` or `suba` of 1 to 8 is `addq` or `subq`. Writing the immediate `#n.l` keeps the long form, `move.l #1.l,d0`; it is the one size suffix an immediate takes.
- An unsized branch is short when its target is above it and its displacement fits a byte and is not 0, which marks a word branch, and a word otherwise; `.s` and `.b` force a short one, and `.w` and `.l` a word one. An unsized absolute address is `.w` when it is above the line and a word holds it, and `.l` otherwise. "Above" counts a label on the line itself, and a number is always above.
- An immediate is kept at the size the instruction reads it at, so `move.w #-1,d0` holds `$FFFF`, and `exg a0,d0` is `exg d0,a0`.

There are two passes and nothing in between, as in EASy68K: the first pass decides each instruction's size by these rules, and a name defined below the line is always given the long form, whatever its value, so the second pass encodes every instruction at the size it was laid out at. An instruction takes the bytes of its encoding, two to ten, and starts on an even address. A branch whose displacement does not fit is an error, as in EASy68K: a short one out of -128 to 127 or to the very next instruction (a displacement of 0 is the mark of a word branch), and a word one or a `dbcc` out of -32768 to 32767. A PC-relative displacement is measured from its own extension word, which is two bytes past the instruction, and four for `movem` and `btst #n`.

Three lines EASy68K refuses are refused here too, since the 68000 has no encoding for them: an address register added into memory (`add.l a0,(a1)`), a bit number written as an immediate on an immediate (`btst #1,#5`, where `btst d1,#5` is the instruction), and a `link` displacement outside -32768 to 32767.

## The S-record

`Program::to_srecord()`, `program.toSRecord()` in TypeScript, writes the program as the `.S68` file EASy68K's assembler writes, which EASy68K's simulator loads and runs (`src/assembler/srecord.rs`, after EASy68K's `OBJECT.CPP`):

1. the header `S0` record, `68KPROG   20CREATED BY EASY68K`, which is what tells EASy68K's loader the file is its own;
2. the code and the data, every byte the program writes and nothing else, in `S1` records when a record's first address fits 16 bits and `S2` when it fits 24, up to 32 bytes each;
3. one more `S0` record for each range of the memory map, as EASy68K writes them, `      ROM 008000,008FFF`;
4. the `S8` record of the entry point.

Each record ends with CR LF, as EASy68K writes them on Windows. EASy68K breaks its data records where the next item it assembles would not fit, and these break every 32 bytes; both load the same bytes at the same addresses.

## Supported directives
| Directive | What it does |
|---|---|
| `org` | sets the address the following lines are laid out at; the default origin is `$1000` |
| `equ` | names a value, once |
| `set` | names a value that may be set again; each use sees the latest definition above it |
| `dc` | puts values or a string in memory, `.b`, `.w` or `.l` |
| `dcb` | puts a value in memory a given number of times |
| `ds` | reserves room and writes nothing to it |
| `end` | ends the program and, with an operand, sets the entry point |
| `reg` | names a `movem` register list, `AllRegs reg d0-d7/a0-a6`, for `movem.l AllRegs,-(sp)` |
| `fail` | reports the rest of the line as an error of the program's own; the assembly carries on |
| `simhalt` | assembles to `$FFFF $FFFF`, EASy68K's halt, and pauses the run there; the next `run` or `step` resumes at the following instruction |
| `section` | switches between the sixteen location counters, 0 to 15, each going on from where it was left |
| `offset` | opens a region that produces no bytes, where `ds` names the fields of a structure by their offsets; `org *` ends it |
| `include` | assembles another file of the project here, as if its lines had been pasted at this line |
| `incbin` | puts another file's bytes in memory here, as a `dc.b` of the whole file would |
| `memory` | declares a range of EASy68K's memory map, `memory rom $8000,$8FFF`: `rom`, `read`, `protected` or `invalid`, each kind once; the program carries it, and the interpreter does not enforce it yet |
| `opt`, `list`, `nolist`, `page` | accepted and ignored: they are about the listing file, which there is none of |

Without `end`, the entry point is a label named `START`, and failing that the first instruction.

`memory` reads its addresses as EASy68K does: no forward reference, and each masked to the 24 bits of the address bus, so `$FF001000` is `$001000`. EASy68K's map has one range of each kind, so a second `memory rom` replaces the first, with a warning, and a range that ends before it starts, which covers nothing, is a warning too.

`equ`, `set` and `reg` need a label, and so does a `section` with no number, which it sets to the number of the section in force; `page`, `memory` and the conditional directives take none. A `reg` list has to be defined above the `movem` that reads it, and it may not appear in an expression. 

## Projects of several files

A **project** is a map from a root-relative path to a file's text or to its bytes, plus the path of the **entry file** to assemble. Every other file is reached from the entry file through `include` or `incbin`, or is not read at all; nothing on a disk is ever opened by the assembler.

```ts
S68k.assemble({
    files: {'main.x68': source, 'lib/io.x68': library, 'data/sprite.bin': bytes},
    entry: 'main.x68'
})
```

`include` is **textual**: the included file's lines are assembled where the `include` line is, in the same section, at the same address, in one symbol namespace, with the local label scopes running across the boundary. The file name may be quoted with either quote or not quoted at all, the path is looked for beside the file that wrote it first and at the project root second. A file may be included more than once. `end` can only appear in the entry file.

`incbin` takes either kind of file: a binary one contributes its bytes and a text one its Windows-1252 bytes. Adding a label on it names its first byte.

Every location carries the file it is in, so a diagnostic, a breakpoint (`{file, line}`), the current line and a call-stack frame all name one file of the project. A diagnostic raised in an included file carries the `include` lines it was reached through as related locations, and so does every assembled instruction (`includeChain`, innermost first): a file included twice has one location per line and two addresses, and the chain is the only thing that tells the two copies apart.

## Text and the trap #15 tasks

A character is one byte in **Windows-1252**, the code page EASy68K runs in: Latin-1, plus `€ ‚ ƒ „ … † ‡ ˆ ‰ Š ‹ Œ Ž ‘ ’ “ ” • – — ˜ ™ š › œ ž Ÿ` at `$80` to `$9F`. The assembler stores `dc.b 'café €'` as those bytes and refuses a character with no byte (`character_above_latin1`), and the interpreter uses the same table both ways at run time.

`TRAP #15` stops the interpreter with an `Interrupt` and waits for an answer. The interpreter owns what each task means; the host only displays text and passes on what was typed:

| Task | The interrupt carries, or the answer is |
|---|---|
| 0, 1 | the text: the D1.W characters at (A1), at most 255, stopping at a NUL |
| 13, 14 | the text: the NUL terminated string at (A1) |
| 3 | D1.L as a signed decimal number: `-5` |
| 6 | the character in D1.B |
| 15 | D1.L unsigned in the base in D2.B, upper case: `FF` |
| 17 | the string at (A1), then D1.L as in task 3, as one text |
| 20 | D1.L in a field of D2.B columns, a signed byte: `    -5` for 6, `-5    ` for -6 |
| 2 | answered with the line typed: its first 79 characters go to (A1) with a NUL, and their count to D1.L |
| 4, 18 | answered with the line typed, read into D1.L by `atoi`: `12abc` is 12, `abc` and an empty line are 0 |
| 5 | answered with the key typed, into D1.B; Enter is `$0D`, whether the host sends `'\r'` or `'\n'` |

A line ends at its first line terminator, and a typed character Windows-1252 cannot store is stored as `?`. `atoi` reads leading white space, one sign and the digits up to the first non-digit; a number too long for 32 bits wraps, so every number from -2147483648 to 4294967295 lands in D1.L exactly.

### Input settings: tasks 12 and 16

Task 12 turns the echo of what is typed off for D1.B = 0 and on for anything else; task 16 turns the input prompt (EASy68K's flashing cursor) off and on with D1.B = 0 and 1, and the line feed a key read of Enter echoes after the carriage return with 2 and 3. They raise no interrupt: they are the interpreter's own state, `getInputSettings()` (`{echo, prompt, line_feed}`), all on when a program starts as in EASy68K, and journaled like a register (a `SetInputSettings` mutation), so undo puts them back. A host reads them while a read task waits and shows the input accordingly. With the echo off, Enter still ends a line read on a new line, as in EASy68K.

### Files: tasks 50 to 59

The host's file system does the work, so each file task is an interrupt carrying its arguments decoded, and the answer says what the file system did; the interpreter writes EASy68K's result to D0.W: 0 success, 1 end of file, 2 error, 3 read only. A path is the NUL terminated string at the address, at most 255 characters, decoded from Windows-1252, with `\` written as `/` because Windows takes both as separators; a host maps it to its own files.

| Task | The interrupt carries | The answer, and what is written |
|---|---|---|
| 50 | `CloseAllFiles` | `true` when every file closed: 0, else 2 |
| 51 | `OpenFile(path)`: open an existing file for reading and writing, or for reading only when it cannot be written | `{handle, read_only}` or `null`: D1.L is the file number and D0.W 0, or 3 for reading only; `null` is -1 in D1.L and 2 |
| 52 | `NewFile(path)`: open for reading and writing, creating the file or emptying it | the file number or `null`, as for 51 |
| 53 | `ReadFile {handle, count}`: at most D2.L bytes from the file's position | the bytes or `null`: some bytes go to (A1), their count to D2.L and 0 to D0.W, a short read included; none is the end of the file, 1, with D2.L left as it was; `null` is 2 |
| 54 | `WriteFile {handle, bytes}`: the D2.L bytes at (A1) | `true` when all were written: 0, else 2 |
| 55 | `PositionFile {handle, offset}`: D2.L bytes from the start | `true`: 0, else 2 |
| 56 | `CloseFile(handle)` | `true`: 0, else 2 |
| 57 | `DeleteFile(path)` | `true`: 0, else 2 |
| 58 | `FileDialog {mode, title, filter, path}`: D1.L 0 opens and 1 saves; the title at (A1) and the filter at (A2) are empty when their register is 0; the path at (A3) is where the dialog starts | the path chosen or `null` for a cancel: a path goes to (A3), at most 255 characters and NULs to 256 bytes, with 1 in D1.L; a cancel is 0 in D1.L; D0.W is 0, or 2 when the 256 bytes do not fit |
| 59 | `FileExists(path)` | `"Writable"`, `"ReadOnly"` or `"Missing"`: 0, 3 or 2 |

A file number is 0 to 7, the lowest free one, as EASy68K numbers its eight files; a host opens no more than eight at once and answers `null` for a ninth. What EASy68K decides before it touches a file is decided by the interpreter, with 2 in D0.W and no interrupt: a file number outside 0 to 7 for tasks 53 to 56, a read or a write of no bytes, a buffer that runs past the end of memory and a negative position. A file opened for reading only reports 3 when it opens, and a write to it fails with 2, as `fwrite` fails in EASy68K. The bytes of a write and of a read cross the WebAssembly boundary as a `Uint8Array` (serde's bytes, which `serde_wasm_bindgen` copies in one go), and a read's bytes are written to memory through the interpreter's own journal, so undo puts back what a read wrote.

### Sound: tasks 70 to 77

The interrupts carry the arguments, and the interpreter plays nothing itself: `PlaySound(path)` (70), `LoadSound {path, index}` (71), `PlayLoadedSound(index)` (72), their DirectX forms `PlaySoundDirectX`, `LoadSoundDirectX` and `PlayLoadedSoundDirectX` (73 to 75), and `ControlSound {index, control}` and `ControlSoundDirectX` (76, 77), where the index is D1.B and the control D2.L: 0 plays once, 1 loops, 2 stops the sound and 3 stops every sound. A host with something to play them on answers with a `SoundOutcome`, what its player did, and the interpreter writes D0.W as EASy68K does; 71 has no result. A host with nothing to play them on answers `Terminate`.

| Outcome | It means | 70, 72 | 73 to 77 |
|---|---|---|---|
| `"Done"` | the task did what it asked: a sound started, a load loaded, a stop stopped | 1 | 1 |
| `"Busy"` | the standard player was still playing an earlier sound; the DirectX player mixes, so it is never busy | 0 | 0 |
| `"Missing"` | no file at the path, or no sound in the slot | D0 left as it was | 0 |
| `"Unplayable"` | a file that is there but cannot be played | 0 | 0 |

### When a task cannot be done, and why a program ended

Every error an instruction raises ends the program, as an exception would on a 68000 with no handler: the status is `TerminatedWithException`, the error is thrown, and `getTermination()` answers it as the cause. The trap tasks raise two of their own: `UnsupportedTrapTask {task}` for a number that is not one of EASy68K's tasks and for the tasks a page in a browser cannot do faithfully (10, 21, 22, 25, 30 to 32, 40 to 43, 60, 62, 100 to 107), and `InvalidTrapArgument {task, reason}` for a value a task cannot take, the reason naming the register. Undoing the step that failed brings the program back to it, running.

`getTermination()` is `null` while the program runs, and then says why it ended: `TerminateTask` (task 9), `EndOfProgram {address}` (it ran into `$FFFF $FFFF` it never placed, above), `TerminatedByHost` (an interrupt answered with `Terminate`) or `Exception` with the error.

An answer is taken only by the interrupt of the same `type`, with a value the task can take, or it is `Terminate`, which answers any task and ends the program. Anything else is refused with a typed error and leaves the interrupt waiting: `NoPendingInterrupt`, `InvalidAnswer {interrupt, reason}` for an answer of another type, a value that is not an answer at all, a key state of the other form, more bytes than a read asked for or a file number past 7, and the `OutOfBounds` of a write that does not fit in memory. Every WebAssembly method throws a `RuntimeError` object and none of them panics, so the interpreter stays usable after a bad call.

Where s68k departs from EASy68K, on purpose:

- A base outside 2 to 36 for task 15, a D1.B other than 0 to 3 for task 16 and a D1.L other than 0 and 1 for task 58 stop the program with `InvalidTrapArgument`; EASy68K does nothing (and task 58 reports success).
- A number that is no task, and a task s68k does not carry out, stop the program with `UnsupportedTrapTask`; EASy68K takes the trap exception for the first and carries out the second.
- A D1.W of `$8000` or more for tasks 0 and 1, which EASy68K reads as a negative index into its buffer, displays up to 255 characters, as for any D1.W above 255.
- Overflow in `atoi` wraps modulo 2^32; EASy68K's own `atoi` (Borland's runtime) was not available to check.
- The characters typed after the 79th of a line are dropped. EASy68K ends the line at the 80th key and keeps the next one for the following read.
- A read of no bytes (task 53 with D2.L = 0) reports 2. EASy68K's `fread` reports 1 instead once an earlier read has reached the end of the file, which is C's end-of-file indicator, and s68k does not keep one.
- A count that runs past the end of memory is 2 also when (A1) + D2.L passes 4 GB, where EASy68K's 32 bit check wraps round and reads past its memory; task 58 checks the whole 256 bytes it writes where EASy68K checks only the name.
- A path is sent with `/` for `\`.
- On Windows EASy68K cannot delete a file that is open, and reports 2; whether a file that is open can be deleted is the host's file system's to say.

## Known limitations
1. The program runs as supervisor, always. The status register is a 16-bit register whose low byte is the condition codes and whose high byte is trace, supervisor and the interrupt mask, readable (`getSr()`, and the `SR:` line of the command line) and of no effect at all. It starts at `$2700`, so `andi #$00,sr` "puts the CPU in user mode" and changes nothing that runs. `move usp,an` and `move an,usp` are refused for the same reason: there is one stack pointer, `a7`.
2. `chk`, `trapv` and `illegal` raise their exception by ending the run, and so does every other runtime error: an address error, a word that is no instruction, a division by zero, a `trap #15` task that cannot be done. There are no exception vectors, no supervisor stack frame and no `rte`, so a program cannot handle one. The runtime error names the cause, and is the program's termination.

## Running the command-line interpreter

Install [Rust](https://www.rust-lang.org/tools/install), clone the repository,
and run `cargo run` from its root. This assembles and runs `code-to-run.asm`.
To run another file, pass its path: `cargo run -- my-program.asm`.

The command line answers the text tasks on the terminal, honouring the echo of task 12, and the file tasks on the disk: a path is taken relative to the directory of the file it runs, task 51 opens a file it cannot write for reading only, and the dialog of task 58 asks for a path on the terminal, an empty line cancelling it. The graphics, mouse and sound tasks are reported and end the run, and the last line says why the program ended.

## Building the WebAssembly package

Install [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/), then run
`npm run build-wasm` in the `ts-lib` directory. This creates the compiled
package in `ts-lib/src/pkg`.
