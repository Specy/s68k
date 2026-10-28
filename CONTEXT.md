# s68k

The M68K assembler and interpreter behind the asm-editor's M68K language: it turns a student's source into a runnable program and explains, in the language of the 68000 and of EASy68K, what is wrong when it cannot. It exists to teach, so it favours a clear diagnosis over an exact reproduction of the hardware.

## Language

### Source

**Source line**:
One line of a source file, read as up to four fields in order: label, operation, operand and comment. Assembly is line oriented; no construct spans lines.

**Label**:
A name for the address of the line it is on. It is declared by an identifier that starts in column 1 and is not a mnemonic or directive name, or by an identifier followed by a colon anywhere on the line. The colon is not part of the name.
_Avoid_: symbol (a Label is one kind of Symbol), tag

**Operation**:
The field that says what the line does: a mnemonic, a directive name or, later, a macro name, with an optional size suffix. It follows the label, or leads the line when there is none.
_Avoid_: opcode (that is the encoded instruction), command

**Mnemonic**:
The name of a 68000 instruction as written in source (`move`, `dbra`, `bsr`). Case insensitive.

**Directive**:
A command to the assembler rather than to the processor (`org`, `dc`, `equ`, `end`, `include`). Case insensitive.
_Avoid_: pseudo-instruction, pseudo-op

**Operand field**:
The comma separated operands of the operation. It ends at whitespace that does not sit beside a comma, at `;`, or at the end of the line.

**Operand**:
One argument of an operation: an Addressing mode with its registers and Expression, a Register list, or one of the special registers SR, CCR and USP. Parsed on its own, judged against the instruction.

**Addressing mode**:
One of the 68000's ways of naming an operand's value or place: data register, address register, indirect, postincrement, predecrement, displacement, index, PC-relative displacement, PC-relative index, absolute short, absolute long and immediate. Which ones an instruction accepts, and in which position, is what the analyzer checks.
_Avoid_: operand type, argument kind

**Comment field**:
Whatever follows the operand field. It is explicit when it starts with `;` or `*`, and implicit otherwise, in which case it is EASy68K's bare comment, which s68k accepts.

**Comment line**:
A line whose first non-blank character is `*` or `;`.

### Files

**File**:
One named entry of a Project, text or bytes, addressed by a root-relative path with `/` separators, the same notion as the asm-editor's File. The Assembler sees only Files: text Files through `include`, either kind through `incbin`.
_Avoid_: document, module, unit, asset

**Entry file**:
The File the Assembler starts from; every other File is reached from it through `include` or is not assembled at all. Its `end` directive is the only one allowed.
_Avoid_: main file, root file

**Include**:
The insertion of a File's lines at the `include` line of another, as if pasted there: same section, same current address, one Symbol namespace, Local label scopes running across the boundary. A File may be included more than once; a cycle is an error.

**Incbin**:
The insertion of a File's bytes, untouched, at the current address, as if written with `dc.b`.

**Include chain**:
The sequence of `include` lines that led to a given line, from the Entry file down. Every Diagnostic in an included File carries it, and so does every instruction assembled from one.

**Assembled sequence**:
The lines the Assembler assembles, in order: the Entry file's, with every `include` line followed by the lines of the File it names, recursively. It is what a textual Include means, written down.

**Position**:
An index into the Assembled sequence, which is what "above" and "below" mean in a Project of several Files: a Variable sees the latest definition above it by Position, and a Register list has to be defined above the `movem` that reads it by Position. A line index alone cannot say it, since a File may be included twice.

### Symbols and expressions

**Symbol**:
A name with a value known at assembly time. There are four kinds: Label, Constant, Variable and Register list. Names are case sensitive.

**Constant**:
A Symbol defined once with `equ`; defining it again is an error.
_Avoid_: equate, define, alias

**Variable**:
A Symbol defined with `set`, which may be redefined; each use sees the latest definition above it.

**Register list**:
A Symbol defined with `reg` that stands for a `movem` register list and cannot appear in an Expression.

**Global label**:
A Label whose name does not start with a dot. It is visible from the whole program and bounds the scope of the Local labels that follow it.

**Local label**:
A Label whose name starts with a dot, visible only between the Global label above it and the next one. The same local name may be reused under different Global labels.

**Expression**:
A value computed at assembly time from numbers, character literals, Symbols and EASy68K's operators, with EASy68K's precedence. It contains no whitespace.

**Character**:
One byte, read and written as Windows-1252, the code page EASy68K runs in: Latin-1 with `€`, the typographic quotes and dashes and a few more in place of the control codes `$80` to `$9F`. A source character Windows-1252 has no byte for cannot be stored and is an error; the text tasks display bytes decoded the same way, and a typed character with no byte is stored as `?`.
_Avoid_: code point, UTF-8 character

**Current address**:
The address the next byte will be placed at, written `*` inside an Expression.
_Avoid_: location counter, PC (that is the running program's register)

**Entry point**:
The address the program starts running at: the value of the `end` directive's operand, else the address of a Label named `START`, else the first instruction.
_Avoid_: start address, start label (the label is only a convention)

**Forward reference**:
A use of a Symbol above its definition. Allowed in instruction operands and `dc` data; refused where the value decides the layout of the program.

**Back reference**:
An Expression every Symbol of which is defined above it by Position, or on its own line, as EASy68K defines a line's Label before it reads the Operands: EASy68K's `backRef`. A number, a character literal and `*` are always one. It is what EASy68K's choices of encoding read beside the value — a short branch, an absolute address in a word, `moveq`, `addq` and `subq` — so a Forward reference is always given the long form, and both passes choose the same size.
_Avoid_: backward branch (that is where a branch goes, not when its target is defined)

### Assembly

**Assembler**:
The whole front end: it reads the source Files from the Entry file, resolves Symbols, lays the program out and produces the Program and the Diagnostics.
_Avoid_: compiler, semantic checker, pre-interpreter, lexer (each was one stage of the old pipeline)

**Program**:
What the Assembler produces when no Diagnostic is an error: the instructions with their addresses, sizes and encodings, the initial contents of memory, the Symbols, the Entry point and the Processor dialect, ready for the Interpreter.
_Avoid_: compiled program, Compiler (the old type name), binary

**Layout**:
The assignment of an address to every line that produces bytes or an instruction, following `org`, alignment and sections. An instruction takes the bytes of the encoding EASy68K's assembler chooses for it, decided once in the first pass from Back references, so the second pass encodes it at the size it was laid out at. Two lines laid out over the same address are an error.

**Memory map**:
The ranges a Program's `memory` lines declare, EASy68K's: ROM, where a write is ignored; read only, where a write is a bus error; protected, where any access from user mode is one; and invalid, where any access is one. One range of each kind at most, both ends in the 24 bits of the address bus and both inside it. The Program carries the map, and the Interpreter is what will enforce it; the debugger is not the program, so a Poke and the host's reads will pass it.
_Avoid_: memory regions (that is the asm-editor's word for the parts of the memory view), protection

**Image**:
Everything a Program writes into memory: each instruction's encoding, `simhalt`'s `$FFFF $FFFF` among them, and the bytes of `dc`, `dcb` and `incbin`. These are the bytes the Program **placed**. A pad byte before an aligned item and the room `ds` reserves are not part of it, so they keep the fill memory starts with, and are not placed.
_Avoid_: binary, object code (that is EASy68K's word for the S-record file)

**S-record**:
A Program written as the `.S68` file EASy68K's assembler writes and its simulator loads: a header, the Image in records of up to 32 bytes, the Memory map and the Entry point, each record a line of hexadecimal with its own checksum. It is written and not read back.
_Avoid_: object file, hex file, binary

**Interpreter**:
What runs a Program: registers, memory, the step, run, undo and interrupt operations. It runs what memory holds, through a decode cache: the instructions it has decoded, kept by address, filled from the Program's own when it is made, and cleared wherever a write changes their bytes, so a changed instruction is decoded again before it runs. It never reads source; it reaches the source only through the Program's map of the lines it laid out, which does not move when bytes change, so a changed line keeps its Location.
_Avoid_: emulator (that is the asm-editor's object around it), simulator, CPU

**Execution step**:
One entry of the Interpreter's history: an instruction that ran, or a Poke. It carries an id of its own, the mutations undo replays, the program counter and the status register before it, the Code write an instruction made, and says which of the two kinds it is.
_Avoid_: history entry, undo record

**Mutation**:
One thing an Execution step changed: a register write, a memory write at a size, a run of bytes, or a call pushed or popped. Every write carries the value it replaced beside the value it wrote, both read where the write happened — a register write reports the whole register on both sides, because a sized store changes only part of one — so nothing about what a step did is reconstructed after the fact.
_Avoid_: diff, change record, delta

**Poke**:
A register or memory value the host — the asm-editor's user or coding agent — changes between two instructions. Everything written between `begin_poke` and `end_poke` is one Execution step of its own, undone by the Interpreter like an instruction; the same setters called outside a transaction write directly and record nothing, which is what a Testcase's preset values need.
_Avoid_: patch, edit, host write (that is any write from outside, Poke or not)

**Paused**:
The Interpreter status after it executes `simhalt`, which is `$FFFF $FFFF` inside the bytes the Program placed. The Program counter already
names the following instruction, and the next step or run operation resumes
from it. A Paused Interpreter has not terminated.

**Code write**:
A write of the program's own, a store or a Trap task's answer on its behalf, into the bytes of an instruction the Interpreter has decoded. The Execution step records the first instruction it overwrote, by address and by the line laid out there, and a run can be asked to stop after it; the overwritten instruction then runs as its new bytes decode. A Poke, a Testcase's starting memory and undo write code too, and are never a Code write.
_Avoid_: self-modifying code (that is the program's purpose, not the event), code corruption

**Watched range**:
The one range of addresses, its start included and its end not, whose stores the Interpreter records for the host, for a device the host keeps in memory. Every store with a byte in it is a Watched write.
_Avoid_: watchpoint (a debugger's stops the user; this one stops for the host), memory observer

**Watched write**:
One store into the Watched range: its address, its size and the value stored, kept in order until the host takes it. The instruction that made one has finished and ends the run or step that ran it with the status `WatchedWrite`; the next step or run operation goes on, as after a Pause. Undo empties the log, so the host reads the range back.

**Trap task**:
One of EASy68K's `trap #15` services, chosen by the number in D0.B. The Interpreter carries some out itself — the Input settings, a file result it can decide alone — and raises an Interrupt for the rest; a task it does not carry out, or one given a value it cannot take, is a Runtime error.
_Avoid_: syscall, service call

**Interrupt**:
What a Trap task asks the host for, with its arguments decoded: text to display, a line or a key to read, a file operation, a sound. The instruction is not finished until the host answers it, with the answer of the same name or with Terminate; any other answer is refused and the Interrupt still waits.
_Avoid_: callback, event, request (in prose, for the Rust type)

**Input settings**:
How the read tasks show what is typed: the echo of task 12, and the input prompt and the line feed after Enter of task 16. They are the Interpreter's state, all on when a program starts, journaled like a register so that undo puts them back, and read by the host while a read task waits.

**Termination**:
Why a program ended: task 9, running into `$FFFF $FFFF` in memory the Program never placed (the end of a program that does not end itself, with its address), the host answering Terminate, or the Runtime error it ended with. Undoing the step that ended the program takes it back.

### Diagnosis

**Diagnostic**:
A finding about the source made while assembling, of a stable kind, at a Location, with a message, a Hint, optionally related Locations and optionally Suggested fixes, tagged with a severity: `error` stops the program from building, `warning` and `suggestion` are reported while it still builds. The same term as in the asm-editor, which displays them.
_Avoid_: SemanticError, compile error, lexer error, linter message

**Suggested fix**:
A change to the source a Diagnostic carries for an editor to apply, the Hint made into an edit: a title, a Location and the text that replaces what it covers, where a Location with no columns inserts. A refused `extb.l` carries the `opt p=68020` line that writes the program for the 68020 and the `ext.w` and `ext.l` that do the same on a 68000.
_Avoid_: quick fix (that is the editor's offer of one), code action, autofix

**Location**:
Where in the source something is: a file, a line and the range of columns of the token or operand concerned. Two lines of a File included twice share one Location and are told apart by their Include chain.
_Avoid_: line index (a Location is more than a line), position (that is the index in the Assembled sequence, which is a different thing)

**Runtime error**:
A failure of the running program, produced by the interpreter and attributed to the instruction's Location. It ends the program with an exception, as there are no exception vectors to take, and is the program's Termination. It carries a Hint but is not a Diagnostic.

**Hint**:
The part of a Diagnostic or Runtime error that gives the learner a safe next action, as opposed to what is wrong. It gives a correction when the likely intent is clear, alternatives when it is ambiguous, or a concrete investigation step when no correction can be inferred.

### Reference

**EASy68K**:
The assembler and simulator whose dialect s68k follows. Where s68k is more lenient than EASy68K it stays so; where it is stricter, that is a deliberate deviation and is recorded as a decision.

**Processor dialect**:
The processor a Program is written for and its words are decoded for: the 68000, EASy68K's processor and the default, or the 68010 or the 68020, each of which adds instructions, addressing modes and behaviour to the one before it. A Program chooses it in its source with `opt p=68010` or `opt p=68020`, once and before its first instruction, and `opt bit` adds the 68020's bit fields to it (ADR 0011). Every form of the instruction table and every row of the decoder names the first processor that has it; on an earlier processor the form is refused and its words are an illegal instruction: `move ccr,<ea>` is the 68010's and `extb.l` the 68020's. Those two are the whole of the dialects so far; every other instruction of the 68010 and the 68020 is not implemented yet.
_Avoid_: CPU type, architecture, target (that is the asm-editor's word for a language and its machine)

**Conformance suite**:
SingleStepTests/m68000, generated from MAME's microcoded 68000: tests of one instruction each, the state before it, the state after it and its bus cycles. It is the oracle of what the Interpreter does with an instruction (ADR 0010), and a test that disagrees with it is a bug unless the Skip list says why. Musashi's self-checking programs are its second check, whole programs that report their own verdict.
_Avoid_: test vectors, Harte tests (Tom Harte's set is another suite, at most a local second opinion)

**Skip list**:
The one list of the Conformance suite's tests the Interpreter is not checked on, and of what it is compared on less, each entry with its reason. An entry that waits for a milestone of the machine says which, and goes when that milestone lands.
_Avoid_: allow list, expected failures (a known failure is a bug being fixed, not a skip)
