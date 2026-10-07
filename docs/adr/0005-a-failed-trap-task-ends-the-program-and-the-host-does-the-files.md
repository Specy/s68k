---
status: accepted
date: 2026-10-06
---

# A failed trap task ends the program, and the host does the files

EASy68K's `trap #15` tasks reach what is outside the program: the terminal, files, sound. Following the asm-editor's rule that each environment matches its reference and that the Core owns what a service means while the host owns the transport, we decided three things about them.

- **A task that cannot be done ends the program**, as every runtime error now does: with an exception and a typed error, `UnsupportedTrapTask` or `InvalidTrapArgument`, kept as the program's termination. Undoing the failed step brings the program back to it, running. Before, a task error left the program running one instruction further on, and so did a division by zero.
- **The host's file system does the files, and the Interpreter decides their results.** Each of tasks 50 to 59 is an Interrupt with its arguments decoded; the host answers what its file system did, and the Interpreter writes EASy68K's result to D0.W. What EASy68K decides before it touches a file — a file number outside 0 to 7, a count of zero, a buffer past the end of memory, a negative position — is decided by the Interpreter, without an Interrupt. The file numbers are the host's, at most eight and the lowest free first, so the Interpreter keeps no handle table of its own.
- **The input settings of tasks 12 and 16 are the Interpreter's state**, journaled for undo like a register, which the host reads while a read task waits.

## Considered options

- Leaving a program running after a task error: the old behaviour, which ran on past a trap that had not happened.
- A handle table in the Interpreter: it could answer a closed file number without asking, but two tables, its own and the host's, would have to agree after every undo.
- Passing a read's bytes through a dedicated WebAssembly method: serde's bytes cross as a `Uint8Array` in one copy each way, inside the one answer object.
