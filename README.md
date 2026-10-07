# s68k

s68k is an M68K assembler and interpreter written in Rust. The assembler reports useful errors and warnings to help with learning assembly. The emulator runs the assembled program (not in memory) and implements useful debugging features to step, undo, add breakpoints through the program

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
syntax, addressing modes, and directives. It is not intended to produce
instruction binaries for real 68000 systems.

## Architecture

The library has three parts:

- **Assembler** (`src/assembler/`): reads the source files of a project, starting from the entry file, and returns diagnostics and, when there are no errors, a program. It contains a tokenizer, a parser, a symbol table, an expression evaluator, the layout pass, the instruction table, and the analyzer. Assembly reports all of the diagnostics it finds.

- **Program**: the assembler's output, a program containing assembled instructions with their addresses, sizes, and source locations; the initial memory; the symbols; and the entry point.

- **Interpreter** (`src/interpreter.rs`): runs a program using registers, memory, and the status register. It supports stepping, running to the end, undo history, breakpoints and a call stack.

The interpreter does not load encoded instructions into memory, so instructions
cannot be modified at runtime. Every instruction occupies four bytes regardless
of its size on a real 68000.

## Future work

- Real instruction encodings and programs loaded in memory
- Macros and conditional assembly
- A disassembler


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
| `simhalt` | pauses the run; the next `run` or `step` resumes at the following instruction |
| `section` | switches between the sixteen location counters, 0 to 15, each going on from where it was left |
| `offset` | opens a region that produces no bytes, where `ds` names the fields of a structure by their offsets; `org *` ends it |
| `include` | assembles another file of the project here, as if its lines had been pasted at this line |
| `incbin` | puts another file's bytes in memory here, as a `dc.b` of the whole file would |
| `opt`, `list`, `nolist`, `page` | accepted and ignored: they are about the listing file, which there is none of |

Without `end`, the entry point is a label named `START`, and failing that the first instruction.

`equ`, `set` and `reg` need a label, and so does a `section` with no number, which it sets to the number of the section in force; `page` and the conditional directives take none. A `reg` list has to be defined above the `movem` that reads it, and it may not appear in an expression. 

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

The interrupts carry the arguments and nothing plays: `PlaySound(path)` (70), `LoadSound {path, index}` (71), `PlayLoadedSound(index)` (72), their DirectX forms `PlaySoundDirectX`, `LoadSoundDirectX` and `PlayLoadedSoundDirectX` (73 to 75), and `ControlSound {index, control}` and `ControlSoundDirectX` (76, 77), where the index is D1.B and the control D2.L: 0 plays once, 1 loops, 2 stops the sound and 3 stops every sound. A host with something to play them on answers whether it did, 1 or 0 in D0.W; 71 has no result. A host with nothing to play them on answers `Terminate`.

### When a task cannot be done, and why a program ended

Every error an instruction raises ends the program, as an exception would on a 68000 with no handler: the status is `TerminatedWithException`, the error is thrown, and `getTermination()` answers it as the cause. The trap tasks raise two of their own: `UnsupportedTrapTask {task}` for a number that is not one of EASy68K's tasks and for the tasks a page in a browser cannot do faithfully (10, 21, 22, 25, 30 to 32, 40 to 43, 60, 62, 100 to 107), and `InvalidTrapArgument {task, reason}` for a value a task cannot take, the reason naming the register. Undoing the step that failed brings the program back to it, running.

`getTermination()` is `null` while the program runs, and then says why it ended: `TerminateTask` (task 9), `EndOfProgram` (the program counter left the last instruction), `TerminatedByHost` (an interrupt answered with `Terminate`) or `Exception` with the error.

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
- Tasks 70 and 72 write the host's answer to D0.W; EASy68K leaves D0.W alone when the file or the loaded sound is missing.
- On Windows EASy68K cannot delete a file that is open, and reports 2; whether a file that is open can be deleted is the host's file system's to say.

## Known limitations
1. Every instruction is four bytes wide whatever it encodes to on a real 68000, so an address computed from instruction sizes will not match the hardware. It is also why a PC-relative operand is resolved while the program is assembled: there is no extension word in memory to read the displacement from, so the assembler stores it and the interpreter adds it to the address of the instruction being executed.
2. The program runs as supervisor, always. The status register is a 16-bit register whose low byte is the condition codes and whose high byte is trace, supervisor and the interrupt mask, readable (`getSr()`, and the `SR:` line of the command line) and of no effect at all. It starts at `$2700`, so `andi #$00,sr` "puts the CPU in user mode" and changes nothing that runs. `move usp,an` and `move an,usp` are refused for the same reason: there is one stack pointer, `a7`.
4. `chk`, `trapv` and `illegal` raise their exception by ending the run, and so does every other runtime error: an address error, a division by zero, a `trap #15` task that cannot be done. There are no exception vectors, no supervisor stack frame and no `rte`, so a program cannot handle one. The runtime error names the cause, and is the program's termination.

## Running the command-line interpreter

Install [Rust](https://www.rust-lang.org/tools/install), clone the repository,
and run `cargo run` from its root. This assembles and runs `code-to-run.asm`.
To run another file, pass its path: `cargo run -- my-program.asm`.

The command line answers the text tasks on the terminal, honouring the echo of task 12, and the file tasks on the disk: a path is taken relative to the directory of the file it runs, task 51 opens a file it cannot write for reading only, and the dialog of task 58 asks for a path on the terminal, an empty line cancelling it. The graphics, mouse and sound tasks are reported and end the run, and the last line says why the program ended.

## Building the WebAssembly package

Install [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/), then run
`npm run build-wasm` in the `ts-lib` directory. This creates the compiled
package in `ts-lib/src/pkg`.
