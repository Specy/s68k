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

- **Machine** (`src/machine/`): the 68000's encoding. A decoder reads the words of an instruction for a 68000, a 68010 or a 68020, an encoder writes them, a disassembler prints an instruction in EASy68K's syntax, the decode cache keeps the instructions the interpreter runs, and the exceptions' frames and the cycles are each processor's.

The assembler chooses each instruction's 68000 encoding as EASy68K does (below) and writes it into the program's memory at its real size, so every address is the one a 68000 and EASy68K's listing give. The interpreter runs those bytes (below, "Code is memory").

## Code is memory

The interpreter runs what memory holds, through a cache of decoded instructions: 4 KB pages of one slot per word, a page made only where code is, filled from the assembler's own instructions when the interpreter is made, so the program's code is never decoded while it runs. Every write passes the cache, the program's, a trap task's, the host's and undo's, and clears the instructions it covers, which are decoded again from their new bytes before they run. So a program can read and change its own code, and the change runs, as on a 68000; undo writes the old bytes back, and the old instruction runs again.

- **A code write** is a write of the program's own, its stores and a trap task's answer, into an instruction it had decoded. Each step says which instruction it overwrote first (`wroteCode` on the step, `getCodeWrite()`), as the instruction's address and the index of its line. A run stops after one when asked to (`runWithBreakpoints(…, {stopOnCodeWrite: true})`, the status `CodeWrite`), and the next run goes on. A Poke, a testcase's starting memory and undo never make one.
- **The source** is read through the program's own map of the lines it laid out, which does not move when bytes change: a changed line keeps its location, its breakpoint and its `getInstructionAt` as assembled, and `disassembleAt(address)` prints what memory holds there now. `getInstructionAt` also says what the line assembled as when the assembler chose something it did not write (`assembledAs`: `add.l #1,d0` is `ADDQ.L #1,D0`, `bra loop` is `BRA.S loop`, `lea $1000,a0` is `LEA $1000.W,A0`).
- **The 24-bit bus.** Memory is the 16 MB the 68000's 24 address lines reach, and an address is read in its low 24 bits, as EASy68K's `& ADDRMASK` reads it. An access that runs past `$FFFFFF` goes on at `$000000`, as the bus does: a long at `$FFFFFE`, the stack pushed below `$000000`, an instruction whose words run past the end, a string a trap task reads and a line it writes back, and the host's reads and writes. The file tasks' buffers are the exception, as in EASy68K: a read or a write that runs past the end answers 2 (below). The map, the decode cache and the watched range see the bytes where they land. Only a run of bytes longer than the whole 16 MB is refused, out of bounds.
- **The end.** `$FFFF $FFFF` is EASy68K's SIMHALT wherever it is. Inside the bytes the program placed, its code and its data, it pauses, as `simhalt` does. Anywhere else it ends the program, `EndOfProgram` with the address: memory starts as `$FF`, so that is where a program that does not end itself ends, one past its last instruction when nothing follows it (ADR 0009). Any other bytes run as what they decode to, so a program that runs on into its data, or jumps there, runs the data, and a word that is no instruction is an illegal instruction. An instruction fetched from an odd address is an address error, which a jump to one raises in the jump itself, as a 68000 does.

## Exceptions

The machine is a 68000 as EASy68K simulates one, and a 68000 where EASy68K is wrong (ADR 0007). Every exception is taken through the vector table at `$000` to `$3FF` (ADR 0008):

| Vector | Address | Raised by | Stacked PC |
|---|---|---|---|
| 2 | `$008` | a bus error: an access the memory map refuses (below, "The memory map") | as for an address error at the same access; an instruction that could not be fetched, its own |
| 3 | `$00C` | an address error: a word or long at an odd address, an instruction fetched from one, a jump to one | where the prefetch had got to, the 68000's (below) |
| 4 | `$010` | `illegal`, and a word that is no instruction | the instruction's |
| 5 | `$014` | `divs` or `divu` by zero | the next instruction's |
| 6 | `$018` | `chk` out of bounds | the next instruction's |
| 7 | `$01C` | `trapv` with V set | the next instruction's |
| 8 | `$020` | a privileged instruction in user mode: `move <ea>,sr`, `andi`/`ori`/`eori #n,sr`, `rte`, `stop`, `reset`, `move usp` | the instruction's |
| 9 | `$024` | trace: after an instruction that started with T set | the next instruction's, or the handler's of the exception it raised |
| 10, 11 | `$028`, `$02C` | a line A word `$Axxx` and a line F word `$Fxxx` | the instruction's |
| 25 to 31 | `$064` to `$07C` | an interrupt request at level 1 to 7, through its autovector (below, "Interrupts") | the instruction it came before |
| 32 to 46 | `$080` to `$0B8` | `trap #0` to `trap #14` | the next instruction's |
| 47 | `$0BC` | a `trap #15` task that is none of EASy68K's | the next instruction's |

A program for the 68010 has the same table and two more causes, `bkpt` and `movec` of a register it does not have, both vector 4, and the format error of `rte`, vector 14 at `$038`; it reads the vectors through VBR, pushes the 68010's frames, and its address and bus errors stack the instruction that faulted (below, "The 68010"). A program for the 68020 has the 68010's, `chk2` and `trapcc` beside `chk` and `trapv`, and the 68020's frames, and reads and writes data at odd addresses, so its only address error is an instruction fetched from one (below, "The 68020's supervisor model").

- **Taking one.** The processor reads the vector, copies the status register, sets S and clears T, pushes the frame on the supervisor stack and jumps to the handler. A frame is the status register then the program counter, 6 bytes, except a bus error's and an address error's, 14: the special status word (bits 15 to 5 of the instruction register, R/W, I/N clear, and the function code of the mode and space of the access), the access address, the instruction register, the status register and the program counter. `rte` pops the six bytes; a bus or address error's handler takes the other eight off itself.
- **No handler.** A vector holding an odd address has no handler, and the program ends with the error as it did before exceptions were taken, the vector named beside it (`Exception {cause, vector}`). Memory starts as `$FF`, so every vector reads `$FFFFFFFF` until the program writes it, and a program that installs nothing ends exactly as before.
- **A double fault** halts the processor, the termination `Halted {cause, vector}`: a frame pushed on an odd supervisor stack pointer, or into a range the memory map refuses, a vector read from an invalid range, and a bus or address error whose handler the memory map refuses to fetch. The handler of any other exception is fetched by the next step, whose bus error halts in turn if its own handler cannot be fetched.
- **Supervisor and user mode.** A program starts in supervisor mode with EASy68K's registers: SR `$2000`, A7 the supervisor stack pointer at `$01000000`, the user stack pointer at `$00FF0000`. A7 is the stack pointer of the mode S names, and every write of SR, every exception and `rte` can swap the two. `getSystem()` answers `{sr, usp, ssp}`, and `setSr`, `setUsp` and `setSsp` write them, journaled inside a poke. A 68020 has a third, the master stack pointer, which M chooses (below, "The 68020's supervisor model"). `move sr,<ea>` is not privileged on a 68000, and `trap #15` works in user mode, as in EASy68K.
- **Trace** is T sampled before an instruction: an instruction that started with it set takes the trace exception after it, unless it raised an illegal instruction, a privilege violation, a line A or F word or an address error, which it did not finish. After `trap`, `trapv`, `chk` or a division by zero, the trace frame goes on top of theirs, so the trace handler runs first and returns into theirs, as on a 68000 (EASy68K pushes them the other way round). A `trap #15` task that waits on the host is traced once it is answered.
- **The address error's program counter and side effects** are the 68000's as MAME's microcode leaves them on the 55,606 address errors of the conformance suite: a jump to an odd address faults in its own prefetch, `bsr` with its return address pushed and `jsr` without; a data access stacks the address the prefetch had reached, which depends on the operands; a word through `(An)+` has moved the register and a long has not; `clr` and `move sr,<ea>` read their destination first, so an odd one faults as a read. `src/machine/exception.rs` holds the rules.
- **The call stack** gains a frame for each exception taken, `kind: 'exception'` with its `vector`, which `rte` takes off, what raised it, `cause`, the name of its error, and the vector base register it was taken with, `vbr`; Undo walks back through all of it, the stack swaps included (`WriteStackPointers`, which says which stack pointer A7 became, `PushException`, with the same `cause` and `vbr`, and `PopException` mutations).

## Interrupts

The 68000's interrupt requests, IRQs, are EASy68K's without its hardware window: seven levels, all autovectored, from timers, the keyboard and the mouse (the asm-editor's `docs/design/m68k-machine.md`, decisions 13 and 16 to 22).

- **Latched.** A request at a level from 1 to 7 waits in a latch until it is taken; a second request at a level already pending is the same request.
- **Taken between two instructions.** Before the next instruction, the highest pending level above the status register's interrupt mask is taken through its autovector, vector 24 plus the level at `$60` plus four times it, and level 7 whatever the mask. Taking one is an exception like the others, a frame of 6 bytes on the supervisor stack, S set and T cleared, and then the mask rises to the level, so a handler is interrupted only by a higher level or by level 7. It is an execution step of its own, `kind: 'irq'`, with no location: the History shows it and Undo takes it back on its own. An autovector with no handler ends the program with `Irq {level}` as the cause, the vector named.
- **Against the other exceptions**, as on a 68000: an instruction's trace frame and the frame of a `trap`, `chk`, `trapv` or division by zero are pushed first, and the IRQ's handler runs before theirs; an illegal instruction at the program counter has not been fetched yet, and raises its exception after the IRQ handler's `rte`. A request that comes while a `trap #15` task waits on the host is taken after the answer. (EASy68K loses an interrupt taken beside a trap, and runs an illegal instruction twice.)
- **`stop #n`** is privileged, loads the whole status register, T and S included, and the processor waits for a request above the new mask: the status `Stopped`, the program counter on the next instruction. `step()` and every run return `Stopped` at once while it waits. A request that can wake it makes the status `Running` at once, and the next step takes it; `getMachineState().stopped` is true until then. A `stop` with a request above its new mask already pending does not wait at all, and one that starts with T set takes the trace exception instead of stopping, as on a 68000. With S clear in the immediate, the processor stops in user mode, where EASy68K raises a privilege violation, and EASy68K runs one more instruction before the interrupt that wakes it.
- **`reset`** is privileged and does nothing but take its time, 132 cycles (below, "Cycles"): there are no devices behind the reset line. Nothing runs the 68000's reset sequence.
- **The sources** are the program's to set up and the host's to deliver:

  | Task | `D1` | What it sets |
  |---|---|---|
  | 32 | `D1.B` = 6, with `D2.B` and `D3.L` | the timers, EASy68K's `setAutoIRQ`: `D2.B` = 0 turns all off, 1 to 7 turns that level's off, `$81` to `$87` fires that level every `D3.L` milliseconds; any other `D2.B` does nothing |
  | 60 | level in bits 15 to 8, events in bits 0 to 2 | the mouse's: a button down (bit 0), a button up (bit 1), a move (bit 2), each chosen one set to the level, 0 turning it off; a level above 7 makes the call do nothing |
  | 62 | the same, bits 0 and 1 | the keyboard's: a key down (bit 0) and a key up (bit 1) |

  Task 32 also answers EASy68K's version, `$00051001`, in `D1.L` for `D1.B` = 4, and accepts 5. Its parts 0 to 3 and 7, the hardware window and its devices' addresses, are refused with `UnsupportedTrapTask {task: 32}`: there is no hardware window. Any other `D1.B` does nothing, as in EASy68K.
- **The host calls.** `getMachineState()` answers `{pendingIrqs, stopped, autoIrqs: {level, intervalMs}[], keyboardLevels: {keyDown, keyUp}, mouseLevels: {down, up, move}}`, a level of 0 being none. The host runs the timers on the program's time and calls `raiseIrq(level)` at each firing, and reports the Screen's events with `deviceEvent('keyDown' | 'keyUp' | 'mouseDown' | 'mouseUp' | 'mouseMove')`, which latches the level the program set for it and answers it, or `null`. Both are input between two instructions, like a key pressed, and do nothing after the end of the program.
- **Undo.** What the program set up and whether the processor is stopped are journaled like a register, a `SetMachineState {old, new}` mutation, and undo puts them back. The latch is the host's input as well as the program's: undoing the IRQ step that took a request makes it pending again, so running on takes it again, but a request that came after a step stays pending when the step is undone, since the program never saw it.

## The memory map

A program declares ranges of EASy68K's memory map with `memory`, one of each kind at most, and the interpreter enforces them as EASy68K's simulator does (`memoryMapCheck`, `mem_put` and `mem_req` in its `UTILS.CPP`):

| Kind | A read | A write |
|---|---|---|
| `rom` | allowed | ignored: memory keeps what it held, and it is no error |
| `read` | allowed | a bus error |
| `protected` | a bus error from user mode | a bus error from user mode |
| `invalid` | a bus error | a bus error |

- **What is checked.** Reads against invalid and protected, by their first byte; writes against all four, by any byte, so a long that starts below a range and runs into it is refused as a write and not as a read; and every word of an instruction as it is fetched, as EASy68K fetches each as a read, so code in an invalid range, or in a protected one in user mode, is a bus error in program space. The kinds are checked in EASy68K's order, invalid, protected, read only and ROM. An address error comes first, then the map.
- **A bus error** is vector 2 with the 14-byte frame of an address error, its status word the 68000's: R/W as the access was, I/N clear, and the function code of the processor's real mode, where EASy68K always says "write" and supervisor. With no handler the program ends with `BusError {address, size, access, range}`, the vector named beside it. The instruction aborts there, as a 68000's does; EASy68K finishes it.
- **Protected reads the processor's S bit**, where EASy68K reads it from its window's SR field. The vector of an exception is read in supervisor mode, so a protected vector table serves user mode.
- **A ROM store is no store**: it journals nothing, changes no code, is no Code write and is not a watched write.
- **The `trap #15` tasks check their buffers as EASy68K does**: a string a task reads (tasks 0, 1, 13, 14, 17, 18, 95, and the paths of 51, 52, 57, 59, 70, 71, 73 and 74), from its start to its NUL, against invalid; the bytes task 54 writes a file from against invalid; and the bytes task 53 reads a file into against invalid, read only and ROM, where ROM takes nothing and the task does nothing. None checks protected, and task 2's line and task 58's dialog are written with no check, as in EASy68K. A buffer of no bytes, a string that is only its NUL or a read of none, is refused where it starts inside a range past the range's first byte, and not on that byte, as EASy68K's `loc + bytes - 1 >= start && loc <= end` has it.
- **The debugger is not the program.** A Poke, a testcase's starting memory, the host's reads and `disassembleAt`, undo, and the program's own initial contents go past the map.
- With no range declared, an access asks one flag and goes on. The decode cache keeps no instruction a word of which an invalid or protected range reaches, so each fetch of one is decoded and checked in the mode the processor is in then, and a write into one is no Code write.

## Cycles

The interpreter counts the clock cycles a 68000 takes, with the 68000's own time for each instruction, exception and interrupt request (the asm-editor's `docs/design/m68k-machine.md`, decision 23; `src/machine/timing.rs`). A 68010 program counts the 68010's, exactly too, and a 68020 program an estimate, the cache case of the 68020's manual (below, "The 68020's cycles").

- **The source.** An instruction's base time and the time of its effective addresses are those of section 8 of the 68000 User's Manual, with the corrections of Yacht ("Yet Another Cycle Hunting Table"), and every one is checked against the length in cycles the conformance suite records for each of its tests (below, "Conformance"). The bus answers at once, as the suite's does: there are no wait states.
- **The data counts** where it does on a 68000. `mulu` takes two cycles for each bit set in its source, and `muls` for each change between two of its bits; `divu` and `divs` take the time of the microcode's loop, worked out with Jorge Cwik's method (76 to 136 and 120 to 156, or 10 and 16 or 18 when the quotient overflows); a shift or a rotate of a register takes two per bit of its count, mod 64; `movem` four or eight per register; `Bcc` 10 when it branches, and 8 for `.s` or 12 for `.w` when it does not; `DBcc` 10 when it branches back, 12 when its condition is true and 14 when its counter runs out; `Scc` two more when it sets a register; and `bchg`, `bclr` and `bset` two more for a bit of a register above 15.
- **An exception** takes the 68000's time, which is the step's whole time: 34 for `trap #0` to `#14`, `trapv`, an illegal instruction, a privilege violation and the line A and F words; 38 and its source's effective address for a division by zero; 38 for `chk` above its bound and 40 below zero, with its source's; and 58 for an address error or a bus error from the access it aborted, after the time the instruction had taken until then. A trace exception adds 34 after the instruction, and an IRQ, a step of its own, takes 44. `stop` takes 4 and `reset` 132.
- **`trap #15`'s tasks take no time**, as in EASy68K. A number that is none of EASy68K's tasks is a `trap` through vector 47, 34.
- **The counter** is 64 bits. `getCycles()` reads it, a `bigint` (`get_cycles()` in Rust); each step of the history says what it took, `cycles`, and undo takes it back. Task 30 clears the counter and task 31 reads it into `D1.L`, 0 once it is past 32 bits, as in EASy68K; a step that clears it still says what it took.
- **The 68010** counts its own time (below, "The 68010"), and **the 68020** an estimate (below, "The 68020's cycles"). `program.getInfo().cyclesExact`, `program.hasExactCycles()` in TypeScript, says which: true for the 68000 and the 68010, false for the 68020, whose count a host shows as an estimate, and false for a 68000 or 68010 program under `opt bit` that has a bit-field instruction, whose bit fields take EASy68K's time, the 68020's cache case (below, "The 68020's instructions"), while every other instruction takes its exact time.

Where EASy68K's counts differ from the 68000's, s68k counts the 68000's:

| | s68k, the 68000 | EASy68K |
|---|---|---|
| `trap #0` to `#14`, and a task through vector 47 | 34 | 38 |
| a division by zero | 38 and the source's effective address | 42 |
| `chk` out of bounds | 38 above the bound, 40 below zero, and the source's effective address | 44 |
| an IRQ | 44 when it is taken | 34 when it is taken, and 44 more when it is requested |
| an address or bus error | the time before the access, and 58 | 50 |
| `tst` of memory | 4 and the effective address | 4 |
| `muls` | two per change between two bits of the source, a zero below the lowest | two per change, counted slightly differently |

## Conformance

The interpreter is checked against [SingleStepTests/m68000](https://github.com/SingleStepTests/m68000), tests generated from MAME's microcoded 68000 (MIT): each is the state before one instruction and the state after it, and the runner sets the first, runs the instruction and compares the registers, the status register, the program counter, every word of RAM the second lists and the length in cycles, and checks that nothing else was written (ADR 0010). Musashi's sixty self-checking 68000 programs run as well, each to the verdict it writes.

- `cargo test` runs the vendored subset, the first 50 tests of each of the 127 files that raise no address error (`tests/m68000/`), and Musashi's programs (`tests/musashi/mc68000/`).
- The whole suite, 317,500 tests, runs from a fetched copy:

  ```sh
  scripts/fetch-m68000-tests.sh
  cargo test --release --features conformance conformance:: -- --ignored --nocapture
  ```

  It takes a few minutes and prints the counts of every file; `conformance::` leaves out the crate's other ignored tests, which need GNU binutils for m68k. `S68K_M68000_ONLY=ASR.b,ASR.w` runs only the files it names, and `S68K_M68000_TESTS` points at another copy of the suite's `v1` folder. A GitHub Actions workflow runs it weekly.
- What is not checked is in one skip list with a reason per entry (`src/conformance/skips.rs`): `trap #15`, EASy68K's service trap. Every exception the suite takes is checked, its frame included: the address errors, the privilege violations, line A and F, `trap`, `chk`, `trapv` and `rte`, in both modes, and so are `stop` and `reset`. A `stop` leaves the processor stopped before it prefetches, so its program counter is compared with the suite's next prefetch itself.
- The length of every test is compared with the cycles its step took (above, "Cycles"), those of `tas` and `trapv` included: the suite's README does not vouch for them, but the 68000's times and the suite's agree on all 2,500 of each.

The counts on 2026-10-28, with the suite at its pinned commit, the lengths compared:

| | Tests | Run and passed | Failed | Skipped |
|---|---|---|---|---|
| The whole suite | 317,500 | 317,340 | 0 | 160 |
| The vendored subset | 6,350 | 6,347 | 0 | 3 |
| Musashi's programs | 60 | 55 | 0 | 5 |
| Musashi's 68040 programs, as 68020 programs | 17 | 17 | 0 | 0 |

126 of the 127 files run and pass all 2,500 of their tests, length included, and `TRAP` its 2,340 that are not `trap #15`, the only skip. Of Musashi's programs, `abcd`, `sbcd`, `divs` and `divu` check Musashi's own answers where the 68000's microcode differs (a BCD digit above 9, the flags of a division that overflows), and `move` uses a 68020 form of `cmpi`, so those five stay skipped.

### The 68010

No single-step suite covers the 68010, so it is checked by a differential harness that runs a reference 68010 beside s68k, instruction by instruction (the asm-editor's `docs/design/m68k-machine.md`, decision 3; `src/test/differential.rs`). Every first word the decoder takes as a 68010 instruction is run from random states, one in eight of them traced under Musashi, by s68k and the reference, and the registers, both stack pointers, the status register, the program counter, VBR, SFC, DFC, every byte either wrote, which is where the frames are, and the cycles compared. Both references are MIT and fetched, never vendored, and compiled by `build.rs` with the `cc` crate only under their feature, for the host's tests:

```sh
scripts/fetch-musashi.sh
cargo test --release --features musashi differential -- --ignored --nocapture
scripts/fetch-moira.sh
cargo test --release --features moira differential -- --ignored --nocapture
```

- **Musashi** checks behaviour. Where it is not the 68010 the case is not compared, by one list with a reason per line (`RELAXATIONS`): it has no address errors in 68010 mode, forgets a pending trace when an instruction takes an exception, lets a 68010 `movec` CACR, runs the 68040's `move16`, pushes `link a7`'s A7 four lower, sets the flags the manual leaves undefined otherwise than MAME's 68000 does, and does not check `rte`'s version. Its cycles are a table of maxima with no loop mode, so they are reported and not compared.
- **Moira** checks behaviour and cycles. Its 68010 counts its cycles bus access by bus access and is checked against cputester on the hardware, loop mode included; the harness compiles it with precise timing and without its mimicry of Musashi. Every one-word instruction also runs in a loop of a `dbf` back to it, step by step, against Moira's loop mode. Only the flags the manual leaves undefined after the decimal instructions and a division by zero, and the cycles of `divs` of `$80000000`, which Moira takes the absolute value of in 32 bits, are left out.
- **GNU `as -m68010`** encodes every form of the 68010's instructions the same, in the crate's opt-in check against GNU `as`, which compares every form of the table with `as -m68000` and the 68010's with `as -m68010`: `S68K_BINUTILS_PREFIX=/usr/bin/m68k-linux-gnu- cargo test --release gnu_as -- --ignored`.

The counts on 2026-10-29, two cases of each first word, four against Moira:

| | Cases | Agree | Left out |
|---|---|---|---|
| Musashi, behaviour | 102,543 | 102,543 | 5,847, by the list |
| Moira, behaviour and cycles | 216,536 | 216,536 | 244 |
| Moira, loop mode, step by step | 28,064 loops, 10,786 of them in loop mode | 28,064 | 9,813, which take an exception |
| GNU `as -m68010` | 134 lines | 134 | |

## Future work

- Macros and conditional assembly


## Supported instructions
| Type                   | Instructions                                                                                                                                                                                                      |
|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Arithmetic             | add, sub, suba, adda, divs, divu, muls, mulu, addq, subq, addi, subi, addx, subx, negx                                                                                                                            |
| Comparison             | tst, cmp, cmpi, cmpa, cmpm                                                                                                                                                                                        |
| Branching and jumping  | bcc, bcs, beq, bne, blt, ble, bgt, bge, bls, bhi, bpl, bmi, blo, bhs, bvc, bvs, bsr, bra, jmp, jsr, rts, dbcc, dbcs, dbeq, dbne, dbge, dbgt, dble, dbls, dblt, dbhi, dbmi, dbpl, dbvc, dbvs, dbf, dbt, dbhs, dblo, dbra |
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo, `move`/`andi`/`ori`/`eori` with `sr` or `ccr` as an operand, and `move usp,an` and `move an,usp`; `move ccr,<ea>` is the 68010's, under `opt p=68010` |
| Bitwise                | not, or, ori, and, andi, eor, eori, lsl, lsr, asr, asl, rol, ror, roxl, roxr, btst, bclr, bchg, bset                                                                                                              |
| Binary coded decimal   | abcd, sbcd, nbcd                                                                                                                                                                                                 |
| Other                  | clr, exg, neg, ext, swap, move, link, unlk, lea, pea, moveq, movea, movem, movep, tas, nop, and the 68020's extb under `opt p=68020`                                                                               |
| The 68010's            | rtd, bkpt, movec, moves and `move ccr,<ea>`, under `opt p=68010` or `opt p=68020`                                                                                                                                 |
| The 68020's            | chk2, cmp2, cas, cas2, pack, unpk, divsl, divul, trapcc and its aliases, the long forms of muls, mulu, divs, divu, chk and link, `link.w`, the 68020 manual's spelling of `link`, tst and cmpi of what the 68000 cannot read, and extb, under `opt p=68020` (below, "The 68020's instructions") |
| Bit fields             | bftst, bfextu, bfexts, bfffo, bfchg, bfclr, bfset, bfins, under `opt p=68020`, or `opt bit` with the 68000's addressing modes                                                                                       |
| Exceptions             | chk, trapv, illegal, trap #0 to #14, rte, and rtr, which returns and restores the condition codes (above, "Exceptions")                                                                                            |
| Interrupts and control | stop, which waits for an interrupt request, and reset (above, "Interrupts")                                                                                                                                         |
| Interrupt              | trap #15, with the I/O tasks 0-9, 11-20, 23, 24 and 33 in `d0`, the file tasks 50-59, the mouse task 61, the sound tasks 70-77, the graphics tasks 80-96, the interrupt-request tasks 60 and 62, and task 32 with 4, 5 or 6 in `d1.b` (the `Interrupt` enum of `src/instructions.rs` is the list of those the host answers) |

## Encodings

Where the 68000 has several encodings of a line, s68k picks the one EASy68K picks:

- The first of EASy68K's forms that fits: `cmp #n,dn` is `cmp`, and `cmp #n,(a0)` is `cmpi`; `and #n,dn` and `or #n,dn` are `and` and `or`, and into memory `andi` and `ori`; `eor #n` is `eori`; `add #n,an` and `sub #n,an` are `adda` and `suba`, and `add #n,dn` is `addi`.
- The optimisations EASy68K always makes: `move.l #n,dn` of -128 to 127 is `moveq`, and an `add`, `addi`, `adda`, `sub`, `subi` or `suba` of 1 to 8 is `addq` or `subq`. Writing the immediate `#n.l` keeps the long form, `move.l #1.l,d0`; it is the one size suffix an immediate takes.
- An unsized branch is short when its target is above it and its displacement fits a byte and is not 0, which marks a word branch, and a word otherwise; `.s` and `.b` force a short one, and `.w` a word one. `.l` forces a word one too on the 68000, which has no long branch, as in EASy68K, and the 68020's long branch, six bytes, under `opt p=68020` (below, "Processor dialects"). An unsized absolute address is `.w` when it is above the line and a word holds it, and `.l` otherwise. "Above" counts a label on the line itself, and a number is always above.
- An immediate is kept at the size the instruction reads it at, so `move.w #-1,d0` holds `$FFFF`, and `exg a0,d0` is `exg d0,a0`.
- Under `opt p=68020`, a line the 68000 encodes keeps the 68000's encoding, and the 68020's wider forms are chosen only where the 68000 has none, by the same rule: a displacement above the line that the 68000's field does not hold, `d(An)` and `d(PC)` past a word or `d(An,Xn)` and `d(PC,Xn)` past a byte, takes the 68020's base displacement, a word or a long as it needs; an unsized branch back further than a word reaches is long, as vasm makes it. A name defined below the line keeps the 68000's field, and is an error when it does not fit, whose hint is to write the width, `(label.w,a0,d0)` or `bra.l`: without relaxation, a branch forward that may be long would be long every time. In the 68020's own forms, a base or outer displacement with no width is none for a known 0, a word or a long for a known value as it needs, and a long for a name below (below, "The 68020's addressing modes").

There are two passes and nothing in between, as in EASy68K: the first pass decides each instruction's size by these rules, and a name defined below the line is always given the long form, whatever its value, so the second pass encodes every instruction at the size it was laid out at. An instruction takes the bytes of its encoding, two to ten, or to 22 with the 68020's addressing modes, and starts on an even address. A branch whose displacement does not fit is an error, as in EASy68K: a short one out of -128 to 127 or to the very next instruction (a displacement of 0 is the mark of a word branch), and a word one or a `dbcc` out of -32768 to 32767. A PC-relative displacement is measured from its own extension word, which is two bytes past the instruction, and four for `movem`, `btst #n` and the 68020's instructions whose own extension word comes first, four or six for the 68020's `cmpi` against the program counter.

Three lines EASy68K refuses are refused here too, since the 68000 has no encoding for them: an address register added into memory (`add.l a0,(a1)`), a bit number written as an immediate on an immediate (`btst #1,#5`, where `btst d1,#5` is the instruction), and a `link` displacement outside -32768 to 32767.

## Processor dialects

A program is written for the 68000 unless it says otherwise, with `opt p=68010` or `opt p=68020` before its first instruction (ADR 0011). That is the Devpac spelling, which vasm reads too; `opt bit` keeps EASy68K's meaning, the 68020's bit-field instructions on a 68000, and `p=68020` implies it. The program carries what it chose, `program.getProcessor()` and the `processor` and `bitFields` of `program.getInfo()`, with `cyclesExact`, false for the 68020, whose cycles are an estimate (below, "The 68020's cycles"), and for a program under `opt bit` with a bit-field instruction, and its code decodes for that processor when it runs, so a word of a later processor is an illegal instruction in an earlier one's program, there as in `disassembleAt`. The assembly says it too, `processor` and `bitFields` beside `diagnostics` in what `S68k.assemble` answers, whether or not it built a program, so that an editor knows what a source with an error in it is written for.

- **Once, before the first instruction.** Every instruction is judged and measured for the processor in force, so an `opt p=` or `opt bit` after one is an error (`processor_chosen_after_code`) and chooses nothing. Saying the same processor again is harmless and naming another is an error (`processor_chosen_twice`), whichever file of the project either is in; a processor that is none of the three is `unknown_processor`.
- **One machine with gates.** Every form of the instruction table names the first processor that has it, `processor` in `S68k.instructionTable()`, and a form of a later processor is refused: `needs_a_later_processor`, whose hint gives the 68000's way of doing the same and the `opt p=` line to add. The diagnostic carries both as suggested fixes, `fixes` in TypeScript, the `opt p=` line first: each a title, a location and the text that replaces what the location covers, where a location with no columns inserts.
- **The 68010 is complete** (below).
- **The 68020 has every user-mode integer instruction** (below, "The 68020's addressing modes" and "The 68020's instructions"): the 68010's, the long branches `bra.l`, `bsr.l` and `b<cc>.l`, every addressing mode, and every instruction and every shape it gives a 68000 instruction, but `callm` and `rtm`, which are left out for good. A shape of a 68000 instruction in an earlier program, `muls.l` or `tst a0`, is `needs_a_later_processor` like an instruction of the 68020's own. It has the 68020's supervisor model, as a 68EC020 (below, "The 68020's supervisor model"), and counts its cycles as an estimate, the manual's cache case (below, "The 68020's cycles").
- **`opt bit`** gives a 68000 or a 68010 program the 68020's eight bit-field instructions, as EASy68K's option does, with that processor's addressing modes: `bfextu (8,a0,d1.w*4){0:8},d0` still needs `opt p=68020`. A bit-field instruction in an earlier program without it is `needs_a_later_processor`, whose second fix writes `opt bit`. Its code decodes for that processor and its bit fields.

### The 68020's addressing modes

`opt p=68020` gives a program every addressing mode of the 68020 (the M68000 Programmer's Reference Manual, 2.2.7 to 2.2.10 and 2.2.13 to 2.2.15), written in the Motorola syntax vasm and GNU `as` read (`docs/grammar.md` 2.5), wherever the 68000 takes `d8(An,Xn)` or `d8(PC,Xn)`:

| Form | Example | What it reaches |
|---|---|---|
| A scaled index, the brief extension word | `(4,a0,d1.w*4)`, `8(pc,d0.l*2)` | A0 + 4 + D1.W × 4 |
| A base displacement of a word or a long | `(1000,a0,d1.l)`, `(100000.l,a0)` | A0 + 1000 + D1.L |
| A base suppressed or left out | `(d0.l*4)`, `(table,d1.w*2)`, `(4,za0,d1)`, `(zpc,d0)` | D0.L × 4; `table` + D1.W × 2 |
| An index suppressed | `(a0,zd0)`, `(label.l,pc,zd0)` | A0 |
| Memory indirect, postindexed | `([4,a0],d1.w*4,8)` | the long at A0 + 4, + D1.W × 4 + 8 |
| Memory indirect, preindexed | `([4,a0,d1.w*4],8)` | the long at A0 + 4 + D1.W × 4, + 8 |
| The same from the program counter | `([table,pc],d0.l)`, `([zpc])` | reads in program space |

- **Any part may be left out**, the base, the index and either displacement, as the manual allows. A displacement's width is written after it, `.w` or `.l`, or chosen (above, "Encodings"); `(d.w,An)` and `(d.w,PC)` are the 68000's own `d(An)` and `d(PC)`. `za0` to `za7`, `zd0` to `zd7` and `zpc` are registers only inside the parentheses, where `pc` is one, and only where no Symbol has the name: a Symbol called `za0` keeps its meaning wherever it can stand, `(za0)` its address and `(za0,a1,d0)` a displacement, as in a 68000 program, which has no suppressed register, and a 68020 program is warned where the register could have stood too (`symbol_named_like_a_register`; `docs/grammar.md` 1.10). An index register with no size is `.w`, as on the 68000, where GNU `as` makes it a long.
- **The words** are the brief and the full extension words of M68020UM, the base displacement and the outer one after the full word. The decoder reads a 68020 Program's index word as the 68020 does, bits 10–8 as a scale and the mark of the full word, and as the 68000 does in every other Program; the combinations the manual reserves are read bit by bit, as Musashi reads them, and print as their words. The disassembler prints every form with its widths, and what it prints assembles back to the same words.
- **Running.** A memory indirect Operand reads its pointer, a long, as the program's read, in program space from the program counter; a pointer or an operand at an odd address is read as any other, as the 68020 reads data at any address.
- **Cycles.** Each mode takes the time of its row of the manual's effective address tables, by the widths of its displacements and whether it reads memory (below, "The 68020's cycles").
- **Long branches** `bra.l`, `bsr.l` and `b<cc>.l` take a 32-bit displacement, and an unsized branch back further than a word reaches is one.
- **Verification.** GNU `as -m68020` writes the same words for every form, 2,000 lines of them in the opt-in check (`S68K_BINUTILS_PREFIX=… cargo test --release gnu_as -- --ignored`), and the differential harness runs random Operands of every mode beside Musashi's 68EC020, `cargo test --release --features musashi the_68020s_addressing -- --ignored`: 39,930 cases agree, data at odd addresses among them, and 70 jumps to an odd address, the one exception these cases raise, are left out, as s68k's 68020 takes the address error and Musashi's, which has none on a 68020, does not; the runs of the supervisor model compare every other exception (below).

### The 68020's instructions

`opt p=68020` gives a program every integer instruction of the 68020 a user-mode program runs, from M68020UM and the Programmer's Reference Manual. Where Motorola leaves a flag undefined, or a result for registers that are one, s68k does what the 68020 does as WinUAE has it from Toni Wilen's cputester runs on the hardware (its `newcpu_common.cpp` and `gencpu.cpp`), which sometimes differs from the 68040 that Musashi models.

| Instruction | Forms | What it does |
|---|---|---|
| `chk.l` | `chk.l <ea>,Dn` | `chk` of the whole register against a long bound, vector 6 |
| `link.l`, `link.w` | `link.l An,#d`, `link.w An,#d` | `link` with a 32-bit displacement; and the 68000's `link` written with its size, as the 68020's manual, GNU `as` and vasm write it, which EASy68K refuses, so an earlier program is told it is the 68020's and offered `link` without the size |
| `tst`, `cmpi`, `cmp` | `tst.w a0`, `tst.l #5`, `tst.b label(pc)`, `cmpi.w #1,label(pc)` | the 68000's, of an address register (a word or a long), an immediate and the program counter |
| `trapcc` | `trapeq`, `trapne.w #n`, `trapgt.l #n`, the sixteen conditions with `hs` and `lo` | vector 7 when the condition is true, the stacked program counter past the operand, which only the handler reads (`ConditionalTrap`); with no size an operand is a word |
| `muls.l`, `mulu.l` | `<ea>,Dl` and `<ea>,Dh:Dl` | 32 bits by 32, into the low 32 with V when bits are lost, or into all 64; N and Z of what is kept; into `Dh:Dh` the high half wins |
| `divs.l`, `divu.l`, `divsl.l`, `divul.l` | `<ea>,Dq`, `<ea>,Dr:Dq` (64 by 32), `divsl.l <ea>,Dr:Dq` (32 by 32) | the quotient into Dq and the remainder, of the dividend's sign, into Dr; a quotient past 32 bits sets V and leaves the registers; 0 is vector 5 |
| `pack`, `unpk` | `pack d0,d1,#adj`, `unpk -(a0),-(a1),#adj` | two digits of a word plus the adjustment into a byte, or a byte's two digits spread into a word plus the adjustment; in memory `pack` reads the word below Ax, which moves by two even for A7, and `unpk` writes one below Ay; no flag changes |
| `chk2`, `cmp2` | `chk2.b bounds,a0`, `cmp2.w (a0),d1` | the register against the lower and the upper bound at the operand: a data register at the size, an address register whole against the bounds sign extended; signed when the lower bound is not above the upper, and otherwise the unsigned range that wraps round. Z says it is on a bound, C outside; `chk2` traps through vector 6 there (`Chk2OutOfBounds`) |
| `cas`, `cas2` | `cas.w d0,d1,(a0)`, `cas2.l d0:d1,d2:d3,(a0):(d4)` | compare, then swap when equal or load the compare registers when not, both operands of `cas2` or neither; one register for both compare values keeps the first operand |
| Bit fields | `bfextu d0{4:8},d1`, `bfins d1,(a0){d2:d3}`, `bfffo (a0){0:32},d0` | a field of a data register, from bit 31 down modulo 32 and wrapping round, or of memory, from the byte the operand names plus the offset over eight, any distance either way, one to five bytes read and written; N and Z of the field, or of the value `bfins` inserts; `bfffo` answers the offset as written plus how far in its first set bit is |

- **Syntax.** A register pair is `d1:d2`, and `cas2`'s two addresses `(a0):(d1)`, an address or a data register in each; a bit field is `{offset:width}` straight after its operand, each half a number, with a `#` or without, or a data register; an offset is 0 to 31 and a width 1 to 32 (`docs/grammar.md` 2.5). A data register holds an offset of any size and a width modulo 32.
- **`cas` and `cas2`** read and write in one step, which nothing can come between: s68k has one processor and no bus to lock.
- **Cycles.** Each takes the manual's cache case (below, "The 68020's cycles"). The bit fields of a 68000 program under `opt bit` take EASy68K's: the 68020's cache case for the field and the 68000's effective address time, more for a field in five bytes. No 68000 or 68010 has them, so that time is no processor's, and such a program that has a bit-field instruction says its count is an estimate, `cyclesExact` false, though every other instruction of it takes its exact time.
- **Verification.**
  - GNU `as -m68020` writes the same words for all 550 lines of the 68020's forms, `extb.l` and `link.w` among them, in the opt-in check (`S68K_BINUTILS_PREFIX=… cargo test --release gnu_as -- --ignored`).
  - The differential harness runs every first word the 68020 has and the 68010 does not, 64 random cases each, beside Musashi's 68EC020, from states of the 68020's supervisor model, its exceptions and frames compared (`cargo test --release --features musashi the_68020s_instructions -- --ignored`): 70,612 cases agree. Left out, by the list in `src/test/differential.rs`, the supervisor model's gaps (below, "The 68020's supervisor model"), and where Musashi is not the 68020: it sign extends `cas2.w`'s compare value through an address register, reads `pack`'s bytes the other way round, compares `chk2`'s address register at the size and its bounds signed only, misplaces a bit field at an offset within eight of −2³¹, takes `bfffo`'s register offset modulo 32, keeps the second operand in one compare register, and leaves its own undefined flags.
  - Musashi and the decoder agree on which of the 65,536 first words are 68020 instructions, but for `callm`, `rtm`, `illegal` and the line F words a 68020 with no coprocessor takes as line F.
  - Seventeen of Musashi's self-checking programs for the 68040 run as 68020 programs and pass (`tests/musashi/mc68040`): the bit fields, `cas`, `chk2`, `cmp2`, the long multiplies and divides, `trapcc`, `rtd`, `jmp` and `shifts3`. Their checks of the 68040's undefined flags after a division, and of `chk2` and `cmp2` where they rely on Musashi's own reading, are relaxed, each with its reason (`src/conformance/skips.rs`).

### The 68020's supervisor model

`opt p=68020` gives a program the supervisor model of the 68EC020, the 68020 with the 68000's 24-bit bus (M68020UM, sections 1, 2.3, 6 and 7), with no coprocessor and no cache:

- **The status register** has T1, T0, S, M, the interrupt mask and the condition codes, so `move #$FFFF,sr` reads back `$F71F` where a 68000 and a 68010 read `$A71F`. T1 is the 68000's T, and traces every instruction. T0, with T1 clear, traces an instruction that changed the flow of the program: a branch taken, `bra` and `bsr` always, `jmp`, `jsr`, `rts`, `rtr`, `rtd` and `rte`, and a write of the status register, `move` to SR and `andi`, `ori` and `eori` to SR or to CCR, as WinUAE's 68020 has it from cputester. An instruction that raises an exception is not traced by T0, as taking it clears T0, and a `stop` stops; T1 traces `stop`, which then does not stop. The trace frame is format `$2`, with the address of the instruction traced.
- **Three stack pointers.** A7 is the user stack pointer while S is clear, and while it is set the interrupt stack pointer, ISP, while M is clear and the master stack pointer, MSP, while M is set. The ISP is the 68000's SSP: a program starts with it at `$01000000` and M clear, and one that never sets M runs as on a 68010. Every write of SR, every exception and `rte` can swap them, and each keeps its own stack top for the memory view (`getStackTop()`).
- **An interrupt in the master state** pushes its format `$0` frame on the MSP, clears M and pushes the throwaway frame, format `$1`, on the ISP: the same program counter and vector, and the status register it interrupted with S set, as Musashi and WinUAE push it. The handler runs on the ISP; its `rte` pops the throwaway frame, whose status register, S and M set, makes A7 the MSP again, and goes on to the frame there.
- **`movec`** reaches `msp`, `isp`, `cacr` and `caar` beside the 68010's four, codes `$803`, `$804`, `$002` and `$802`: a stack pointer is A7 when it is the one A7 is. CACR keeps E and F, bits 0 and 1; C and CE, bits 3 and 2, are commands to clear the cache, which read as 0, and s68k has no cache, so code a program writes always runs as written. CAAR keeps all 32 bits. On a 68010 the four codes are illegal instructions, and the four names are reserved in every program; an earlier program that names one is told it is a 68020 control register.
- **Frames** (M68020UM 6.5). Format `$0`, eight bytes, for an interrupt, `trap #n`, an illegal instruction, line A and F, a privilege violation and the format error; format `$2`, twelve bytes, which adds the instruction's address after the format word, for `chk`, `chk2`, `trapcc`, `trapv`, a division by zero and trace. A bus or an address error pushes the long bus fault frame, `$B`, 92 bytes, but a bus error on the fetch of an instruction none of which ran, at an instruction boundary, which pushes the short one, `$A`, 32 bytes; the address error of an odd program counter is `$B`, as WinUAE's 68020 pushes it. As with the 68010's `$8`, they are simplified: the program counter is the instruction's, the special status word and the data cycle fault address at `+$0A` and `+$10` are real (DF for a data access, RW for a read, SIZ and the function code; a fetch is a read of a word in program space), and every internal word is 0, the version number of `$B` at `+$36` among them.
- **`rte`** pops format `$0`, `$2`, `$A` and `$B`, the last two returning to the instruction that faulted, which runs again from its start, once `$B`'s version is 0; `$1` it pops and goes on to the frame of the stack its status register names. Any other format, the 68010's `$8` among them, is the format error, which stacks the `rte` and leaves the flags, as WinUAE's 68020 does.
- **Unaligned data.** A word or a long at an odd address is read and written on every path into memory: an operand of any mode, the stack and the frames, `movem`, a pointer of a memory indirect Operand, the memory map's checks, a write into code, a watched range, round the end of the bus at `$FFFFFF`; only an instruction fetched from an odd address, a jump or a return to one, is an address error. The check of an odd address asks for the processor only once it finds one, so an even access costs a 68000 program what it did.
- **Line F** with no coprocessor is vector 11, format `$0`, stacking the instruction: every word of `$F000` to `$FFFF`, the FPU's and the coprocessor's among them, decodes as line F, one word long, but EASy68K's `simhalt`, `$FFFF $FFFF`.
- **The System register file** of a 68020 program, `getSystem()`, adds `isp`, `msp`, `cacr` and `caar`; `ssp` is the ISP under its 68000 name. `setIsp`, `setMsp`, `setCacr` and `setCaar` poke them, and throw `InvalidArgument` in an earlier program. The history's `WriteStackPointers` carries `{usp, ssp, msp}`, `msp` being 0 before the 68020, and `a7`, which of them A7 is after the change, in order where an `rte` through a throwaway frame swaps twice; and `WriteControlRegister` `cacr` and `caar`.
- **Verification.** The hand tests cover every format, T1 and T0, MSP and ISP and undo across them, `movec`, `rte`'s checks and odd accesses on every path (`src/test/m68020_supervisor.rs`). The differential harness runs Musashi's 68EC020, which models the 68020's frames, its three stack pointers and M, from states with T1, T0, S and M random and a master stack of their own (`src/test/differential.rs`):

  ```sh
  cargo test --release --features musashi the_68020s -- --ignored --nocapture
  ```

  On 2026-10-29: the 68010's instructions on the 68020, two cases of each first word, 96,980 agree, their exceptions, frames, tracing and odd accesses among them; the 68020's own, 64 cases each, 70,612; 20,000 interrupts, 10,007 of them in the master state, with their throwaway frames; and 20,000 `rte`s of frames both pop, 6,678 through a throwaway frame. Left out, by one list with a reason per line (`RELAXATIONS_SUPERVISOR`), beside the 68010's and the 68020's instructions' own: Musashi has no address error on a 68020, so a jump or a return to an odd address is not compared; it forgets a pending trace when an instruction takes an exception; under T0 it traces `nop`, `movec`, `moves`, `move` to USP, `stop`, `cas` and `cas2`, as the 68040 does, does not trace `andi`, `ori` and `eori` to CCR, and of the writes of SR alone does not trace `move Dn,sr`; it runs the line F words as an FPU's and a coprocessor's instructions; it raises the format error on `$A` and `$B` and stacks the address after `rte` for one; it keeps CACR's C and CE; and the flags the 68020 leaves undefined after `chk`, `chk2` and a long division, which s68k sets as WinUAE's 68020 does, are left out of the status registers its frames stack too.

### The 68020's cycles

A 68020 program counts an estimate: the **cache case** of the timing tables of the M68020 User's Manual (section 8 of the 1995 edition, 9 of the 1984 one, whose values are the same), the instruction in the cache and no overlap with the instructions around it (the asm-editor's `docs/design/m68k-machine.md`, decision 3; `src/machine/timing.rs`). The manual gives every row three times, a best case, the cache case and a worst case, and says that exact times cannot be worked out from the tables, as they depend on the instruction cache, the prefetch and how neighbouring instructions overlap. The cache case is the deterministic middle, and the column Musashi's 68020 keeps. `program.getInfo().cyclesExact` is false for a 68020 program, `program.hasExactCycles()` in TypeScript, so that a host can show its count as an estimate.

- **How a time is made up**, as the manual's notes say: the time of the operation, and the fetch, calculate, immediate or jump effective address time of its operand, with an extension word's time for an instruction that has one, `movem`, `chk2`, `muls.l`, the bit fields and their kin. `move` has its own table: two cycles, the source's fetch, and the destination's time, less after a source in memory, whose write overlaps the read. Every mode has its row: the brief extension word with a scale is `d8(An,Xn)`'s, and the full one's rows go by the widths of its displacements and whether it reads memory, its base and index being there or not, the index's size and its scale changing nothing. Operands are taken as aligned, as the tables assume, and there are no wait states.
- **The data counts** only where the manual's times depend on it: `Bcc.s` 4, and 6 when it branches; `Bcc.w`, `Bcc.l` and `bra` 6; `DBcc` 6, and 10 when its counter runs out; a bit field in five bytes of memory, more than in four; `cas` 12 and `cas2` 22, 3 more when they write; `rte` 21, 43 of the short bus fault frame, 92 of the long one and 16 more through a throwaway frame. A shift or a rotate takes the same by any count, `mulu.w` 27, `divu.w` 44, `divs.w` 56, `mulu.l` 43, `divu.l` 78 and `divs.l` 90 whatever their operands, and `movem` 8 and 4 per register from memory, and 4 and 3 per register into it.
- **Exceptions** take the manual's times in place of the instruction's: 20 for `trap #n`, an illegal instruction, a privilege violation and the line A and F words; 25 for `trapcc` and `trapv` that trap; 25 for a trace exception, after the instruction; 26 for an IRQ, and 41 in the master state, with its throwaway frame; 43 and 79 for a bus or an address error that pushes the short or the long bus fault frame. `stop` takes 8 and `reset` 518, and `trap #15`'s tasks none, as on the 68000.
- **Where the manual gives no time**, s68k chooses (`machine::timing`'s `m68020`): `chk`, `chk2` and a division by zero take `trapv`'s 25 and their source's effective address, as on the 68000 they take their trap's time and theirs; `bkpt` its breakpoint cycle, 10, and the illegal instruction's 20; the format error `rte`'s 21 and an exception's 20; a bus or an address error its frame's time alone, the manual giving none for what the instruction did before it; and `nbcd` of memory Musashi's, 6 and the fetch. Where the manual is not consistent: a full extension word with a word displacement takes the `(d16,B)` row, as Musashi does, not the `(d16,An,Xn)` row, whose times are lower for the same word; `-(An)`, which the calculate immediate table leaves out, takes `(An)+`'s, as in the calculate table; and `([d16,B],I,d32)`, which the fetch immediate table leaves out, takes `([d16,B],I,d16)`'s, as every other outer long takes its word's.
- **Verification.** Unit tests check every cell of the manual's MOVE table, the effective address tables through `tst`, `cmpi`, `lea`, `jmp` and `movem`, rows of every other table and the exceptions, and programs that run them (`src/machine/timing.rs`, `src/test/cycles.rs`). Musashi's 68EC020 keeps the cache case too, one number per opcode and a table per mode, and the differential harness reports how the two compare rather than asserting it:

  ```sh
  cargo test --release --features musashi the_68020s_cycles -- --ignored --nocapture
  ```

  On 2026-10-29, 92,080 of 134,646 cases agree: 76,833 of 96,980 of the 68010's instructions on the 68020, two cases of each first word; 6,481 of 17,703 of the 68020's own, sixteen each; and 8,766 of 19,963 of the addressing modes in `lea`, `move`, `add`, `clr` and `jmp`. Where they differ, Musashi departs from the manual: it adds its fetch time where the manual adds the calculate or the jump time (`lea`, `pea`, `jmp`, `jsr`, `clr`, `Scc`, `tas`, `move` from SR, `movem`, `moves`, `cas` and the bit fields); it leaves out the immediate of `addi` and its kin to a register, and the extension word of `muls.l`, `divu.l`, `chk2` and `cmp2`; it adds both effective addresses of a `move` from memory to memory, where the manual overlaps them; it takes a cycle per bit of a rotate, 10 for `bra`, 8 for `move sr,Dn`, 4 per register of `movem` into memory, 12 for `cas2`, and 84 for every long division; and it takes 20 for `trapcc` and `trapv` that trap and 20 for `bkpt`, where the manual has 25 and 30, and its 68000 times, 40, 38 and 34, for `chk` out of bounds, a division by zero and a privilege violation.

### The 68010

`opt p=68010` gives a program the whole 68010 of the 68000 User's Manual (its sections 6 and 9 and appendix A) and the Programmer's Reference Manual:

- **Its instructions**: `move ccr,<ea>`, the user mode's way to read the condition codes; `rtd #d`, a return that adds the displacement to the stack pointer; `bkpt #n`, which takes the illegal instruction exception, as a 68010 does when no debugger answers its breakpoint cycle; `movec` between a general register and `sfc`, `dfc`, `usp` or `vbr`, privileged, any other control register an illegal instruction; and `moves`, privileged. s68k has one address space, so `moves` reaches the memory `move` does, as a supervisor data access, whatever SFC and DFC hold: they are three bits each that `movec` writes and reads back. `sfc`, `dfc` and `vbr` are reserved names in every program, as `usp` is.
- **`move sr,<ea>` is privileged**, the one change a user-mode program sees; `clr` and `move sr` no longer read before they write.
- **VBR.** Every exception, IRQs included, reads its vector at VBR plus four times the vector; VBR is 0 until `movec` moves it.
- **Frames.** Every exception but the two of group 0 pushes the format `$0` frame, eight bytes: the status register, the program counter and the format/vector-offset word, four times the vector. The bus and address errors push the format `$8` frame, 29 words, of which the three reserved words are not written: the status register, the program counter of the instruction that faulted, `$8` and the vector offset, the special status word (IF, DF, BY, R/W and the function code), the fault address, and the three buffers and the sixteen internal words as zeros, version 0 among them.
- **`rte`** reads the format word first: format `$0` pops eight bytes and format `$8` 58, and returns to the stacked program counter, where the instruction that faulted runs again; s68k keeps no internal state to continue it. Any other format, or a version that is not 0, is the format error, vector 14, which leaves the frame where it is, stacks the `rte`'s address and, for a format it does not have, sets N to the format's high bit and clears Z and V, as a 68010 does.
- **Cycles** are the manual's section 9, and Moira's where the manual gives a maximum or departs from it (`src/machine/timing.rs`): `mulu` 40 and `muls` 40 or 42 with the source, `divu` 78 and two per step without a carry, `divs` 118 or 120, 8 and 16 for an overflow; an exception 38, an IRQ 46, `chk` 42 or 44, a division by zero 42, `trapv` 40, `bkpt` 42, `rte` 24, and an address or bus error 134 from the access, after the 68010's own lead; `Bcc.s` 6 not taken and 10 taken, `DBcc` 10, or 16 when its counter runs out.
- **Loop mode.** A `DBcc` with a displacement of −4 that branches back to a one-word instruction the 68010 can loop, with T clear, enters loop mode, and the loop's next turns fetch nothing: `move.w (a0)+,(a1)+` takes 8 and `dbra` 6 where they took 12 and 10. It ends when the counter runs out or the condition is true, at the time of its own the instruction gives it, on any exception, an IRQ's included, and on a write into either instruction of the loop, the program's, a trap task's or a Poke's, after which the loop runs as its words are decoded again, out of loop mode until its `DBcc` enters it again; undo puts it back. It changes the cycles and nothing else.

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
| `memory` | declares a range of EASy68K's memory map, `memory rom $8000,$8FFF`: `rom`, `read`, `protected` or `invalid`, each kind once; the program carries it, and the interpreter enforces it (above, "The memory map") |
| `opt` | `opt p=68000`, `p=68010` or `p=68020` chooses the processor the program is written for, and `opt bit` the 68020's bit-field instructions, once and before the first instruction (above, "Processor dialects"); EASy68K's listing options `cre`, `mex`, `sex`, `war` and `cex`, and each with `no`, are accepted and ignored, and any other word is a warning |
| `list`, `nolist`, `page` | accepted and ignored: they are about the listing file, which there is none of |

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

### The cycle counter: tasks 30 and 31

Task 30 clears the cycle counter and task 31 reads it into D1.L, 0 once it is past 32 bits, as in EASy68K (above, "Cycles"). They raise no interrupt and take no time; undo puts the counter back.

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

An error the 68000 has an exception for goes through its vector (above, "Exceptions"), and ends the program only when the vector holds no handler: the status is `TerminatedWithException`, the error is thrown, and `getTermination()` answers it as the cause, with the vector. The trap tasks raise two errors of their own: `UnsupportedTrapTask {task}`, for a number that is none of EASy68K's tasks, which goes through vector 47 as in EASy68K, and for the tasks a page in a browser cannot do faithfully (10, 21, 22, 25, 32 with 0 to 3 or 7 in `D1.B`, 40 to 43, 100 to 107), which end the program whatever vector 47 holds; and `InvalidTrapArgument {task, reason}` for a value a task cannot take, the reason naming the register, which ends it too. Undoing the step that failed brings the program back to it, running.

`getTermination()` is `null` while the program runs, and then says why it ended: `TerminateTask` (task 9), `EndOfProgram {address}` (it ran into `$FFFF $FFFF` it never placed, above), `TerminatedByHost` (an interrupt answered with `Terminate`), `Exception {cause, vector}` with the error, or `Irq {level}` for an interrupt request, and the vector that held no handler, when it had one, or `Halted {cause, vector}`, a double fault.

An answer is taken only by the interrupt of the same `type`, with a value the task can take, or it is `Terminate`, which answers any task and ends the program. Anything else is refused with a typed error and leaves the interrupt waiting: `NoPendingInterrupt`, `InvalidAnswer {interrupt, reason}` for an answer of another type, a value that is not an answer at all, a key state of the other form, more bytes than a read asked for or a file number past 7, and the `OutOfBounds` of a write that does not fit in memory. Every WebAssembly method throws a `RuntimeError` object and none of them panics, so the interpreter stays usable after a bad call.

Where s68k departs from EASy68K, on purpose:

- A base outside 2 to 36 for task 15, a D1.B other than 0 to 3 for task 16 and a D1.L other than 0 and 1 for task 58 stop the program with `InvalidTrapArgument`; EASy68K does nothing (and task 58 reports success).
- A task s68k does not carry out stops the program with `UnsupportedTrapTask`, where EASy68K carries it out.
- A D1.W of `$8000` or more for tasks 0 and 1, which EASy68K reads as a negative index into its buffer, displays up to 255 characters, as for any D1.W above 255.
- Overflow in `atoi` wraps modulo 2^32; EASy68K's own `atoi` (Borland's runtime) was not available to check.
- The characters typed after the 79th of a line are dropped. EASy68K ends the line at the 80th key and keeps the next one for the following read.
- A read of no bytes (task 53 with D2.L = 0) reports 2. EASy68K's `fread` reports 1 instead once an earlier read has reached the end of the file, which is C's end-of-file indicator, and s68k does not keep one.
- A count that runs past the end of memory is 2 also when (A1) + D2.L passes 4 GB, where EASy68K's 32 bit check wraps round and reads past its memory; task 58 checks the whole 256 bytes it writes where EASy68K checks only the name.
- A path is sent with `/` for `\`.
- On Windows EASy68K cannot delete a file that is open, and reports 2; whether a file that is open can be deleted is the host's file system's to say.

## Known limitations
1. Cycles are counted with a bus that answers at once: no wait states. The 68020's are an estimate, the manual's cache case, with no instruction cache, prefetch or overlap of neighbouring instructions, and its operands taken as aligned (above, "The 68020's cycles").
6. A step keeps ten bytes of the instruction it ran, the 68000's longest, so a step that ran a longer 68020 instruction decoded from memory, code the program wrote or reached in its data, says nothing of what ran (`ran` is absent); one the assembler placed says it. Keeping 22 would cost every step that keeps a history.
4. The bus and address errors of the 68010 and the 68020 are restarted by `rte` from the instruction's start, not continued from the internal state the processor writes, which s68k writes as zeros, and the 68020's choice between its short and long bus fault frames is s68k's rule, not the pipeline's; the 68020 has no instruction cache, so CACR's E changes nothing, and no coprocessor; loop mode changes cycles only, so an IRQ is still taken between the looped instruction and its `DBcc`, where a 68010 takes it after the `DBcc`; and SFC and DFC name no address space.
2. Vectored and spurious interrupts, the reset exception and the 68000's HALT pin are left out, as EASy68K leaves them out; nothing reads the vectors at `$0` and `$4`, and `reset` runs no reset sequence.
3. The timers of task 32 are the host's to run: the interpreter keeps their settings and takes the requests the host raises. EASy68K's hardware window, its IRQ buttons and its devices are not there.

## Running the command-line interpreter

Install [Rust](https://www.rust-lang.org/tools/install), clone the repository,
and run `cargo run` from its root. This assembles and runs `code-to-run.asm`.
To run another file, pass its path: `cargo run -- my-program.asm`.

The command line answers the text tasks on the terminal, honouring the echo of task 12, and the file tasks on the disk: a path is taken relative to the directory of the file it runs, task 51 opens a file it cannot write for reading only, and the dialog of task 58 asks for a path on the terminal, an empty line cancelling it. The graphics, mouse and sound tasks are reported and end the run, and the last line says why the program ended. It raises no interrupt request, so a program that stops with `stop` ends the run there, waiting.

## Building the WebAssembly package

Install [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/), then run
`npm run build-wasm` in the `ts-lib` directory. This creates the compiled
package in `ts-lib/src/pkg`.
