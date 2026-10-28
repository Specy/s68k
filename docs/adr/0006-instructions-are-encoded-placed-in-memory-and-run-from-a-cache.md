---
status: accepted
date: 2026-10-28
---

# Instructions are encoded, placed in memory, and run from a cache the assembler fills

s68k interpreted a Program it never encoded. Every instruction was an `Instruction` value four bytes after the one before it, looked up in the Program by address, and no byte of it was in memory. So an address a program computed from its own code, a listing a student compared with EASy68K's and every address table in a course disagreed with a 68000, a program could not read or change its own code, and nothing could be written out for EASy68K to run. The asm-editor's design for a real 68000 behind the M68K Target (its `docs/design/m68k-machine.md`, decisions 4, 5 and 6, confirmed by the owner) asked for the machine EASy68K simulates. We decided three things.

- **Each line is encoded as EASy68K's assembler encodes it, at that encoding's size.** There are two passes and no relaxation, as in EASy68K (`docs/research/easy68k-machine-behaviour.md` §13 there). Every choice between two encodings reads only whether each Symbol in an Operand is defined above the line, EASy68K's back reference, and its value. Pass 1 knows both for every Symbol above, and a Symbol below is never given a short form whatever its value turns out to be, so the size pass 1 measures is the size pass 2 encodes.
    - An unsized `bra`, `bsr` or `b<cc>` is short when its target is above it, within −128 to 127 and not at the next instruction; otherwise it is a word, so a branch forwards is always a word. `.s` and `.b` force a short one and `.w` a word one; `.l` is a word on the 68000, which has no long branch.
    - An unsized absolute address is `.w` when it is above the line and a word holds it, and `.l` otherwise.
    - `move.l #n,dn` of −128 to 127 is `moveq`, and an `add`, `addi`, `adda`, `sub`, `subi` or `suba` of 1 to 8 is `addq` or `subq`, both only for a back reference; `#n.l` keeps the long form. Where an instruction has two encodings, the first of EASy68K's forms wins.
    - A PC-relative displacement is measured from its own extension word: the instruction's address plus 2, plus 4 for `movem` and `btst #n`, whose mask or bit number comes first.
    - What does not fit its field is an error, with EASy68K's ranges: a short branch out of −128 to 127 or to the next instruction (`short_branch_to_next_instruction`), a word branch or a `DBcc` out of −32768 to 32767, a PC-relative distance out of its field (`value_out_of_range`).
- **The code is in memory.** The Program's image holds each instruction's encoding beside the bytes of `dc`, `dcb` and `incbin`, and `simhalt` is the four bytes `$FFFF $FFFF`. Instructions and word or long data start on even addresses, and the pad byte before them is not written, nor is the room `ds` reserves, as in EASy68K. An instruction keeps its bytes and its lowered `Instruction` side by side, so the code stays distinguishable from the data although both are in memory, and the host reads what a 68000 would hold there.
- **The Interpreter runs from a cache of decoded instructions that the assembler fills**, rather than looking each instruction up in the Program.
    - The cache is kept in 4 KB pages of one slot per word, a page existing only once code is placed or run in it. At the Build the slots are filled from the instructions the assembler has just encoded, so assembled code is never decoded at run time. The decoder runs on a miss only: bytes the assembler did not place as code, or code the program has changed.
    - Every write passes the cache, the program's, a Poke's, undo's and the trap tasks', and clears the slots of the instructions it covers.
    - A write by the program into its code flags the step, and a Run pauses after it, naming what was overwritten; running on runs the program's change. A Preference of the editor turns the pause off. This replaces the error a load or store into code was.

The first two are milestone C3 of the asm-editor's `docs/design/m68k-machine-plan.md`, done on 2026-10-28; the cache is milestone C4. Until C4 the Interpreter still runs the Program's instruction list and still refuses the program a load or a store that touches a byte of code (`InstructionAccess`), while the host reads those bytes.

## Considered options

- **Four-byte instructions with a size stored per instruction**, as before: the cheapest, and what ADR 0001 left room to replace. Rejected for the reasons above: no address of the hardware, no self-modifying code, no S-record a student could load into EASy68K.
- **Relaxation**, sizing a forward branch short when it turns out to reach, as GNU `as` does: smaller code, but not EASy68K's, so the addresses would differ from the listing EASy68K prints for the same source, which is the one students compare with.
- **Decoding at every fetch** instead of a cache: the least code, but each step would decode again what the assembler already knew, and the gate of the design is that throughput does not fall below today's 10 to 20 M instructions a second.
- **The code in memory, the instruction list run as before**, for good: a program could read its code, but a change to it would not run, which is the state between C3 and C4 and not a place to stop.

## Consequences

- ADR 0001 is amended. Its "Instructions stay 4 bytes each" no longer holds. The two strictnesses it justified by instructions not being in memory stay, for the learner, because each is almost always a mistake: an address forced to `.w` outside −32768 to 32767 is an error, where EASy68K warns and encodes the sign-extended low word, and two placements over the same address are an error, which EASy68K does not check.
- `AssembledInstruction` carries `bytes`, its encoding, and its `size` is that encoding's length; `Program::image` is everything the Program writes into memory, code and data, in address order.
- The corpus fixtures print each instruction at its real address, with a branch's size and a word address's `.w`, and the memory hash of a run covers the code; `tests/corpus/README.md` records what moved on 2026-10-28.
- What `$FFFF $FFFF` does where the program placed nothing, the end of a program that does not end itself, is decided with the cache (C4, the design's decision 7): it ends the program there.
