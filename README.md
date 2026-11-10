# s68k

s68k is an M68K assembler and interpreter written in Rust. The assembler reports errors and warnings to help with learning assembly and encodes instructions in memory as EASy68K does. The interpreter runs the assembled program and supports stepping, undo, and breakpoints.

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
syntax, addressing modes, and directives. It can write programs as S-record
files in the format produced by EASy68K's assembler and loaded by its simulator.
It is not intended to produce binaries for real 68000 systems.

## Architecture

The library has four parts:

- **Assembler** (`src/assembler/`): reads project source files starting from the entry file. It reports all diagnostics it finds and, if there are no errors, returns a program. It contains a tokenizer, parser, symbol table, expression evaluator, layout pass, instruction table and analyzer.

- **Program**: the assembler's output. It contains assembled instructions with their addresses, sizes, encodings and source locations, along with initial memory, symbols and the entry point.

- **Interpreter** (`src/interpreter.rs`): runs a program from memory using registers and the status register. It supports stepping, running to the end, undo history, breakpoints and a call stack. A watched memory range supports a memory-mapped device: every store to the range ends the run.

- **Machine** (`src/machine/`): handles processor encodings. The decoder reads instruction words for the 68000, 68010 and 68020, the encoder writes them, and the disassembler prints EASy68K syntax. The decode cache holds instructions for the interpreter. Exception frames and cycle counts follow the selected processor.

The assembler chooses each instruction's 68000 encoding as EASy68K does (below) and writes it into the program's memory at its real size. Addresses therefore match those of a 68000 and EASy68K's listing. The interpreter runs those bytes (below, "Code is memory").

## Code is memory

The interpreter runs the bytes in memory through a cache of decoded instructions. The cache uses 4 KB pages with one slot per word, allocated only where code exists. When the interpreter is created, the cache is filled from the assembler's instructions, so that code does not need decoding while it runs.

Every write passes through the cache, including writes by the program, trap tasks, the host and undo. A write clears the instructions it covers, which are decoded from their new bytes before they run. A program can therefore read and change its own code, as on a 68000. Undo restores the old bytes, so the old instruction runs again.

- **A code write** occurs when a program store or a trap task's answer writes into an instruction that has been decoded. Each step reports the first instruction overwritten (`wroteCode` on the step, `getCodeWrite()`), giving its address and line index. With `runWithBreakpoints(…, {stopOnCodeWrite: true})`, a run stops after a code write with status `CodeWrite`; the next run continues. A Poke, a testcase's starting memory and undo never report a code write.
- **Source locations** come from the program's map of the lines it laid out. The map does not move when bytes change: a changed line keeps its location, breakpoint and `getInstructionAt` result as assembled. `disassembleAt(address)` instead prints the current contents of memory. `getInstructionAt` also reports an assembler-selected form that differs from the source (`assembledAs`): `add.l #1,d0` is `ADDQ.L #1,D0`, `bra loop` is `BRA.S loop`, and `lea $1000,a0` is `LEA $1000.W,A0`.
- **The 24-bit bus.** Memory covers the 16 MB reachable by the 68000's 24 address lines. Addresses use their low 24 bits, as in EASy68K's `& ADDRMASK`. An access past `$FFFFFF` wraps to `$000000`: this applies to a long at `$FFFFFE`, a stack pushed below `$000000`, instruction words crossing the end, strings read and lines written by trap tasks, and host reads and writes. File task buffers are the exception, as in EASy68K: a read or write past the end returns 2 (below). The map, decode cache and watched range see the bytes at their wrapped addresses. Only a run of bytes longer than the whole 16 MB is refused as out of bounds.
- **The end.** `$FFFF $FFFF` is EASy68K's SIMHALT wherever it occurs. Within bytes placed by the program, whether code or data, it pauses execution like `simhalt`. Elsewhere it ends the program with `EndOfProgram` and the address. Memory starts as `$FF`, so a program that does not end itself stops there, one past its last instruction if nothing follows it. Other bytes execute as the instructions they decode to: execution that reaches or jumps into data runs that data, and a word that is not an instruction raises an illegal instruction exception. Fetching an instruction at an odd address raises an address error; a jump to an odd address raises it in the jump itself, as on a 68000.

## Exceptions

The machine follows the 68000, including where EASy68K differs from it. Every exception is taken through the vector table at `$000` to `$3FF`:

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

The 68010 uses the same table and adds `bkpt` and `movec` of a register it does not have as causes of vector 4. An `rte` format error uses vector 14 at `$038`. The processor reads vectors through VBR, pushes 68010 frames, and stacks the faulting instruction's address for address and bus errors (below, "The 68010").

The 68020 has the 68010's exceptions, adds `chk2` and `trapcc` alongside `chk` and `trapv`, and uses 68020 frames. It permits data reads and writes at odd addresses, so its only address error is an instruction fetch at an odd address (below, "The 68020's supervisor model").

- **Taking an exception.** The processor reads the vector, copies the status register, sets S, clears T, pushes a frame on the supervisor stack and jumps to the handler. A normal frame is six bytes: the status register followed by the program counter. A bus or address error frame is 14 bytes: the special status word, access address, instruction register, status register and program counter. The special status word contains bits 15 to 5 of the instruction register, R/W, I/N clear, and the function code for the mode and space of the access. `rte` pops six bytes; a bus or address error handler removes the other eight itself.
- **No handler.** An odd address in a vector means there is no handler. The program ends with the error and vector in `Exception {cause, vector}`. Memory starts as `$FF`, so each vector reads `$FFFFFFFF` until the program writes it. A program that installs no handler therefore ends when an exception is raised.
- **A double fault** halts the processor with `Halted {cause, vector}`. This happens if a frame is pushed on an odd supervisor stack pointer or into a range refused by the memory map, if a vector is read from an invalid range, or if the map refuses to fetch a bus or address error handler. Other exception handlers are fetched by the next step. If that fetch causes a bus error and its handler cannot be fetched, the processor halts in turn.
- **Supervisor and user mode.** A program starts in supervisor mode with EASy68K's registers: SR `$2000`, A7 as the supervisor stack pointer at `$01000000`, and the user stack pointer at `$00FF0000`. A7 holds the stack pointer for the mode selected by S. A write to SR, an exception or `rte` can swap the two. `getSystem()` returns `{sr, usp, ssp}`; `setSr`, `setUsp` and `setSsp` write them and are journaled inside a poke. The 68020 has a third, master stack pointer selected by M (below, "The 68020's supervisor model"). On a 68000, `move sr,<ea>` is not privileged and `trap #15` works in user mode, as in EASy68K.
- **Trace.** T is sampled before an instruction. If it was set, a trace exception follows the instruction unless it raised an illegal instruction, privilege violation, line A or F, or address error and did not finish. After `trap`, `trapv`, `chk` or division by zero, the trace frame is pushed on top of that exception's frame. The trace handler runs first and returns to the other handler, as on a 68000; EASy68K pushes the frames in the opposite order. A `trap #15` task that waits for the host is traced after it is answered.
- **Address error PC and side effects.** These follow the 68000 behavior left by MAME's microcode for the 55,606 address errors in the conformance suite. A jump to an odd address faults in its own prefetch: `bsr` has pushed its return address, but `jsr` has not. A data access stacks the address reached by prefetch, which depends on the operands. A word access through `(An)+` has moved the register; a long access has not. `clr` and `move sr,<ea>` read their destination first, so an odd destination faults as a read. The rules are in `src/machine/exception.rs`.
- **The call stack** gains a frame for each exception, with `kind: 'exception'`, its `vector`, the error name in `cause`, and the vector base register used in `vbr`. Like a call's frame, it is gone once the stack pointer of the stack it was pushed on is above it, which `rte` does, and so does an `rtr` that pops it. The step records `PushException` (with the same `cause` and `vbr`) and `PopException`, and stack swaps through the `WriteStackPointers` mutation (which identifies the stack pointer A7 became). Undo reverses all of it.

## Interrupts

The 68000's interrupt requests, IRQs, are EASy68K's without its hardware window: seven levels, all autovectored, from timers, the keyboard and the mouse.

- **Latched.** A request at level 1 to 7 waits in a latch until it is taken. A second request at an already pending level is the same request.
- **Taken between instructions.** Before the next instruction, the processor takes the highest pending level above the status register's interrupt mask. Level 7 is taken regardless of the mask. The autovector is vector 24 plus the level, at `$60` plus four times the level. The exception pushes a six-byte frame on the supervisor stack, sets S, clears T and raises the mask to the level. A handler can therefore be interrupted only by a higher level or level 7. Taking an IRQ is a separate execution step with `kind: 'irq'` and no location; History shows it and Undo reverses it on its own. An autovector with no handler ends the program with `Irq {level}` as the cause and identifies the vector.
- **Other exceptions.** As on a 68000, an instruction's trace frame and the frame for `trap`, `chk`, `trapv` or division by zero are pushed first, and the IRQ handler runs before their handlers. An illegal instruction at the program counter has not yet been fetched, so it raises its exception after the IRQ handler's `rte`. A request arriving while a `trap #15` task waits for the host is taken after the answer. EASy68K loses an interrupt taken beside a trap and runs an illegal instruction twice.
- **`stop #n`** is privileged and loads the whole status register, including T and S. The processor waits for a request above the new mask, with status `Stopped` and the program counter at the next instruction. While it waits, `step()` and every run return `Stopped` immediately. A request that can wake it changes the status to `Running` immediately, and the next step takes the request. `getMachineState().stopped` is true until then. If a request above the new mask is already pending, `stop` does not wait. If it starts with T set, it takes the trace exception instead of stopping, as on a 68000. With S clear in the immediate, the processor stops in user mode; EASy68K raises a privilege violation instead. EASy68K also runs one more instruction before the interrupt that wakes it.
- **`reset`** is privileged and takes 132 cycles but does nothing else (below, "Cycles"). There are no devices behind the reset line, and nothing runs the 68000's reset sequence.
- **The sources.** The program configures interrupt sources and the host delivers their requests:

  | Task | `D1` | What it sets |
  |---|---|---|
  | 32 | `D1.B` = 6, with `D2.B` and `D3.L` | the timers, EASy68K's `setAutoIRQ`: `D2.B` = 0 turns all off, 1 to 7 turns that level's off, `$81` to `$87` fires that level every `D3.L` milliseconds; any other `D2.B` does nothing |
  | 60 | level in bits 15 to 8, events in bits 0 to 2 | the mouse's: a button down (bit 0), a button up (bit 1), a move (bit 2), each chosen one set to the level, 0 turning it off; a level above 7 makes the call do nothing |
  | 62 | the same, bits 0 and 1 | the keyboard's: a key down (bit 0) and a key up (bit 1) |

  For `D1.B` = 4, task 32 also returns EASy68K's version, `$00051001`, in `D1.L`; it accepts 5 as well. Values 0 to 3 and 7 request the hardware window and its devices' addresses and are refused with `UnsupportedTrapTask {task: 32}` because there is no hardware window. Any other `D1.B` does nothing, as in EASy68K.
- **The host calls.** `getMachineState()` returns `{pendingIrqs, stopped, autoIrqs: {level, intervalMs}[], keyboardLevels: {keyDown, keyUp}, mouseLevels: {down, up, move}}`. A level of 0 means none. The host runs timers on the program's time and calls `raiseIrq(level)` at each firing. It reports screen events with `deviceEvent('keyDown' | 'keyUp' | 'mouseDown' | 'mouseUp' | 'mouseMove')`, which latches and returns the level configured by the program, or `null`. Both calls provide input between instructions, like a key press, and do nothing after the program ends.
- **Undo.** The program's configuration and the stopped state are journaled like registers in a `SetMachineState {old, new}` mutation. Undo restores them. The latch also contains host input: undoing an IRQ step makes its request pending again, so continuing takes it again. A request arriving after a step remains pending when that step is undone, since the program never saw it.

## The memory map

A program declares ranges of EASy68K's memory map with `memory`, one of each kind at most, and the interpreter enforces them as EASy68K's simulator does (`memoryMapCheck`, `mem_put` and `mem_req` in its `UTILS.CPP`):

| Kind | A read | A write |
|---|---|---|
| `rom` | allowed | ignored: memory keeps what it held, and it is no error |
| `read` | allowed | a bus error |
| `protected` | a bus error from user mode | a bus error from user mode |
| `invalid` | a bus error | a bus error |

- **Checks.** Reads are checked against invalid and protected ranges by their first byte. Writes are checked against all four kinds by every byte. A long that starts below a range and extends into it is therefore refused as a write but not as a read. Each instruction word is checked as it is fetched, as EASy68K fetches each word as a read. Code in an invalid range, or in a protected range in user mode, raises a bus error in program space. The kinds are checked in EASy68K's order: invalid, protected, read only, then ROM. Address errors take precedence over the map.
- **Bus errors** use vector 2 and the 14-byte address error frame. The 68000 status word records R/W for the access, clears I/N and gives the function code for the processor's actual mode. EASy68K always reports a supervisor write. Without a handler, the program ends with `BusError {address, size, access, range}` and the vector. The instruction aborts at the error, as on a 68000; EASy68K finishes it.
- **Protected ranges** use the processor's S bit; EASy68K instead reads the SR field in its window. An exception vector is read in supervisor mode, so a protected vector table can serve user mode.
- **ROM stores** are ignored. They create no journal entry, change no code, and are neither code writes nor watched writes.
- **`trap #15` buffers** are checked as in EASy68K. Strings read by tasks 0, 1, 13, 14, 17, 18 and 95, and paths read by tasks 51, 52, 57, 59, 70, 71, 73 and 74, are checked from the start through the NUL against invalid ranges. Task 54's file-write buffer is checked against invalid ranges. Task 53's file-read buffer is checked against invalid, read-only and ROM ranges; a ROM range accepts nothing and the task does nothing. None checks protected ranges. Task 2's line and task 58's dialog are written without checks, as in EASy68K. An empty buffer, a string consisting only of its NUL, or a read of no bytes is refused if it starts inside a range after its first byte, but not on that first byte. This follows EASy68K's `loc + bytes - 1 >= start && loc <= end` check.
- **Debugger and host access** bypasses the map: Pokes, testcase starting memory, host reads, `disassembleAt`, undo and the program's initial contents are not checked.
- With no range declared, an access checks one flag and continues. The decode cache does not keep instructions whose words overlap invalid or protected ranges. Each fetch is decoded and checked in the current processor mode, and a write into one is not a code write.

## Cycles

The interpreter counts the clock cycles a 68000 takes, with the 68000's own time for each instruction, exception and interrupt request (`src/machine/timing.rs`). A 68010 program also uses exact processor timings. A 68020 program uses an estimate based on the cache case in the 68020 manual (below, "The 68020's cycles").

- **Sources.** Base instruction and effective-address times come from section 8 of the 68000 User's Manual, with corrections from Yacht ("Yet Another Cycle Hunting Table"). Each is checked against the cycle lengths recorded by the conformance suite (below, "Conformance"). As in that suite, the bus responds immediately, with no wait states.
- **Data-dependent times.** `mulu` takes two cycles for each set bit in its source, and `muls` takes two for each change between its bits. `divu` and `divs` use the microcode loop time, calculated with Jorge Cwik's method: 76 to 136 and 120 to 156 cycles respectively, or 10 and 16 or 18 when the quotient overflows. A register shift or rotate takes two cycles per bit of its count, modulo 64. `movem` takes four or eight per register. `Bcc` takes 10 when it branches, or 8 for `.s` and 12 for `.w` when it does not. `DBcc` takes 10 when it branches back, 12 when the condition is true and 14 when the counter runs out. `Scc` takes two more cycles when it sets a register; `bchg`, `bclr` and `bset` take two more for a register bit above 15.
- **Exceptions** take the 68000's time for the whole step: 34 cycles for `trap #0` to `#14`, `trapv`, illegal instructions, privilege violations and line A and F words; 38 plus the source effective address for division by zero; and 38 above the bound or 40 below zero for `chk`, plus the source effective address. An address or bus error takes 58 cycles from the aborted access, after the time the instruction has already taken. Trace adds 34 after the instruction; an IRQ takes 44 in its own step. `stop` takes 4 and `reset` 132.
- **`trap #15` tasks** take no time, as in EASy68K. A number that is not an EASy68K task takes 34 cycles as a `trap` through vector 47.
- **The counter** is 64 bits. `getCycles()` returns it as a `bigint` (`get_cycles()` in Rust). Each history step records its own `cycles`, and undo restores the counter. Task 30 clears the counter; task 31 reads it into `D1.L`, returning 0 once it exceeds 32 bits, as in EASy68K. A step that clears the counter still records its own time.
- **Processor precision.** The 68010 counts its own time (below, "The 68010"); the 68020 uses an estimate (below, "The 68020's cycles"). `program.getInfo().cyclesExact`, or `program.hasExactCycles()` in TypeScript, reports whether the count is exact. It is true for 68000 and 68010 programs and false for 68020 programs, whose count a host shows as an estimate. It is also false for a 68000 or 68010 program using `opt bit` with a bit-field instruction: those instructions take EASy68K's time, the 68020 cache case (below, "The 68020's instructions"), while the other instructions retain exact timings.

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

The interpreter is checked against [SingleStepTests/m68000](https://github.com/SingleStepTests/m68000), a suite generated from MAME's microcoded 68000 (MIT). Each test provides the state before and after one instruction. The runner sets the initial state, executes the instruction and compares the registers, status register, program counter, every RAM word listed in the final state, and cycle count. It also checks that nothing else was written. Musashi's sixty self-checking 68000 programs run as well, each until it writes its verdict.

- `cargo test` runs the vendored subset, the first 50 tests of each of the 127 files that raise no address error (`tests/m68000/`), and Musashi's programs (`tests/musashi/mc68000/`).
- The whole suite, 317,500 tests, runs from a fetched copy:

  ```sh
  scripts/fetch-m68000-tests.sh
  cargo test --release --features conformance conformance:: -- --ignored --nocapture
  ```

  The suite takes a few minutes and prints counts for every file. The `conformance::` filter excludes the crate's other ignored tests, which need GNU binutils for m68k. `S68K_M68000_ONLY=ASR.b,ASR.w` selects only the named files; `S68K_M68000_TESTS` points to another copy of the suite's `v1` folder. A GitHub Actions workflow runs the suite weekly.
- The skip list gives a reason for each exclusion (`src/conformance/skips.rs`): `trap #15`, EASy68K's service trap. Every exception the suite takes is checked, including its frame: address errors, privilege violations, line A and F, `trap`, `chk`, `trapv` and `rte`, in both modes. The suite also checks `stop` and `reset`. Since `stop` leaves the processor stopped before prefetch, its program counter is compared with the suite's next prefetch.
- Every test's length is compared with its step's cycle count (above, "Cycles"), including `tas` and `trapv`. The suite's README does not vouch for those two timings, but its results agree with the 68000's times for all 2,500 tests of each.

The counts on 2026-10-28, with the suite at its pinned commit, the lengths compared:

| | Tests | Run and passed | Failed | Skipped |
|---|---|---|---|---|
| The whole suite | 317,500 | 317,340 | 0 | 160 |
| The vendored subset | 6,350 | 6,347 | 0 | 3 |
| Musashi's programs | 60 | 55 | 0 | 5 |
| Musashi's 68040 programs, as 68020 programs | 17 | 17 | 0 | 0 |

Of the 127 files, 126 pass all 2,500 tests, including their lengths. `TRAP` passes all 2,340 tests that are not `trap #15`, the only skip. Five Musashi programs remain skipped. `abcd`, `sbcd`, `divs` and `divu` check Musashi's answers where they differ from 68000 microcode: BCD digits above 9 and flags after division overflow. `move` uses a 68020 form of `cmpi`.

### The 68010

No single-step suite covers the 68010, so a differential harness runs s68k alongside a reference 68010, one instruction at a time (`src/test/differential.rs`). It runs every first word the decoder accepts as a 68010 instruction from random states. Under Musashi, one in eight states has trace enabled. The harness compares registers, both stack pointers, the status register, program counter, VBR, SFC, DFC, every byte either implementation wrote (including frames), and cycles.

Both references are MIT licensed and fetched rather than vendored. For host tests, `build.rs` compiles each with the `cc` crate only when its feature is enabled:

```sh
scripts/fetch-musashi.sh
cargo test --release --features musashi differential -- --ignored --nocapture
scripts/fetch-moira.sh
cargo test --release --features moira differential -- --ignored --nocapture
```

- **Musashi** checks behaviour. The `RELAXATIONS` list gives a reason for each case excluded because Musashi differs from a 68010: it has no address errors in 68010 mode, forgets a pending trace when an instruction takes an exception, allows a 68010 `movec` CACR, runs the 68040's `move16`, and pushes `link a7`'s A7 four lower. It also sets flags left undefined by the manual differently from MAME's 68000 and does not check the `rte` version. Its cycle table gives maxima and has no loop mode, so cycles are reported but not compared.
- **Moira** checks behaviour and cycles. Its 68010 counts cycles for each bus access and is checked against cputester on hardware, including loop mode. The harness compiles it with precise timing and without Musashi mimicry. Each one-word instruction also runs step by step against Moira in a loop with a `dbf` back to it. The harness excludes flags the manual leaves undefined after decimal instructions and division by zero. It also excludes cycles for `divs` of `$80000000`, whose absolute value Moira computes in 32 bits.
- **GNU `as -m68010`** encodes every 68010 instruction form the same way in the crate's opt-in GNU `as` check. The check compares every form in the table with `as -m68000` and the 68010 forms with `as -m68010`: `S68K_BINUTILS_PREFIX=/usr/bin/m68k-linux-gnu- cargo test --release gnu_as -- --ignored`.

The counts on 2026-10-29 use two cases of each first word, or four against Moira:

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
| Condition codes and status | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo, `move`/`andi`/`ori`/`eori` with `sr` or `ccr` as an operand, and `move usp,an` and `move an,usp`; `move ccr,<ea>` is the 68010's, under `opt p=68010` |
| Bitwise                | not, or, ori, and, andi, eor, eori, lsl, lsr, asr, asl, rol, ror, roxl, roxr, btst, bclr, bchg, bset                                                                                                              |
| Binary coded decimal   | abcd, sbcd, nbcd                                                                                                                                                                                                 |
| Other                  | clr, exg, neg, ext, swap, move, link, unlk, lea, pea, moveq, movea, movem, movep, tas, nop, and the 68020's extb under `opt p=68020`                                                                               |
| The 68010's            | rtd, bkpt, movec, moves and `move ccr,<ea>`, under `opt p=68010` or `opt p=68020`                                                                                                                                 |
| The 68020's            | chk2, cmp2, cas, cas2, pack, unpk, divsl, divul, trapcc and its aliases, the long forms of muls, mulu, divs, divu, chk and link, `link.w`, the 68020 manual's spelling of `link`, tst and cmpi with operands the 68000 cannot read, and extb, under `opt p=68020` (below, "The 68020's instructions") |
| Bit fields             | bftst, bfextu, bfexts, bfffo, bfchg, bfclr, bfset, bfins, under `opt p=68020`, or `opt bit` with the 68000's addressing modes                                                                                       |
| Exceptions             | chk, trapv, illegal, trap #0 to #14, rte, and rtr, which returns and restores the condition codes (above, "Exceptions")                                                                                            |
| Interrupts and control | stop, which waits for an interrupt request, and reset (above, "Interrupts")                                                                                                                                         |
| Interrupt              | trap #15, with the I/O tasks 0-9, 11-20, 23, 24 and 33 in `d0`, the file tasks 50-59, the mouse task 61, the sound tasks 70-77, the graphics tasks 80-96, the interrupt-request tasks 60 and 62, and task 32 with 4, 5 or 6 in `d1.b` (the `Interrupt` enum of `src/instructions.rs` is the list of those the host answers) |

## Encodings

Where the 68000 has several encodings of a line, s68k picks the one EASy68K picks:

- The first of EASy68K's forms that fits: `cmp #n,dn` is `cmp`, and `cmp #n,(a0)` is `cmpi`; `and #n,dn` and `or #n,dn` are `and` and `or`, and into memory `andi` and `ori`; `eor #n` is `eori`; `add #n,an` and `sub #n,an` are `adda` and `suba`, and `add #n,dn` is `addi`.
- The optimisations EASy68K always makes: `move.l #n,dn` of -128 to 127 is `moveq`, and an `add`, `addi`, `adda`, `sub`, `subi` or `suba` of 1 to 8 is `addq` or `subq`. Writing the immediate `#n.l` keeps the long form, `move.l #1.l,d0`; it is the one size suffix an immediate takes.
- An unsized branch is short if its target is above the line and its displacement fits a byte and is not 0. Zero marks a word branch. Otherwise, the branch is a word. `.s` and `.b` force a short branch, and `.w` forces a word branch. On the 68000, `.l` also forces a word branch, as in EASy68K, because there is no long branch. Under `opt p=68020`, it forces the 68020's six-byte long branch (below, "Processor dialects"). An unsized absolute address is `.w` if it is above the line and fits a word, and `.l` otherwise. "Above" includes a label on the same line; a number is always above.
- An immediate is kept at the size the instruction reads it at, so `move.w #-1,d0` holds `$FFFF`, and `exg a0,d0` is `exg d0,a0`.
- Under `opt p=68020`, a line that the 68000 can encode keeps its 68000 encoding. The assembler chooses a wider 68020 form only where the 68000 has none. For a known displacement above the line that exceeds a 68000 field—a word for `d(An)` and `d(PC)`, or a byte for `d(An,Xn)` and `d(PC,Xn)`—it selects a word or long 68020 base displacement as needed. An unsized backward branch beyond word range becomes long, as in vasm. A name defined below the line keeps the 68000 field and causes an error if it does not fit. The hint suggests an explicit width, `(label.w,a0,d0)` or `bra.l`: without relaxation, a forward branch that might be long would always be long. In 68020-specific forms, an unsized base or outer displacement is omitted for a known zero, uses a word or long as needed for a known value, and uses a long for a name below (below, "The 68020's addressing modes").

Like EASy68K, the assembler uses two passes with nothing in between. The first chooses instruction sizes using these rules; a name defined below the line is always given the long form, whatever its value. The second encodes each instruction at its laid-out size. Instructions start on even addresses and occupy two to ten bytes, or up to 22 with 68020 addressing modes.

A branch displacement that does not fit is an error, as in EASy68K. A short branch must be in -128 to 127 and cannot target the very next instruction, because displacement 0 marks a word branch. A word branch or `dbcc` must be in -32768 to 32767. A PC-relative displacement is measured from its own extension word: two bytes past the instruction, four for `movem`, `btst #n` and 68020 instructions whose own extension word comes first, and four or six for the 68020's `cmpi` against the program counter.

Three lines EASy68K refuses are refused here too, since the 68000 has no encoding for them: an address register added into memory (`add.l a0,(a1)`), a bit number written as an immediate on an immediate (`btst #1,#5`, where `btst d1,#5` is the instruction), and a `link` displacement outside -32768 to 32767.

## Processor dialects

A program targets the 68000 unless it specifies `opt p=68010` or `opt p=68020` before its first instruction. This is Devpac syntax, also accepted by vasm. `opt bit` retains EASy68K's meaning: it enables the 68020's bit-field instructions on a 68000. `p=68020` implies it.

The program exposes its selection through `program.getProcessor()` and the `processor` and `bitFields` fields of `program.getInfo()`. The latter also has `cyclesExact`, which is false for the 68020's estimated cycles (below, "The 68020's cycles") and for a program using a bit-field instruction under `opt bit`. The program's code is decoded for the selected processor. A later processor's word is an illegal instruction in an earlier processor's program, both during execution and in `disassembleAt`.

`S68k.assemble` also returns `processor` and `bitFields` beside `diagnostics`, whether or not it builds a program. An editor can therefore identify the target even when the source has an error.

- **Selection must precede code.** Each instruction is checked and measured for the processor in force. An `opt p=` or `opt bit` after an instruction is an error (`processor_chosen_after_code`) and changes nothing. Repeating the same processor is harmless; selecting another is `processor_chosen_twice`, even across project files. A processor other than these three produces `unknown_processor`.
- **Instruction availability.** Each form in `S68k.instructionTable()` has a `processor` field naming the first processor that supports it. A later form is refused with `needs_a_later_processor`. Its hint gives the 68000 alternative and the `opt p=` line to add. The TypeScript diagnostic's `fixes` contains both suggestions, with the `opt p=` fix first. Each has a title, location and replacement text; a location without columns inserts text.
- **The 68010 is complete** (below).
- **The 68020** has every user-mode integer instruction except `callm` and `rtm`, which are permanently omitted (below, "The 68020's addressing modes" and "The 68020's instructions"). It includes the 68010 instructions, long branches `bra.l`, `bsr.l` and `b<cc>.l`, every addressing mode, and every form it adds to a 68000 instruction. A later form such as `muls.l` or `tst a0` in an earlier program produces `needs_a_later_processor`, just like a new 68020 instruction. The supervisor model is that of a 68EC020 (below, "The 68020's supervisor model"); cycle counts are estimates based on the manual's cache case (below, "The 68020's cycles").
- **`opt bit`** gives a 68000 or 68010 program the eight 68020 bit-field instructions, as in EASy68K, using that processor's addressing modes. For example, `bfextu (8,a0,d1.w*4){0:8},d0` still needs `opt p=68020`. Without the option, a bit-field instruction in an earlier program produces `needs_a_later_processor`, whose second fix adds `opt bit`. The code decodes for the selected processor and its bit fields.

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

- **Omitted parts.** The base, index and either displacement may be omitted, as the manual allows. Write a displacement width as `.w` or `.l` after it, or let the assembler choose (above, "Encodings"). `(d.w,An)` and `(d.w,PC)` are the 68000's own `d(An)` and `d(PC)`. An index without a size is `.w`, as on the 68000; GNU `as` makes it long.

  `za0` to `za7`, `zd0` to `zd7` and `zpc` are registers only inside parentheses, where `pc` is also a register, and only if no Symbol has that name. A Symbol named `za0` keeps its meaning wherever it can stand: `(za0)` is its address and `(za0,a1,d0)` uses it as a displacement. This also holds in a 68000 program, which has no suppressed registers. A 68020 program receives `symbol_named_like_a_register` where the name could also have been a register (`docs/grammar.md` 1.10).
- **Extension words.** These are the brief and full extension words of M68020UM, with base and outer displacements after the full word. The decoder reads a 68020 Program's index word as a 68020 does: bits 10–8 mark the scale and full word. Other Programs use the 68000 interpretation. Reserved combinations are read bit by bit as in Musashi and printed as their words. The disassembler prints every form with its widths; its output assembles back to the same words.
- **Running.** A memory indirect Operand reads its long pointer as a program read, in program space when using the program counter. A pointer or operand at an odd address is read normally, as the 68020 permits data at any address.
- **Cycles.** Each mode uses its row in the manual's effective-address tables, according to displacement widths and whether it reads memory (below, "The 68020's cycles").
- **Long branches** `bra.l`, `bsr.l` and `b<cc>.l` use a 32-bit displacement. An unsized backward branch beyond word range uses this form.
- **Verification.** GNU `as -m68020` writes the same words for every form in the 2,000-line opt-in check (`S68K_BINUTILS_PREFIX=… cargo test --release gnu_as -- --ignored`). The differential harness also runs random Operands of every mode against Musashi's 68EC020: `cargo test --release --features musashi the_68020s_addressing -- --ignored`. All 39,930 cases agree, including data at odd addresses. The 70 jumps to an odd address are excluded: they are the only exception these cases raise, and s68k's 68020 takes an address error while Musashi's 68020 has none. The supervisor-model runs compare every other exception (below).

### The 68020's instructions

`opt p=68020` enables the 68020's user-mode integer instructions described in M68020UM and the Programmer's Reference Manual. Where Motorola leaves a flag or the result for overlapping registers undefined, s68k follows WinUAE's 68020 behavior, derived from Toni Wilen's cputester runs on hardware (`newcpu_common.cpp` and `gencpu.cpp`). This sometimes differs from the 68040 modeled by Musashi.

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

- **Syntax.** Write a register pair as `d1:d2` and `cas2`'s two addresses as `(a0):(d1)`, using an address or data register for each. A bit field follows its operand as `{offset:width}`. Each part is a number, with or without `#`, or a data register. A literal offset is 0 to 31 and a literal width is 1 to 32 (`docs/grammar.md` 2.5). A data register holds an offset of any size and a width modulo 32.
- **`cas` and `cas2`** read and write in one indivisible step. s68k has one processor and no bus to lock.
- **Cycles.** Each instruction takes the manual's cache-case time (below, "The 68020's cycles"). For bit fields in a 68000 program under `opt bit`, s68k uses EASy68K's timing: the 68020 cache case for the field plus the 68000 effective-address time, with more for a field spanning five bytes. No 68000 or 68010 has these instructions, so that timing belongs to no processor. A program containing one reports an estimated count with `cyclesExact` false, although its other instructions retain their exact timings.
- **Verification.**
  - GNU `as -m68020` writes the same words for all 550 lines of 68020 forms, including `extb.l` and `link.w`, in the opt-in check (`S68K_BINUTILS_PREFIX=… cargo test --release gnu_as -- --ignored`).
  - The differential harness runs every first word available on the 68020 but not the 68010 against Musashi's 68EC020, with 64 random cases each. It starts from 68020 supervisor-model states and compares exceptions and frames (`cargo test --release --features musashi the_68020s_instructions -- --ignored`). All 70,612 cases agree. The list in `src/test/differential.rs` excludes the supervisor-model gaps (below, "The 68020's supervisor model") and cases where Musashi differs from the 68020. Musashi sign-extends `cas2.w`'s compare value through an address register, reads `pack` bytes in the opposite order, and compares a `chk2` address register at operand size and its bounds as signed only. It also misplaces a bit field with an offset within eight of −2³¹, takes a `bfffo` register offset modulo 32, keeps the second operand in a shared compare register, and leaves its own undefined flags.
  - Musashi and the decoder agree on which of the 65,536 first words are 68020 instructions, except for `callm`, `rtm`, `illegal` and the line F words that a 68020 without a coprocessor takes as line F.
  - Seventeen of Musashi's self-checking 68040 programs run and pass as 68020 programs (`tests/musashi/mc68040`): bit fields, `cas`, `chk2`, `cmp2`, long multiplies and divides, `trapcc`, `rtd`, `jmp` and `shifts3`. Their checks of the 68040's undefined flags after division are relaxed, as are checks of `chk2` and `cmp2` that depend on Musashi's interpretation. Each relaxation has a reason (`src/conformance/skips.rs`).

### The 68020's supervisor model

`opt p=68020` gives a program the supervisor model of the 68EC020, the 68020 with the 68000's 24-bit bus (M68020UM, sections 1, 2.3, 6 and 7), with no coprocessor and no cache:

- **The status register** contains T1, T0, S, M, the interrupt mask and condition codes. Thus `move #$FFFF,sr` reads back `$F71F`; a 68000 or 68010 reads `$A71F`. T1 corresponds to the 68000's T and traces every instruction. With T1 clear, T0 traces instructions that change program flow: a taken branch, `bra` and `bsr` always, `jmp`, `jsr`, `rts`, `rtr`, `rtd` and `rte`. It also traces writes to the status register: `move` to SR and `andi`, `ori` and `eori` to SR or CCR, as in WinUAE's 68020 based on cputester. An instruction that raises an exception is not traced by T0 because taking the exception clears T0, and `stop` stops. T1 traces `stop`, which then does not stop. The trace frame is format `$2` and includes the traced instruction's address.
- **Three stack pointers.** With S clear, A7 is the user stack pointer. With S set, A7 is the interrupt stack pointer (ISP) if M is clear, or the master stack pointer (MSP) if M is set. ISP is the 68000's SSP. A program starts with ISP at `$01000000` and M clear; one that never sets M runs as on a 68010. A write to SR, an exception or `rte` can swap the pointers. Each retains its own stack top for the memory view (`getStackTop()`).
- **An interrupt in master state** pushes a format `$0` frame on MSP, clears M and pushes a format `$1` throwaway frame on ISP. The latter contains the same program counter and vector, with S set in the interrupted status register, as in Musashi and WinUAE. The handler runs on ISP. Its `rte` pops the throwaway frame; that frame's status register has S and M set, making A7 the MSP again. `rte` then continues to the frame there.
- **`movec`** accesses `msp`, `isp`, `cacr` and `caar` in addition to the 68010's four registers, using codes `$803`, `$804`, `$002` and `$802`. A stack pointer is A7 when it is the active pointer. CACR retains E and F, bits 0 and 1. C and CE, bits 3 and 2, are cache-clear commands and read as 0. s68k has no cache, so code written by a program always runs as written. CAAR retains all 32 bits. The four codes are illegal instructions on a 68010, and the names are reserved in every program. An earlier program that names one is told it is a 68020 control register.
- **Frames** follow M68020UM 6.5. Format `$0` is eight bytes and is used for interrupts, `trap #n`, illegal instructions, line A and F, privilege violations and format errors. Format `$2` is twelve bytes, adding the instruction address after the format word, and is used for `chk`, `chk2`, `trapcc`, `trapv`, division by zero and trace.

  Bus and address errors use the long bus fault frame, format `$B`, of 92 bytes. A bus error fetching an instruction at an instruction boundary, before any of it ran, instead uses the 32-byte short frame `$A`. An odd program counter's address error uses `$B`, as in WinUAE's 68020. Like the 68010's `$8` frame, these frames are simplified. The program counter identifies the instruction; the special status word at `+$0A` and data-cycle fault address at `+$10` are real. The status word contains DF for a data access, RW for a read, SIZ and the function code; a fetch is a word read in program space. Every internal word is zero, including `$B`'s version at `+$36`.
- **`rte`** pops formats `$0`, `$2`, `$A` and `$B`. The last two return to the faulting instruction, which restarts, provided `$B`'s version is 0. For `$1`, `rte` pops the frame and continues to the frame on the stack named by its status register. Any other format, including the 68010's `$8`, causes a format error that stacks the `rte` and leaves flags unchanged, as in WinUAE's 68020.
- **Unaligned data.** Words and longs at odd addresses can be read and written through every memory path: operands of any mode, stack and frames, `movem`, memory-indirect Operand pointers, memory-map checks, code writes and watched ranges, including accesses wrapping past `$FFFFFF`. Only an instruction fetch, jump or return to an odd address causes an address error. The processor is checked only after an odd address is found, so an even access does not need a processor check.
- **Line F.** Without a coprocessor, line F uses vector 11 and a format `$0` frame stacking the instruction. Every word from `$F000` to `$FFFF`, including FPU and coprocessor words, decodes as a one-word line F instruction, except EASy68K's `simhalt`, `$FFFF $FFFF`.
- **System registers.** For a 68020 program, `getSystem()` adds `isp`, `msp`, `cacr` and `caar`; `ssp` is ISP under its 68000 name. `setIsp`, `setMsp`, `setCacr` and `setCaar` poke these registers and throw `InvalidArgument` in an earlier program. History's `WriteStackPointers` carries `{usp, ssp, msp}` (`msp` is 0 before the 68020) and `a7`, identifying which pointer A7 becomes. The mutations remain in order when an `rte` through a throwaway frame swaps twice. `WriteControlRegister` records `cacr` and `caar`.
- **Verification.** The hand tests cover every format, T1 and T0, MSP and ISP and undo across them, `movec`, `rte`'s checks and odd accesses on every path (`src/test/m68020_supervisor.rs`). The differential harness runs Musashi's 68EC020, which models the 68020's frames, its three stack pointers and M, from states with T1, T0, S and M random and a master stack of their own (`src/test/differential.rs`):

  ```sh
  cargo test --release --features musashi the_68020s -- --ignored --nocapture
  ```

  On 2026-10-29, the results were:

  - For 68010 instructions on the 68020, two cases per first word: 96,980 agree, including exceptions, frames, tracing and odd accesses.
  - For the 68020's own instructions, 64 cases per first word: 70,612 agree.
  - For interrupts: 20,000 agree, including 10,007 in master state with throwaway frames.
  - For `rte` on frames both implementations pop: 20,000 agree, including 6,678 through a throwaway frame.

  `RELAXATIONS_SUPERVISOR` lists excluded cases with a reason for each, in addition to the exclusions for the 68010 and 68020 instructions themselves. Musashi has no 68020 address error, so jumps and returns to odd addresses are not compared. It forgets pending trace when an instruction takes an exception. Under T0, it traces `nop`, `movec`, `moves`, `move` to USP, `stop`, `cas` and `cas2` as a 68040 does, but does not trace `andi`, `ori` or `eori` to CCR. Of the SR writes, it does not trace `move Dn,sr`.

  Musashi also runs line F words as FPU and coprocessor instructions, raises a format error on `$A` and `$B` and stacks the address after `rte` for that error, and retains CACR's C and CE bits. Flags that the 68020 leaves undefined after `chk`, `chk2` and long division are also excluded from the status registers stacked in frames; s68k sets them as WinUAE's 68020 does.

### The 68020's cycles

A 68020 program uses an estimated count based on the **cache case** in the M68020 User's Manual timing tables: section 8 of the 1995 edition or section 9 of the 1984 edition, which have the same values (`src/machine/timing.rs`). This assumes the instruction is in the cache and does not overlap its neighbours. The manual gives a best, cache and worst case for each row and says exact times cannot be determined from the tables: they depend on the instruction cache, prefetch and overlap. The cache case is the deterministic middle case, also used by Musashi's 68020. `program.getInfo().cyclesExact`, or `program.hasExactCycles()` in TypeScript, is false for a 68020 program so a host can show the count as an estimate.

- **Calculation.** As the manual's notes specify, the count combines operation time with the operand's fetch, calculate, immediate or jump effective-address time. Instructions with their own extension word, including `movem`, `chk2`, `muls.l` and bit fields, add its time. `move` has its own table: two cycles plus the source fetch and destination time, reduced after a memory source because the write overlaps the read. A scaled brief extension uses the `d8(An,Xn)` row. Full-extension rows depend on displacement widths and whether memory is read; base or index suppression, index size and scale do not change the time. Operands are assumed aligned and there are no wait states.
- **Data-dependent times.** `Bcc.s` takes 4 cycles, or 6 when taken; `Bcc.w`, `Bcc.l` and `bra` take 6. `DBcc` takes 6, or 10 when its counter runs out. A memory bit field spanning five bytes costs more than one spanning four. `cas` takes 12 and `cas2` 22, each with 3 more when writing. `rte` takes 21, 43 for a short bus fault frame, 92 for a long one and 16 more through a throwaway frame.

  A shift or rotate takes the same time for any count. `mulu.w` takes 27, `divu.w` 44, `divs.w` 56, `mulu.l` 43, `divu.l` 78 and `divs.l` 90 regardless of operands. `movem` takes 8 plus 4 per register from memory, or 4 plus 3 per register into memory.
- **Exceptions** replace the instruction's time with the manual's exception time: 20 for `trap #n`, illegal instructions, privilege violations and line A and F; 25 for a taken `trapcc` or `trapv`; and 25 after the instruction for trace. An IRQ takes 26, or 41 in master state with its throwaway frame. A bus or address error takes 43 or 79 for a short or long bus fault frame. `stop` takes 8 and `reset` 518; `trap #15` tasks take no time, as on the 68000.
- **Manual gaps.** s68k makes these choices in `machine::timing`'s `m68020`: `chk`, `chk2` and division by zero take `trapv`'s 25 plus their source effective address, just as the 68000 uses its trap time plus the address; `bkpt` takes its 10-cycle breakpoint plus the illegal instruction's 20; a format error takes `rte`'s 21 plus an exception's 20. A bus or address error takes its frame time alone because the manual gives no time for work before the fault. Memory `nbcd` takes Musashi's 6 plus fetch.

  Where the manual is inconsistent or omits a row, a full extension with a word displacement uses `(d16,B)` as in Musashi, rather than the lower `(d16,An,Xn)` times for the same word. `-(An)` is absent from the calculate-immediate table and uses `(An)+` as in the calculate table. `([d16,B],I,d32)` is absent from the fetch-immediate table and uses `([d16,B],I,d16)`, as every other outer long uses its word equivalent.
- **Verification.** Unit tests check every cell of the manual's MOVE table, the effective address tables through `tst`, `cmpi`, `lea`, `jmp` and `movem`, rows of every other table and the exceptions, and programs that run them (`src/machine/timing.rs`, `src/test/cycles.rs`). Musashi's 68EC020 keeps the cache case too, one number per opcode and a table per mode, and the differential harness reports how the two compare rather than asserting it:

  ```sh
  cargo test --release --features musashi the_68020s_cycles -- --ignored --nocapture
  ```

  On 2026-10-29, 92,080 of 134,646 cases agree: 76,833 of 96,980 cases of 68010 instructions on the 68020 (two per first word), 6,481 of 17,703 of the 68020's own (sixteen each), and 8,766 of 19,963 addressing-mode cases in `lea`, `move`, `add`, `clr` and `jmp`.

  Where they differ, Musashi departs from the manual in these ways:

  - It adds fetch time where the manual adds calculate or jump time: `lea`, `pea`, `jmp`, `jsr`, `clr`, `Scc`, `tas`, `move` from SR, `movem`, `moves`, `cas` and bit fields.
  - It omits the immediate for `addi` and related instructions to a register, and the extension word of `muls.l`, `divu.l`, `chk2` and `cmp2`.
  - It adds both effective addresses for memory-to-memory `move`, where the manual overlaps them.
  - It takes one cycle per rotate bit, 10 for `bra`, 8 for `move sr,Dn`, 4 per register for `movem` into memory, 12 for `cas2` and 84 for every long division.
  - It takes 20 for a taken `trapcc` or `trapv` and 20 for `bkpt`, where the manual has 25 and 30. It uses its 68000 times of 40, 38 and 34 for out-of-bounds `chk`, division by zero and privilege violation.

### The 68010

`opt p=68010` gives a program the whole 68010 of the 68000 User's Manual (its sections 6 and 9 and appendix A) and the Programmer's Reference Manual:

- **Instructions.** `move ccr,<ea>` lets user mode read condition codes. `rtd #d` returns and adds the displacement to the stack pointer. `bkpt #n` takes the illegal instruction exception, as a 68010 does when no debugger answers its breakpoint cycle. Privileged `movec` transfers between a general register and `sfc`, `dfc`, `usp` or `vbr`; any other control register causes an illegal instruction. `moves` is also privileged.

  s68k has one address space, so `moves` accesses the same memory as `move`, as a supervisor data access regardless of SFC and DFC. Each is three bits, writable and readable with `movec`. `sfc`, `dfc` and `vbr` are reserved in every program, as is `usp`.
- **`move sr,<ea>` is privileged**, the one change visible to a user-mode program. `clr` and `move sr` no longer read before writing.
- **VBR.** Every exception, including IRQs, reads its vector at VBR plus four times the vector number. VBR is 0 until `movec` changes it.
- **Frames.** Every exception except the two in group 0 pushes an eight-byte format `$0` frame: status register, program counter, and format/vector-offset word containing four times the vector. Bus and address errors push a 29-word format `$8` frame; its three reserved words are not written. It contains the status register, faulting instruction's program counter, `$8` and the vector offset, special status word (IF, DF, BY, R/W and function code), fault address, three buffers and sixteen internal words. The buffers and internal words are zero, including version 0.
- **`rte`** reads the format word first. It pops eight bytes for `$0` and 58 for `$8`, then returns to the stacked program counter. The faulting instruction runs again because s68k retains no internal state to continue it. Any other format or a version other than 0 causes format error vector 14. The frame stays in place and the `rte` address is stacked. For an unsupported format, N is set to its high bit and Z and V are cleared, as on a 68010.
- **Cycles** follow section 9 of the manual and Moira where the manual gives a maximum or differs (`src/machine/timing.rs`). `mulu` takes 40, `muls` 40 or 42 depending on the source, `divu` 78 plus two per step without a carry, and `divs` 118 or 120; overflow takes 8 and 16 respectively. An exception takes 38, an IRQ 46, `chk` 42 or 44, division by zero 42, `trapv` 40, `bkpt` 42 and `rte` 24. An address or bus error takes 134 from the access, after the 68010's own lead. `Bcc.s` takes 6 when not taken and 10 when taken; `DBcc` takes 10, or 16 when its counter runs out.
- **Loop mode.** With T clear, a `DBcc` with displacement −4 enters loop mode when it branches back to an eligible one-word instruction. Subsequent iterations fetch nothing: `move.w (a0)+,(a1)+` takes 8 and `dbra` 6, instead of 12 and 10. Loop mode ends when the counter runs out or the condition is true, at the time the instruction gives it, on any exception including an IRQ, or on a write to either loop instruction by the program, a trap task or a Poke. After a write, the words are decoded again and the loop runs outside loop mode until `DBcc` enters it again. Undo restores the mode. Loop mode changes cycles only.

## The S-record

`Program::to_srecord()`, or `program.toSRecord()` in TypeScript, writes a `.S68` file in the format produced by EASy68K's assembler. EASy68K's simulator can load and run it (`src/assembler/srecord.rs`, after EASy68K's `OBJECT.CPP`):

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
| `list`, `nolist`, `page` | accepted and ignored; they control a listing file, which s68k does not produce |

Without `end`, the entry point is the label `START`, or the first instruction if there is no `START` label.

`memory` reads addresses as EASy68K does. Forward references are not allowed, and each address is masked to the 24-bit address bus: `$FF001000` becomes `$001000`. The map has one range of each kind. A second `memory rom` replaces the first with a warning. A range ending before it starts covers nothing and also produces a warning.

`equ`, `set` and `reg` require a label. A `section` without a number also requires one and sets it to the current section number. `page`, `memory` and conditional directives take no label. Define a `reg` list before the `movem` that reads it; the list cannot appear in an expression.

## Projects of several files

A **project** maps root-relative paths to file text or bytes and specifies the path of the **entry file** to assemble. The assembler reads other files only when the entry file reaches them through `include` or `incbin`. It never opens files on disk.

```ts
S68k.assemble({
    files: {'main.x68': source, 'lib/io.x68': library, 'data/sprite.bin': bytes},
    entry: 'main.x68'
})
```

`include` is **textual**: the included lines are assembled at the `include` line, in the same section and at the same address. They share the symbol namespace, and local label scopes cross the file boundary. A file name may be unquoted or enclosed in either kind of quote. The assembler looks beside the including file first, then at the project root. A file may be included more than once. `end` may appear only in the entry file.

`incbin` accepts either kind of file: binary files contribute their bytes, and text files contribute their Windows-1252 bytes. A label on the directive names the first byte.

Each location names its file, so diagnostics, breakpoints (`{file, line}`), the current line and call-stack frames identify a project file. A diagnostic in an included file carries the `include` lines through which it was reached as related locations. Each assembled instruction carries them too, in `includeChain`, innermost first. If a file is included twice, each line has one location but two addresses; the chain distinguishes the copies.

## Text and the trap #15 tasks

Characters use one byte in **Windows-1252**, EASy68K's code page. It replaces the Latin-1 control characters at `$80` to `$9F` with `€ ‚ ƒ „ … † ‡ ˆ ‰ Š ‹ Œ Ž ‘ ’ “ ” • – — ˜ ™ š › œ ž Ÿ`. The assembler stores `dc.b 'café €'` using those bytes and rejects a character with no corresponding byte (`character_above_latin1`). The interpreter uses the same table for encoding and decoding at run time.

`TRAP #15` stops the interpreter with an `Interrupt` and waits for an answer. The interpreter defines each task; the host displays text and passes on typed input:

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

A line ends at its first line terminator. A typed character that Windows-1252 cannot represent is stored as `?`. `atoi` reads leading whitespace, one sign and digits up to the first non-digit. Numbers too long for 32 bits wrap, so every number from -2147483648 to 4294967295 lands in D1.L exactly.

### Input settings: tasks 12 and 16

Task 12 turns input echo off when D1.B = 0 and on for any other value. Task 16 turns the input prompt (EASy68K's flashing cursor) off and on with D1.B = 0 and 1. Values 2 and 3 turn off and on the line feed echoed after the carriage return when a key read receives Enter.

These tasks raise no interrupt. The settings belong to the interpreter and are available through `getInputSettings()` as `{echo, prompt, line_feed}`. All start on, as in EASy68K. Changes are journaled like registers with a `SetInputSettings` mutation, so undo restores them. While a read task waits, the host reads the settings and displays input accordingly. Even with echo off, Enter ends a line read on a new line, as in EASy68K.

### The cycle counter: tasks 30 and 31

Task 30 clears the cycle counter. Task 31 reads it into D1.L, returning 0 once it exceeds 32 bits, as in EASy68K (above, "Cycles"). Neither task raises an interrupt or takes time. Undo restores the counter.

### Files: tasks 50 to 59

The host's file system performs file operations. Each file task raises an interrupt with decoded arguments, and the host answers with the result. The interpreter writes EASy68K's result to D0.W: 0 for success, 1 for end of file, 2 for an error and 3 for read only.

A path is a NUL-terminated string of at most 255 characters, decoded from Windows-1252. Backslashes (`\`) are sent as `/` because Windows accepts both separators. The host maps the path to its own files.

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

File numbers run from 0 to 7; the lowest free number is used, as in EASy68K. The host opens at most eight files and answers `null` for a ninth. The interpreter handles checks EASy68K makes before touching a file, returning 2 in D0.W without an interrupt: a file number outside 0 to 7 for tasks 53 to 56, a zero-byte read or write, a buffer past the end of memory, or a negative position. Opening a read-only file returns 3; writing to it fails with 2, as EASy68K's `fwrite` does.

Read and write bytes cross the WebAssembly boundary as a `Uint8Array` (serde bytes copied in one operation by `serde_wasm_bindgen`). The interpreter journals bytes written to memory by a read, so undo restores them.

### Sound: tasks 70 to 77

The interpreter does not play sound itself. It raises interrupts with the arguments: `PlaySound(path)` (70), `LoadSound {path, index}` (71), `PlayLoadedSound(index)` (72), and their DirectX forms `PlaySoundDirectX`, `LoadSoundDirectX` and `PlayLoadedSoundDirectX` (73 to 75). Tasks 76 and 77 use `ControlSound {index, control}` and `ControlSoundDirectX`. The index is D1.B and control is D2.L: 0 plays once, 1 loops, 2 stops the sound and 3 stops every sound.

A host with a player answers with a `SoundOutcome` describing what it did. The interpreter writes D0.W as EASy68K does; task 71 has no result. A host without a player answers `Terminate`.

| Outcome | It means | 70, 72 | 73 to 77 |
|---|---|---|---|
| `"Done"` | the task did what it asked: a sound started, a load loaded, a stop stopped | 1 | 1 |
| `"Busy"` | the standard player was still playing an earlier sound; the DirectX player mixes, so it is never busy | 0 | 0 |
| `"Missing"` | no file at the path, or no sound in the slot | D0 left as it was | 0 |
| `"Unplayable"` | a file that is there but cannot be played | 0 | 0 |

### When a task cannot be done, and why a program ended

An error with a 68000 exception goes through its vector (above, "Exceptions"). It ends the program only if the vector has no handler. In that case the status is `TerminatedWithException`, the error is thrown, and `getTermination()` returns the cause and vector.

Trap tasks can also raise these errors:

- `UnsupportedTrapTask {task}` for a number that is not an EASy68K task goes through vector 47, as in EASy68K. Tasks a browser page cannot perform faithfully (10, 21, 22, 25, 32 with 0 to 3 or 7 in `D1.B`, 40 to 43, and 100 to 107) instead end the program regardless of vector 47.
- `InvalidTrapArgument {task, reason}` ends the program when a task receives an invalid value. The reason names the register.

Undoing the failed step returns the program to it in the running state.

`getTermination()` returns `null` while the program runs. After it ends, it reports `TerminateTask` for task 9, `EndOfProgram {address}` when execution reaches `$FFFF $FFFF` the program did not place (above), `TerminatedByHost` when the host answers an interrupt with `Terminate`, or `Halted {cause, vector}` for a double fault.

It reports `Exception {cause, vector}` for an error that ends the program. The cause can be `Irq {level}` when an IRQ's vector has no handler. The vector is present for an unhandled CPU exception or IRQ; it is absent for an error the program cannot handle.

An interrupt accepts an answer of the same `type` with a value valid for its task. `Terminate` answers any task and ends the program. Other answers are rejected with a typed error and the interrupt remains waiting. Errors include `NoPendingInterrupt`; `InvalidAnswer {interrupt, reason}` for the wrong answer type, a value that is not an answer, a key state of the other form, too many bytes for a read, or a file number above 7; and `OutOfBounds` for a write that does not fit in memory. WebAssembly methods throw `RuntimeError` objects and do not panic, so a bad call leaves the interpreter usable.

Where s68k departs from EASy68K, on purpose:

- For task 15, a base outside 2 to 36 stops the program with `InvalidTrapArgument`. The same error stops it for task 16 with D1.B outside 0 to 3, or task 58 with D1.L other than 0 or 1. EASy68K does nothing for these values, and task 58 reports success.
- A task s68k does not carry out stops the program with `UnsupportedTrapTask`; EASy68K carries it out.
- For tasks 0 and 1, s68k displays up to 255 characters when D1.W is `$8000` or more, as it does for any D1.W above 255. EASy68K reads such a value as a negative index into its buffer.
- `atoi` overflow wraps modulo 2^32. EASy68K's own `atoi` (Borland's runtime) was not available to check.
- s68k drops characters typed after the 79th character of a line. EASy68K ends the line at the 80th key and keeps the next key for the following read.
- A read of no bytes (task 53 with D2.L = 0) reports 2. After an earlier read reaches the end of the file, EASy68K's `fread` instead reports 1 using C's end-of-file indicator; s68k does not keep that indicator.
- A count past the end of memory reports 2 even when (A1) + D2.L passes 4 GB. EASy68K's 32-bit check wraps around in that case and reads past its memory. Task 58 checks all 256 bytes it writes; EASy68K checks only the name.
- s68k sends paths with `/` in place of `\`.
- On Windows, EASy68K cannot delete an open file and reports 2. In s68k, the host file system determines whether an open file can be deleted.

## Known limitations

- Cycle counts assume an immediately responding bus without wait states. The 68020 count estimates the manual's cache case, with aligned operands and no instruction cache, prefetch or overlap between neighbouring instructions (above, "The 68020's cycles").
- A step retains ten bytes of the instruction it ran, the 68000's maximum. If a longer 68020 instruction is decoded from memory written by the program or reached in its data, the step omits `ran`; an instruction placed by the assembler is reported. Retaining 22 bytes would cost space in every step that keeps a history.
- On the 68010 and 68020, `rte` restarts bus and address errors at the instruction's beginning instead of continuing from internal processor state, which s68k writes as zeros. The 68020's choice of short or long bus fault frame follows an s68k rule rather than the pipeline.
- The 68020 has no instruction cache, so CACR's E bit has no effect, and it has no coprocessor. SFC and DFC do not select an address space.
- Loop mode changes cycles only. An IRQ can still be taken between the looped instruction and its `DBcc`, whereas a 68010 takes it after the `DBcc`.
- Vectored and spurious interrupts, the reset exception and the 68000's HALT pin are omitted, as in EASy68K. Nothing reads the vectors at `$0` and `$4`, and `reset` does not run a reset sequence.
- The host runs task 32's timers. The interpreter keeps their settings and takes requests raised by the host. EASy68K's hardware window, IRQ buttons and devices are not present.

## Running the command-line interpreter

Install [Rust](https://www.rust-lang.org/tools/install), clone the repository,
and run `cargo run` from its root. This assembles and runs `code-to-run.asm`.
To run another file, pass its path: `cargo run -- my-program.asm`.

The command line handles text tasks in the terminal, honouring task 12's echo setting, and file tasks on disk. Paths are relative to the directory of the file being run. Task 51 opens a file read-only if it cannot write it. Task 58 asks for a path in the terminal; an empty line cancels the dialog.

Graphics, mouse and sound tasks are reported and end the run. The last line reports why the program ended. The command line raises no interrupt requests, so a program that executes `stop` ends the run there, waiting.

## Building the WebAssembly package

Install [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/), then run
`npm run build-wasm` in the `ts-lib` directory. This creates the compiled
package in `ts-lib/src/pkg`.
