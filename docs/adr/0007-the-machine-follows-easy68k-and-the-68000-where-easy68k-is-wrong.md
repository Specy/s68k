---
status: accepted
date: 2026-10-28
---

# The machine follows EASy68K's simulator, and the 68000 where EASy68K is wrong

ADR 0001 makes EASy68K the reference for the language, and the asm-editor's ADR 0035 makes it the reference for every `trap #15` service. Neither said what the machine underneath is: s68k ran every program in supervisor mode with one stack pointer and a status register whose system byte did nothing, which is no processor at all. The asm-editor's design for a real 68000 behind the M68K Target (its `docs/design/m68k-machine.md`, decisions 1, 2 and 10) asked for the machine EASy68K simulates, and its fact base (`docs/research/easy68k-machine-behaviour.md` there, read from EASy68K 5.16.1's source) lists where that simulator is a 68000 and where it is not. We decided three things.

- **The machine is a 68000 as EASy68K 5.16.1 simulates it**: its reset state, its 24-bit bus and 16 MB of memory filled with `$FF`, its list of exceptions and their frames, `trap #15` as its service trap, its memory map, and, when they come, its cycle counts.
    - **The reset state is EASy68K's** (`STARTSIM.CPP`): SR `$2000`, supervisor mode with the interrupt mask at 0 and no trace; D0 to D7 and A0 to A6 zero; the supervisor stack pointer, and so A7, at `$01000000`, the user stack pointer at `$00FF0000`; the program counter at the Entry point. The vector table is not initialised, so every vector reads `$FFFFFFFF`, and nothing reads the vectors at `$0` and `$4`: there is no reset exception at the start.
    - **Two stack pointers.** A7 is the stack pointer of the mode the S bit names, the other kept aside; every write of the status register, every exception and every `rte` can swap them (`Cpu::set_sr`). The status register keeps the bits a 68000 has, `$A71F`.
- **Where EASy68K departs from a 68000 by mistake, s68k is the 68000, and the conformance suite (ADR 0010) says what the 68000 does.** No program can usefully rely on any of these, and a student who learns the 68000 from s68k should not unlearn them. The known cases, from the fact base:
    - an interrupt taken in the same step as a trap, `chk`, `trapv`, a division by zero or a line A or F word is lost, and one beside an illegal instruction runs the instruction again (interrupts are the plan's milestone C8);
    - the trace and trap frames are pushed in the wrong order, so the trap handler runs before the trace handler;
    - line A, line F and a privilege violation stack the address after the operation word instead of the instruction's own;
    - the bus error's status word always says "write", never sets I/N and always gives a supervisor function code (bus errors came with milestone C7, with the 68000's status word);
    - the protected range's check reads the S bit from the window rather than the processor;
    - `stop` runs one more instruction before the interrupt it waited for, and calls an immediate with S clear a privilege violation;
    - `link` and `unlk` on A7 use the user stack pointer in supervisor mode.
- **Where the 68000 leaves something to the implementation, the suite's MAME microcode decides**: what an address error stacks and what it has done to the registers by then, which the 68000's manuals describe only as depending on the prefetch, and the flags a `move.l` has set when its write faults. `src/machine/exception.rs` and the instruction arms hold the rules, measured on the 55,606 address errors of the suite.

## Considered options

- **EASy68K's simulator in every detail, mistakes included**: the editor's programs would behave as they do in EASy68K, bug for bug, but a trace handler that runs after the trap it traced, or a privilege violation handler that has to subtract 2 to find the instruction, teaches a 68000 that never existed, and nothing in EASy68K's own documentation promises either.
- **The 68000 alone, EASy68K as a reference for services only**: the reset state, the memory fill and the `trap #15` rules would have no source, and EASy68K's own programs, which assume its reset state, would start differently.
- **Keeping s68k's own reset state**, SR `$2700` from EASy68K's help page: the mask of 7 would have blocked the interrupts of milestone C8 at the start of every program, where EASy68K's source starts at 0.

## Consequences

- `INITIAL_STATUS_REGISTER` is `$2000`, beside `INITIAL_SUPERVISOR_STACK_POINTER` and `INITIAL_USER_STACK_POINTER`. A program sees a status register of `$2000` before it runs; the corpus's run fixtures print the flags and not the system byte, so none of them moved (`tests/corpus/README.md`, milestone C6).
- `Cpu` keeps both stack pointers (`get_usp`, `get_ssp`), and the Interpreter answers the System register file, `get_system()` (`getSystem()`, `{sr, usp, ssp}`), with Poke setters for all three. A change of S journals both stack pointers (`WriteStackPointers`), and undo writes the status register back without moving them, then puts them back where they were.
- The conformance harness sets both stack pointers from a test's state (`Interpreter::from_state`) and compares both.
- The exceptions themselves, and what an exception with no handler does, are ADR 0008.
