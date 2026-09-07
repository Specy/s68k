# The corpus

Real M68K programs, kept here as the input side of the golden fixtures of
[`docs/design/assembler-rewrite.md`](../../docs/design/assembler-rewrite.md),
"Tests" items 1 and 2:

1. `editor/` — the 30 programs the asm-editor ships. All 30 assemble on s68k
   1.4.2 with no error, checked on 2026-09-07, and the fixtures record what
   1.4.2 makes of them (the Program: addresses, instructions, Symbols, initial
   memory, Entry point) and where running them gets to, so that the rewritten
   Assembler and the Interpreter behind it can both be held to it.
2. `easy68k/` — the 3 EASy68K original programs. These do not assemble, and
   are not meant to; their fixtures are Diagnostics. On 1.4.2 they raise 88,
   138 and 64 errors respectively, most of them the old checker misreading a
   label in column 1. After the rewrite the only errors left should be the
   "not implemented" ones naming macros, structured control and the
   unsupported trap tasks.

The programs are inputs only. Nothing here is edited to suit s68k: a program
that trips the Assembler is a finding, not a file to fix. The fixtures beside
them change only on purpose, with a note saying what changed and why.

## Provenance

Everything was taken on **2026-09-07** from
[github.com/Specy/asm-editor](https://github.com/Specy/asm-editor) at commit
**`a42cc1dd232b6cbccc0de35eaa52f053558b5604`** (`main` on that date). Each file
below is byte-identical to its source; the checks are recorded in the table.

### `editor/` — 24 lecture playgrounds

| File | Source |
| --- | --- |
| `<lecture>-<n>.asm` | `src/content/m68k/examples/<lecture>/index.md` |

**Extraction rule**: the `<n>`th fenced code block of that `index.md` whose
info string starts with ```` ```m68k ````, block content only, fence lines
dropped, nothing else touched. The info string carries the editor's playground
options after the language (```` ```m68k|playground|console|no-registers ````);
they are display settings and play no part here. Every lecture currently has
exactly one such block, so every file is `-1`; the numbering is in the name so
that a lecture growing a second playground does not renumber the first.

The 24 lectures are `binary-search`, `bit-tricks`, `bouncing-ball`,
`bubble-sort`, `counting-loop`, `drawing-on-the-screen`,
`factorial-and-fibonacci`, `hello-world`, `if-else`, `jump-table`,
`keyboard-control`, `max-of-an-array`, `moving-values`, `multiply-and-divide`,
`number-to-string`, `reverse-a-string`, `snake`, `string-length`,
`subroutine-with-register-arguments`, `subroutine-with-stack-arguments`,
`sum-of-an-array`, `sum-of-two-numbers`, `two-dimensional-array` and
`variables-in-memory`. `src/content/m68k/examples/index.md` is an empty file
upstream and contributes nothing.

### `editor/` — 6 runnable programs

| File | Source |
| --- | --- |
| `bad-apple.x68` | `examples/m68k/bad-apple.x68` |
| `bouncing-ball.x68` | `examples/m68k/bouncing-ball.x68` |
| `flappy-bird.x68` | `examples/m68k/flappy-bird.x68` |
| `graphics-tour.x68` | `examples/m68k/graphics-tour.x68` |
| `keyboard-move.x68` | `examples/m68k/keyboard-move.x68` |
| `mouse-paint.x68` | `examples/m68k/mouse-paint.x68` |

**Extraction rule**: copied verbatim. All six match their upstream git blob
hash exactly, so they carry upstream's line endings (LF) and, in the case of
`bad-apple.x68`, upstream's missing final newline — deliberately kept, because a
source file that does not end in a newline is exactly the kind of edge the
Assembler has to keep handling.

`bad-apple.x68` is 3.3 MB and 6645 lines (the last unterminated, as above),
nearly all of it one `dc.b` blob of video frames. It is by far the largest
program the editor ships and the only one that makes assembly time visible,
which is why it is worth having: on 1.4.2 it takes about 19 seconds to lex,
check and compile in a `cargo test` (unoptimised) build, against 25 to 60
milliseconds for every other program here. A harness that needs a quick pass
can skip it by size.

The design record says "25 lecture playgrounds, 5 runnable `.x68`". The
repository at the commit above has 24 and 6. The total of 30 is right and the
split is not; this README is the accurate one.

### `easy68k/` — 3 EASy68K originals

| File | Source |
| --- | --- |
| `clockDigital.X68` | `examples/m68k/easy68k/clockDigital.X68` |
| `graphicSound.X68` | `examples/m68k/easy68k/graphicSound.X68` |
| `mouseWindowSize.X68` | `examples/m68k/easy68k/mouseWindowSize.X68` |

**Extraction rule**: copied verbatim, converting CRLF to LF. The asm-editor had
already converted them, so no byte changed and all three match their upstream
blob hash.

These came to the asm-editor from the `Examples` folder of the EASy68K
distribution, <http://www.easy68k.com/files/EASy68K.zip> (version 5.16.1),
whose sources are also published at <https://github.com/ProfKelly/EASy68K>.

## Licence of `easy68k/`

The three programs in `easy68k/` are by Charles Kelly and are distributed with
EASy68K under the GNU General Public License; portions are Copyright (C)
2002-2018 Charles Kelly and Tim Larson. The GPL is compatible with this
repository's AGPL-3.0.

They are kept here only as test data — the compatibility reference the
Assembler's Diagnostics are measured against — and are not part of the
published crate or of the `@specy/s68k` npm package: the npm tarball is the
`files` allowlist of `ts-lib/package.json`, which does not include `tests/`.

## Fixture format

Beside each program is its **fixture**, the golden record of what the Assembler
makes of it: `snapshots/<stem>.snap` for an `editor/` program, and
`snapshots/<stem>-errors.snap` for one of the three `easy68k/` originals, which
do not assemble and whose fixture is the list of their Diagnostics instead
("Diagnostics fixtures" below is its format). Each
`editor/` program has a second fixture, `snapshots/<stem>-run.snap`, of what it
does when it runs; "Execution fixtures" below is its format.
`src/test/corpus.rs` writes them, `cargo test` checks them, and
`INSTA_UPDATE=always cargo test corpus` rewrites them when a change to them is
the point.

The format below is the contract, not the current implementation: it names no
Rust type, field or internal convention, so a different Assembler for the same
language can produce the same bytes. Read it as the specification and
`src/test/corpus.rs` as one implementation of it.

### Shape

```json
{
  "entry": "$1000",
  "instructions": [ { "address": "$1000", "line": 3, "text": "move.l #$a,d0" } ],
  "memory": [
    { "address": "$2000", "bytes": "48656c6c6f00" },
    { "address": "$3000", "zeroed": 4000 }
  ],
  "labels": { "greeting": { "address": "$2000", "line": 12 } }
}
```

* **`entry`** — the Entry point, the address the program starts running at.
* **`instructions`** — one entry per assembled instruction, sorted by address.
  `line` is the 0-based index of the Source line it came from; `text` is the
  canonical rendering below.
* **`memory`** — one entry per Directive that puts bytes in memory (`dc`, `dcb`,
  `ds`), sorted by address. `dc` and `dcb` carry their `bytes`; `ds` carries
  `zeroed`, the number of zero bytes the Program writes there. Every other
  Directive (`org`, `equ`, ...) contributes nothing. A memory entry carries no
  `line`, unlike an instruction and a Label: the Program of the version these
  fixtures were taken from keeps no Source line for a data Directive, and phase
  0 does not change the Assembler to record one. Two data Directives that swap
  Source lines without moving therefore give the same fixture, and a changed
  `bytes` run has to be found by address. When Directives carry a Location, the
  field is worth adding, and that is a deliberate change of every fixture that
  has a `memory` entry.
* **`labels`** — one entry per Label, by name, with its address and the 0-based
  index of the Source line it is on. Constants (`equ`) are not Symbols in the
  version these fixtures were taken from — their text is substituted before
  parsing — so they never appear here; when they become Symbols that is a
  deliberate change of the fixtures.

### Conventions

* **Addresses** are lowercase hexadecimal with a `$` prefix and no padding,
  grouping or sign: `$0`, `$1000`, `$199e00`.
* **Byte runs** are lowercase hexadecimal, exactly two characters a byte and
  nothing between them, in the order the bytes sit in memory:
  `dc.w 12,-4` is `000cfffc`.
* **Lines** are 0-based, counted over the whole file, blank and comment lines
  included. The one exception is the message text of a Diagnostics fixture,
  which counts from 1 because that is how this version writes it; "Diagnostics
  fixtures" below says so again where it matters.
* **Lists** are sorted by address, stably, so that two entries laid out over the
  same address keep their source order.
* **Maps** are sorted by name in byte order, so `END` comes before `START` and
  both come before `cell_done`.

### `text`, the canonical rendering

`text` is printed from the assembled instruction and never from the Source line,
so the source's spacing, case, comments and Label names are all gone and two
lines that assemble to the same thing read the same. The shape is

```
mnemonic[.size] source[,destination]
```

with one space after the Mnemonic, a comma and no space between Operands, and
nothing else. Everything is lowercase.

**Size suffix.** `.b`, `.w` or `.l`, and it is always written for an instruction
that carries a size, whether or not the source wrote one — an instruction the
source left unsized shows the default the Assembler chose, not a blank. The
sized instructions are `move`, `movea`, `movem`, `add`, `adda`, `addi`, `addq`,
`sub`, `suba`, `subi`, `subq`, `cmp`, `cmpa`, `cmpi`, `cmpm`, `and`, `andi`,
`or`, `ori`, `eor`, `eori`, `not`, `neg`, `clr`, `tst`, `ext`/`extb`,
`asl`/`asr`, `lsl`/`lsr` and `rol`/`ror`. The unsized ones carry no suffix at
all: `moveq`, `divs`, `divu`, `muls`, `mulu`, `swap`, `exg`, `lea`, `pea`,
`link`, `unlk`, `jmp`, `jsr`, `bsr`, `bra`, every `Bcc`, `DBcc` and `Scc`,
`btst`, `bset`, `bclr`, `bchg`, `trap`, `rts` and `nop`. Branches are among them
because the version these fixtures were taken from stores no branch size.

**Registers.** `d0` to `d7` and `a0` to `a7`. The stack pointer is `a7`; `sp`
never appears, so `move.l sp,a5` prints as `movea.l a7,a5`.

**Operands.**

| Addressing mode | Printed | Example |
| --- | --- | --- |
| Immediate | `#$` and the stored value in hex, 32-bit as an Operand and 8-bit for a quick form's own count (below) | `#$a` |
| Data or address register | the register | `d3`, `a6` |
| Indirect | the register in parentheses | `(a0)` |
| Postincrement | | `(a0)+` |
| Predecrement | | `-(a0)` |
| Displacement | signed decimal displacement, then the base register | `-8(a6)`, `0(a6)` |
| Index | signed decimal displacement, base, index register and its size | `4(a6,d1.w)`, `0(a0,a2.l)` |
| Absolute | `$` and the address in hex | `$2000` |

The displacement of a displacement or index Operand is always written, `0`
included, which is what tells `0(a6)` apart from `(a6)`. An index Operand always
carries its `.w` or `.l`, the default included.

**Immediates** print as the unsigned value the Assembler stored, in hex, and it
is the width the Assembler stored it at, not the size of the instruction, that
decides how many digits come out. An immediate Operand is stored in 32 bits, so
a negative source value comes out as its two's complement whatever size the
instruction carries: `move.w #-1,d2` prints as `move.w #$ffffffff,d2`. The
exception is `moveq`, `addq`, `subq` and `trap`, whose count or vector is not an
Operand but part of the instruction itself and is stored in eight bits by the
version these fixtures were taken from: the same `-1` truncates, so
`moveq #-1,d3` in `editor/binary-search-1.asm` prints as `moveq #$ff,d3` while
`link a6,#-4` in `editor/factorial-and-fibonacci-1.asm`, whose displacement is a
32-bit field, prints as `link a6,#$fffffffc`. `printer_rules` in
`src/test/corpus.rs` holds `moveq #-1,d0` and `move.l #-1,d1` side by side.

**Branch and jump targets** print as absolute addresses, `$` and hex, because a
Label is resolved to its address before the instruction is stored: `bra done`
prints as `bra $1050`.

**Condition codes** print as the canonical Mnemonic suffix and nothing else:
`t`, `f`, `hi`, `ls`, `cc`, `cs`, `ne`, `eq`, `vc`, `vs`, `pl`, `mi`, `ge`,
`lt`, `gt`, `le`. The aliases have no printed form of their own, so `bhs` prints
as `bcc`, `blo` as `bcs`, `shs` as `scc`, `slo` as `scs` and `dbra` as `dbf`.

**`movem` register lists** print in canonical form, `d0-d2/a0/a6`: lowest
register first, every data register before any address register, two or more
consecutive registers of the same kind written as a range (`d0-d1`, never
`d0/d1`), a range never crossing from `d7` into `a0`, groups joined by `/` with
no spaces. The direction decides which Operand comes first: the list, when the
registers go to memory (`movem.l d0-d2/a0/a6,-(a7)`), the memory Operand, when
they come back (`movem.l (a7)+,d0-d2/a0/a6`). An empty mask prints as an empty
Operand. A `movem` into a predecrement stores its mask reversed, because that is
the order the registers are written in; the printer undoes that, so the list
reads as it was written in source.

**Normalisations stay visible.** The Assembler rewrites some instructions as it
stores them, and the fixture shows what it stored:

| Written | Printed |
| --- | --- |
| `add #n,X` | `addi.<size> #$n,X`, X an address register included |
| `sub #n,X` | `subi.<size> #$n,X`, likewise |
| `cmp #n,dn` | `cmpi.<size> #$n,dn` |
| `cmp X,an` | `cmpa.<size> X,an`, and this beats the `cmpi` rule above |
| `add X,an` | `adda.<size> X,an` |
| `sub X,an` | `suba.<size> X,an` |
| `move X,an` | `movea.<size> X,an` |
| `asl X` and the other one-operand shifts | `asl.w #$1,X`: an explicit count of one and the word size |
| `ext.w dn`, `ext.l dn` | the same, `ext` printing with its destination size |
| `extb.l dn` | `extb.l dn`, the byte to long form of the same instruction |
| `dbra dn,label` | `dbf dn,$...` |

A printer written to these rules must match over every instruction and every
Addressing mode with no catch-all arm, so that a new one has to be given a
rendering rather than being printed wrong.

### Diagnostics fixtures

The three `easy68k/` originals do not assemble, so their fixture is the list of
Diagnostics the checker raises instead: `snapshots/<stem>-errors.snap`, a JSON
array of its messages, verbatim, one entry per error, in the order it reports
them.

```json
[
  "Error on line 42: Unknown instruction: \"start\"",
  "Error on line 44: Invalid immediate: Invalid decimal number or non existing label: versionTrap, invalid digit found in string"
]
```

Two things about it are the version's and not the format's. The line numbers
inside the message text are **1-based**, unlike the 0-based `line` fields of the
assembly fixture above; and some messages carry Rust standard library text
(`invalid digit found in string`, `cannot parse integer from empty string`) that
reached the message through a failed number parse. Both are recorded exactly as
they are, because a fixture is a record of what this version does and not of
what it should say.

There is no shape to hold on to here beyond that. The whole array is expected to
be replaced when Diagnostics become structured, carrying a Location, a code, a
Severity and a Hint rather than one formatted string, and the count is expected
to fall from 88, 138 and 64 to the "not implemented" errors alone. A total
rewrite of these three files is the intended outcome of the rewrite, not a
regression.

### What the 1.4.2 fixtures record that the rewrite will change

These are behaviours of the version the fixtures were taken from, not rules to
keep. They are listed so that a snapshot that changes here is recognised as the
fix it is rather than a regression:

* **`ds` writes the wrong number of zeros.** `zeroed` is an eighth of the space
  the Directive reserves, rounded down, although the Layout does reserve the
  full space: `ds.l 1` in `editor/variables-in-memory-1.asm` at `$2008` records
  `0`, `ds.b 34` in `editor/number-to-string-1.asm` at `$2000` records `4`,
  `ds.w 10` in `editor/counting-loop-1.asm` at `$2000` records `2`. It is
  visible at run time: memory starts as `$ff`, so the part of a `ds` block the
  Directive does not zero reads back as `$ff` rather than as `0`.
* **Constants are missing from `labels`.** `equ` is a text substitution, so a
  Constant leaves no Symbol and its value is baked into the instruction:
  `add.l #TAX,d0` in `editor/variables-in-memory-1.asm` at `$1008` prints as
  `addi.l #$14,d0`.
* **Immediates are stored sign extended to 32 bits**, so a word immediate can
  print wider than its instruction: `cmpi.w #$ffffff74,d5` in
  `editor/flappy-bird.x68` and `move.w #$ffffffff,d2` in
  `editor/snake-1.asm`. The quick forms go the other way and truncate to eight
  bits, so `moveq #-1,d3` in `editor/binary-search-1.asm` prints as
  `moveq #$ff,d3`. Both lines move when values are evaluated in 64 bits and
  range-checked against the operand size, as the design record has it.
* **The Entry point is the first instruction in all 30 fixtures**, so no
  snapshot tells the two rules of this version apart: `bad-apple.x68` is the
  only program spelling the Label `START:` and it is on the first instruction
  anyway, and the five other `.x68` write a lowercase `start:` that the
  case-sensitive lookup does not see. `end` is not even a known Mnemonic here,
  so the third source the design gives the Entry point cannot be written. The
  rules are pinned by `entry_point_is_an_uppercase_start_label_or_the_first_instruction`
  in `src/test/corpus.rs` rather than by any fixture, and phase 1 needs its own
  test of the `end expr` / `START` / first-instruction precedence.
* **Every instruction is 4 bytes**, whatever it encodes to on a real 68000, so
  every address in `instructions` steps by 4.
* **`add`/`sub` and `cmp` disagree about which rewrite wins.** `add #1,a0`
  becomes `addi`, `cmp #1,a0` becomes `cmpa`.
* **`extb` cannot be written.** The compiler knows the byte to long `ext` but
  the checker does not know the Mnemonic, so no source reaches it and the
  `extb.l` rendering is only reachable from a hand-built instruction.
* **A one-operand shift may not carry a size.** `asr (a0)` assembles and prints
  as `asr.w #$1,(a0)`; `asr.l (a0)` is refused with "Invalid size, instruction
  is not sized", so the memory form is always word whatever the program meant.

## Execution fixtures

The fixtures above stop where the Assembler stops. Beside each one is the
**execution fixture** of the same program, `snapshots/<stem>-run.snap`: where
the program gets to when it is actually run. That is what guards the
instructions and the Interpreter, which the assembly fixtures say nothing
about — an instruction can be assembled right and executed wrong.

The three `easy68k/` originals have none: they do not assemble, so there is
nothing to run.

### The run

The Program is run with no history kept (the Interpreter's history is a
debugger feature and no part of what a program computes), one instruction at a
time, answering every interrupt as the policy below says, until it terminates,
ends with an exception, or reaches a total of **200 000 executed instructions**.

`status` says which of the three happened:

| `status` | Meaning |
| --- | --- |
| `terminated` | the program stopped on its own, almost always the Terminate task |
| `exception` | the Interpreter stopped it: an address error, an unknown trap task, an instruction outside the program, a division by zero |
| `limit` | the program was still running after 200 000 instructions |

`steps` is how many instructions were executed, counting the one that raised a
runtime error. An interrupt raised by the last instruction of the budget is
still answered before the run stops. The Terminate task ends the run and is
never answered.

Of the 30 programs, 23 terminate and 7 reach the limit: `bad-apple`,
`bouncing-ball`, `bouncing-ball-1`, `flappy-bird`, `keyboard-control-1`,
`keyboard-move` and `mouse-paint`, every one of them an event loop with no way
out when nothing is ever typed or clicked. None ends with an exception, so that
row of the table is held by `a_runtime_error_ends_the_run_with_an_exception` in
`src/test/corpus.rs` — a division by zero and a jump to an address between two
instructions, each asserted to stop the run as `exception` with the failing
instruction counted in `steps` — and not by any fixture. The
limit is the same 200 000 for every program: none of them needs a lower one, as
the whole corpus test, `bad-apple`'s 20 seconds of assembly included, takes
about half a minute in an unoptimised `cargo test` build.

### The interrupt policy

Every answer is fixed. Nothing here reads the clock, the terminal, the file
system or a random source, so a program gives the same fixture on every machine
and on every run.

**Display tasks** append to `output` and nothing else:

| Task | Appends |
| --- | --- |
| Display string with CR/LF | the string, then a newline |
| Display string without CR/LF | the string |
| Display number | the signed number in decimal |
| Display number in base | the unsigned number in that base, digits `0` to `9` then `a` to `z` |
| Display character | the character |
| Display signed number in field | the signed number in decimal, right justified with spaces in a field of that many columns |
| Display string and number | the string, then the number in decimal |
| Display string and read number | the string; the number it answers is below |

**Input tasks** answer the same thing every time:

| Task | Answer |
| --- | --- |
| Read number | 7 |
| Read character | `a` |
| Read keyboard string | `test` |
| Display string and read number | 7 |
| Get time | 0 |
| Check keyboard input | no input pending |
| Get key state | every key up; last key released 0 and last key pressed 0 |
| Read mouse | flags 0, at 0,0 |
| Get pixel colour | 0 |
| Get pen position | 0,0 |
| Get screen size | 640 by 480 |
| Get text cursor position | column 0, row 0 |
| Delay | returns immediately |

**Every other task is acknowledged with no effect**: the drawing tasks, the pen
and fill colours, the pen width, the drawing mode, the repaint, the screen size
and mode, the text cursor position, the clear screen and the simulator
shortcuts all leave `output` and the registers alone. Text drawn on the screen
is a graphics task, not a display task, so it does not reach `output` either;
neither does clearing the screen clear `output`, which is the whole run in
order and not a picture of a terminal.

### Shape

```json
{
  "status": "terminated",
  "steps": 9,
  "d": ["$9", "$2a", "$0", "$0", "$0", "$0", "$0", "$0"],
  "a": ["$0", "$200e", "$0", "$0", "$0", "$0", "$0", "$1000000"],
  "pc": "$1024",
  "flags": "X:0 N:0 Z:0 V:0 C:0",
  "output": "Hello, world!\nThe answer is 42",
  "memory": "af29d2e028e4536c"
}
```

* **`status`**, **`steps`** — as above.
* **`d`**, **`a`** — the eight data and the eight address registers as longs,
  `d0` to `d7` and `a0` to `a7`, in the same hexadecimal as an address. `a7` is
  the stack pointer, which starts at the top of memory, `$1000000`.
* **`pc`** — where the program counter stopped.
* **`flags`** — the CCR as `X:_ N:_ Z:_ V:_ C:_`, each either `0` or `1`, in
  the order the 68000 lists them.
* **`output`** — everything the display tasks wrote, in order, as one string.
  A program that draws rather than prints has an empty one.
* **`memory`** — FNV-1a 64 of the whole 16 MB address space (offset basis
  `$cbf29ce484222325`, prime `$100000001b3`), as 16 lowercase hexadecimal
  digits. One number that changes when any byte of memory does, without putting
  16 MB in the fixture. When it changes and nothing else has, the diff says a
  program wrote different bytes and gives no more; that is the price of the
  short form, and the assembly fixture beside it holds the initial bytes in
  full.

### What the 1.4.2 execution fixtures record that the rewrite will change

* **Memory starts as `$ff`, not as zeros.** Every byte no Directive and no
  instruction has written reads back as `$ff`, and the memory hash carries that
  fill. Together with the `ds` bug above it is visible to a program: the first
  eighth of a `ds` block reads as zeros and the rest as `$ff`
  (`ds_zeroes_less_than_it_reserves` in `src/test/corpus.rs` is that program).
* **Seven programs never end.** They are event loops, and the policy answers
  "nothing was typed, nothing was clicked" for ever, so `limit` is the honest
  outcome rather than a fault. If the rewrite makes one of them terminate, that
  is a change worth reading, not a snapshot to update blindly.
