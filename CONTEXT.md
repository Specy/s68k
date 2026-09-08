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
One byte, read and written as Latin-1. A source character with a code above 255 cannot be stored and is an error.
_Avoid_: code point, UTF-8 character

**Current address**:
The address the next byte will be placed at, written `*` inside an Expression.
_Avoid_: location counter, PC (that is the running program's register)

**Entry point**:
The address the program starts running at: the value of the `end` directive's operand, else the address of a Label named `START`, else the first instruction.
_Avoid_: start address, start label (the label is only a convention)

**Forward reference**:
A use of a Symbol above its definition. Allowed in instruction operands and `dc` data; refused where the value decides the layout of the program.

### Assembly

**Assembler**:
The whole front end: it reads the source Files from the Entry file, resolves Symbols, lays the program out and produces the Program and the Diagnostics.
_Avoid_: compiler, semantic checker, pre-interpreter, lexer (each was one stage of the old pipeline)

**Program**:
What the Assembler produces when no Diagnostic is an error: the instructions with their addresses and sizes, the initial contents of memory, the Symbols and the Entry point, ready for the Interpreter.
_Avoid_: compiled program, Compiler (the old type name), binary

**Layout**:
The assignment of an address to every line that produces bytes or an instruction, following `org`, alignment and sections. Two lines laid out over the same address are an error.

**Interpreter**:
What runs a Program: registers, memory, the step, run, undo and interrupt operations. It never reads source; it reaches the source only through each instruction's Location.
_Avoid_: emulator (that is the asm-editor's object around it), simulator, CPU

**Paused**:
The Interpreter status after it executes `simhalt`. The Program counter already
names the following instruction, and the next step or run operation resumes
from it. A Paused Interpreter has not terminated.

### Diagnosis

**Diagnostic**:
A finding about the source made while assembling, of a stable kind, at a Location, with a message, optionally a Hint and related Locations, tagged with a severity: `error` stops the program from building, `warning` and `suggestion` are reported while it still builds. The same term as in the asm-editor, which displays them.
_Avoid_: SemanticError, compile error, lexer error, linter message

**Location**:
Where in the source something is: a file, a line and the range of columns of the token or operand concerned. Two lines of a File included twice share one Location and are told apart by their Include chain.
_Avoid_: line index (a Location is more than a line), position (that is the index in the Assembled sequence, which is a different thing)

**Runtime error**:
A failure of the running program, produced by the interpreter and attributed to the instruction's Location. Not a Diagnostic.

**Hint**:
The part of a Diagnostic that says what to do about it, as opposed to what is wrong ("add a colon if `clr` is meant as a label").

### Reference

**EASy68K**:
The assembler and simulator whose dialect s68k follows. Where s68k is more lenient than EASy68K it stays so; where it is stricter, that is a deliberate deviation and is recorded as a decision.
