---
status: accepted
date: 2026-10-28
---

# SIMHALT, and running into memory the program never placed ends it

Once the Interpreter runs what memory holds (ADR 0006), nothing says where a program ends. Before, a program ended when its program counter passed the last instruction the Program held, because nothing past that could run. Now everything can run: memory is 16 MB of `$FF`, a program that does not end itself runs on past its last instruction, and EASy68K's simulator halts at `$FFFF $FFFF` wherever it meets one, its SIMHALT. EASy68K halts quietly in erased memory, and a Run there pauses again at every following pair. The asm-editor's design for a real 68000 behind the M68K Target (its `docs/design/m68k-machine.md`, decision 7, confirmed by the owner) asked for the end to be told apart from a pause. We decided four things.

- **`$FFFF $FFFF` is SIMHALT wherever it is**, for every processor, as the decoder already reads it.
- **Inside the bytes the program placed, it pauses**, as `simhalt` does: the status is `Paused`, the program counter is past it, and the next step or run goes on. Placed is the Program's image: its instructions' bytes and those of `dc`, `dcb` and `incbin`, which the Program keeps as ranges. A SIMHALT any of whose four bytes is placed is the program's, so a `dc.w $FFFF` at the end of the data followed by memory's fill pauses too. The pad byte before an aligned item and the room `ds` reserves are not placed.
- **Anywhere else it ends the program**, with the termination `EndOfProgram` and the address the program counter was at. That is where a program that does not end itself ends: one past its last instruction when nothing follows it, which is how a program that ran off its last instruction ended before, now with the place said. A `$FFFF $FFFF` the program or a Poke wrote at run time outside the placed bytes ends it there too: it is the place, not the bytes' history, that decides. The end is reported by the step that got there, when it got there by running on, as the old end was, so the last instruction's step answers `Terminated` and undoing it takes the end back; a step that starts on it ends the program and runs nothing. A Program with no instruction, or whose Entry point is on one, has ended before it starts, at its Entry point. *(Amended on 2026-10-28 after the machine's batch 1 review: a Program with no instruction line runs what it placed at its Entry point, `dc.w` words written as code included, as EASy68K runs an S-record from its start address; only one whose Entry point is on `$FFFF $FFFF` it never placed, which an empty Program's is, has ended before it starts.)*
- **Any other bytes run as what they decode to.** A program that runs on into its data, or jumps there, runs the data; a word that is no instruction is an illegal instruction, and an instruction fetched from an odd address is an address error, both of which end the program until exceptions are taken (the plan's milestone C6). *(Amended on 2026-10-28 by [ADR 0008](0008-exceptions-are-always-taken-and-a-vector-with-no-handler-ends-the-program.md): both are taken through vectors 4 and 3, and end the program only when the vector holds no handler; a jump to an odd address faults in the jump itself.)* A jump between two instructions, which was the runtime error "Invalid instruction address", runs whatever is there.

## Considered options

- **Halting quietly as EASy68K does**, a pause at every `$FFFF $FFFF`: a Run would stop at the end of every program that does not end itself with nothing to tell it from a `simhalt`, and pressing Run again would pause again two bytes on, through 16 MB of fill.
- **Ending at the last instruction**, as before: it would forbid exactly what code in memory allows, running code the program built or jumped to past its last line, and it would end a program before a `dc.w` it meant to run.
- **Placed meaning code only**, not data: a program that keeps a halt in its data, or runs on into a table ending in `$FFFF`, would end where EASy68K pauses.
- **Reporting the end on the next step**, the fetch that meets it: simpler, but every program that runs off its end would need one more Step to say so, and a limited run whose last instruction ends the program would report its limit instead.

## Consequences

- `Termination::EndOfProgram` carries `{address}`, in Rust and in TypeScript; `has_reached_bottom` (`hasReachedBottom()`) is whether the next step ends the program, and the Interpreter keeps no end address. `Program::end_address` stays one past the last byte of the last instruction, where the code ends and not where a run ends.
- `RuntimeError::InstructionAccess` is gone (ADR 0006), and so are the `OutOfBounds` "Invalid instruction address" and the `Raw` "Program has terminated" a step past the end raised.
- A program that ran off its last instruction into data that follows it now runs the data. One of the corpus programs does, the editor's `sum-of-an-array` lecture, whose run fixture moved (`tests/corpus/README.md`, milestone C4); such a program should end itself, with `simhalt` or task 9, as it would have to in EASy68K.
