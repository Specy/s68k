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
   are not meant to; their fixtures are Diagnostics. On 1.4.2 they raised 88,
   138 and 64 errors respectively, most of them the old checker misreading a
   label in column 1; they now raise 15, 3 and 19, and every one of those names
   a feature s68k does not implement — macros, structured control, `rte` — with
   one exception recorded below (`END START` against a Label written `start`).

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
which is why it is worth having: on 1.4.2 it took about 19 seconds to lex, check
and compile in a `cargo test` (unoptimised) build, against 25 to 60 milliseconds
for every other program here; the Assembler of the rewrite takes about 2 seconds
over it. A harness that needs a quick pass can skip it by size.

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

**All three kinds of fixture are the rewrite's** since step 8 of
`docs/design/assembler-rewrite.md`: `<stem>.snap` is built from the Assembler's
`Program` (step 7), `-errors.snap` is the Diagnostics the Assembler raises on a
program that does not build (step 8), and `-run.snap` is the Interpreter running
that `Program` (step 8). The old pipeline is gone from the crate, so nothing
here records what 1.4.2 did any more; the two sections below say what moved when
each half changed hands and why.

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
    { "address": "$3000", "reserved": 4000 }
  ],
  "labels": { "greeting": { "address": "$2000", "line": 12 } }
}
```

* **`entry`** — the Entry point, the address the program starts running at.
* **`instructions`** — one entry per assembled instruction, sorted by address.
  `line` is the 0-based index of the Source line it came from; `text` is the
  canonical rendering below.
* **`memory`** — one entry per Directive that puts bytes in memory or reserves
  room for them (`dc`, `dcb`, `ds`), sorted by address. `dc` and `dcb` carry
  their `bytes`; `ds` carries `reserved`, the number of bytes it takes up and
  does not write. Every other Directive (`org`, `equ`, ...) contributes nothing.
  A memory entry carries no `line`, unlike an instruction and a Label. The
  Program of 1.4.2 kept no Source line for a data Directive, so phase 0 had none
  to write; the Program of the rewrite does carry one, and adding the field is
  still a deliberate change of every fixture that has a `memory` entry, which
  phase 1 did not make. Two data Directives that swap Source lines without
  moving therefore give the same fixture, and a changed `bytes` run has to be
  found by address.
* **`labels`** — one entry per Label, by name, with its address and the 0-based
  index of the Source line it is on. Constants (`equ`) are not Symbols in 1.4.2
  — their text is substituted before parsing — so they never appeared here. They
  are Symbols of the rewrite's Program and are **still** not written here: the
  field is the Labels, by name, and a Constant's value is visible in every
  instruction that uses it. Writing the whole symbol table instead is a
  deliberate change of the fixtures and is not one phase 1 made.
  A name written in the label field **inside an `offset` region** is a Constant
  and not a Label — its value is an offset into a structure, no line of the
  program is laid out at it, and it may be negative — so it is not here either;
  the symbol listing of the Program is where it is, and it says `constant`
  (phase 2 of the design record, `Directives/offset.htm`).

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
`addx`, `sub`, `suba`, `subi`, `subq`, `subx`, `cmp`, `cmpa`, `cmpi`, `cmpm`,
`and`, `andi`, `or`, `ori`, `eor`, `eori`, `not`, `neg`, `negx`, `clr`, `tst`,
`ext`/`extb`, `asl`/`asr`, `lsl`/`lsr`, `rol`/`ror`, `roxl`/`roxr` and `movep`.
The unsized ones carry no suffix at all: `moveq`, `divs`, `divu`, `muls`,
`mulu`, `swap`, `exg`, `lea`, `pea`, `link`, `unlk`, `jmp`, `jsr`, `bsr`, `bra`,
every `Bcc`, `DBcc` and `Scc`, `btst`, `bset`, `bclr`, `bchg`, `trap`, `rts`,
`nop`, `simhalt`, `tas`, `chk`, `rtr`, `trapv`, `illegal`, `abcd`, `sbcd`,
`nbcd` and every instruction that names a half of the status register (below).
Branches are among them because the version these fixtures were taken from
stores no branch size, and the three decimal instructions because a byte is the
only size they have, the way `tas` has only a byte.

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
| Absolute | `$` and the address in hex, with no forced width | `$2000` |
| PC-relative displacement | signed decimal displacement, then `(pc)` | `18(pc)`, `-6(pc)` |
| PC-relative index | signed decimal displacement, `pc`, index register and its size | `14(pc,d1.w)`, `0(pc,a2.l)` |
| The status register | `sr` | `move sr,d0` |
| The condition codes | `ccr` | `move d0,ccr` |

The displacement of a displacement or index Operand is always written, `0`
included, which is what tells `0(a6)` apart from `(a6)`. An index Operand always
carries its `.w` or `.l`, the default included.

**A PC-relative Operand prints the displacement the Assembler worked out and
not the address the source wrote.** The source writes the address it wants —
`move.l data(pc),d0` — and the Assembler stores the distance from that
instruction's extension word to it, which is the address of the instruction plus
two, since an instruction is four bytes here. So the same line at `$1000` with
`data` at `$1014` prints as `move.l 18(pc),d0`, and a `data` *behind* the
instruction prints as a negative displacement. The displacement is signed
decimal, like every other displacement, and it is what the Interpreter adds back
to reach the address again. `(pc,d1.w)`, which writes no address at all, is a
displacement of zero and prints as `0(pc,d1.w)`.

**A forced address width is not printed.** `label.w` and `label.l` name the same
address here, and the Program holds the address alone, so `move.l $1000.w,d0`
and `move.l $1000.l,d0` both print as `move.l $1000,d0`. The `.w` form is range
checked while it is assembled (EASy68K's "Absolute short addressing must be in
the range -32768 through 32767") and leaves no trace after that.

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

**The status register** is `sr` and the condition codes are `ccr`, and the four
`move`s and the six immediates that name one carry **no size**: `ccr` is a byte
and `sr` a word by definition, so the operand says the width and the line does
not repeat it. `move.w d0,ccr` prints as `move d0,ccr`, `andi.b #$1f,ccr` as
`andi #$1f,ccr` and `ori.w #$700,sr` as `ori #$700,sr`. The immediate of one of
those is stored at the width of its destination — eight bits for `ccr`, sixteen
for `sr` — and prints as that, where an ordinary immediate Operand is 32 bits.
`usp` never appears: `move usp,an` is not implemented.

**`movep`** prints its size, because a word and a long are a real choice, and
the data register comes first when the bytes go to memory: `movep.w d0,4(a1)`
and `movep.l 4(a1),d0`.

**The extend-flag group** prints as it is written, source first: `addx.l d0,d1`
and `addx.b -(a0),-(a1)`, `subx.w d0,d1`, `negx.l (a0)`, `abcd d0,d1`,
`sbcd -(a0),-(a1)` and `nbcd (a0)`. `roxl` and `roxr` print like the other
shifts, the memory form included, which normalises to an explicit count of one
and the word size: `roxl (a0)` prints as `roxl.w #$1,(a0)`. `printer_rules` in
`src/test/corpus.rs` writes all eleven lines, since no corpus program does.

**`simhalt`** prints as `simhalt` and nothing else. It is a Directive and not a
68000 instruction, and it is in `instructions` because it is an executable item
of the Program: four bytes at its own address, which end the run
(`Directives/simhalt.htm`, and phase 2 of the design record).

**Branch and jump targets** print as absolute addresses, `$` and hex, because a
Label is resolved to its address before the instruction is stored: `bra done`
prints as `bra $1050`.

**Condition codes** print as the canonical Mnemonic suffix and nothing else:
`t`, `f`, `hi`, `ls`, `cc`, `cs`, `ne`, `eq`, `vc`, `vs`, `pl`, `mi`, `ge`,
`lt`, `gt`, `le`. The aliases have no printed form of their own, so `bhs` prints
as `bcc`, `blo` as `bcs`, `shs` as `scc`, `slo` as `scs` and `dbra` as `dbf`.

A **`reg` register list** has no printed form of its own: `movem.l AllRegs,-(a7)`
prints exactly as the list `AllRegs` stands for would, because the Assembler
lowers it to the same mask. `printer_rules` in `src/test/corpus.rs` writes the
same `movem` twice, once with a `reg` name and once with the list, and asserts
that the two print the same line.

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
| `asl X` and the other one-operand shifts, `roxl` and `roxr` included | `asl.w #$1,X`: an explicit count of one and the word size |
| `ext.w dn`, `ext.l dn` | the same, `ext` printing with its destination size |
| `extb.l dn` | `extb.l dn`, the byte to long form of the same instruction |
| `dbra dn,label` | `dbf dn,$...` |

A printer written to these rules must match over every instruction and every
Addressing mode with no catch-all arm, so that a new one has to be given a
rendering rather than being printed wrong.

### Diagnostics fixtures

The three `easy68k/` originals do not assemble, so their fixture is the list of
Diagnostics the Assembler raises instead: `snapshots/<stem>-errors.snap`, a JSON
array of the Diagnostics themselves, in source order — the same objects the
TypeScript side receives, and the same shape the cases in `tests/diagnostics/`
are snapshotted as.

```json
[
  {
    "severity": "error",
    "code": "unimplemented_operation",
    "message": "`simhalt` is not implemented: halting the simulator is not implemented yet",
    "hint": "write `move.b #9,d0` and `trap #15` to end the program instead",
    "location": {
      "file": "graphicSound.X68",
      "line": 205,
      "column": 4,
      "endColumn": 11
    },
    "related": []
  }
]
```

* **`severity`** — `error`, `warning` or `suggestion`. Only an `error` stops the
  Program from being built.
* **`code`** — the stable snake_case name of the kind, which is what a test or
  an editor matches on; the message is for a person to read.
* **`hint`** — what to do about it, or `null`.
* **`location`** — the File, the 0-based line and the 0-based column range,
  `endColumn` exclusive. Unlike the messages of the fixtures this file used to
  hold, nothing here counts from 1. The key is camelCase because a Location is
  written the same way everywhere, and the TypeScript side is where it is read
  (the design record, "Public API"); step 9 renamed it from `end_column`, which
  is the only difference between these files and the ones step 8 wrote.
* **`related`** — `{ location, message }` pairs, the other places the finding is
  about: the first definition of a name defined twice, the line an address was
  already used by.

These fixtures were rewritten whole in step 8, as this section always said they
would be: on 1.4.2 they were 88, 138 and 64 formatted error strings, nearly all
of them the old checker misreading a Label in column 1, and they are now 16, 4
and 20 structured Diagnostics naming the features s68k does not implement. The
same finding counted by code is
`the_easy68k_originals_raise_only_what_is_not_implemented` in
`src/test/diagnostics.rs`, which is what fails first when one of them moves.

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
  `moveq #$ff,d3`. *(This one turned out not to move: phase 1 evaluates values
  in 64 bits and range-checks them against the operand size, and then stores the
  immediate in the same 32 bits and the quick form's count in the same 8, so
  every one of these lines prints exactly as it did.)*
* **The Entry point is the first instruction in all 30 fixtures**, so no
  snapshot tells the two rules of this version apart: `bad-apple.x68` is the
  only program spelling the Label `START:` and it is on the first instruction
  anyway, and the five other `.x68` write a lowercase `start:` that the
  case-sensitive lookup does not see. `end` is not even a known Mnemonic here,
  so the third source the design gives the Entry point cannot be written. The
  rules are pinned by
  `entry_point_is_end_then_an_uppercase_start_label_then_the_first_instruction`
  in `src/test/corpus.rs` rather than by any fixture — phase 1 renamed that test
  and gave it the `end expr` case, which 1.4.2 could not even parse.
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

### What phase 1's Assembler changed in these fixtures, and why

Phase 1 replaced the pipeline that wrote the `<stem>.snap` fixtures. **Eight
entries of five fixtures moved, and nothing else did**: every instruction, every
address, every byte of `dc` and `dcb` data, every Label, every Entry point and
every `line` came out identical to 1.4.2 over all 30 programs.

**What moved** — the `ds` bug of the list above, fixed, and the key that carries
it:

| Fixture | Was | Is |
| --- | --- | --- |
| `counting-loop-1` `$2000` | `zeroed: 2` | `reserved: 20` |
| `flappy-bird` `$1925`, `$1955`, `$195d` | `zeroed: 6`, `1`, `0` | `reserved: 48`, `8`, `1` |
| `number-to-string-1` `$2000` | `zeroed: 4` | `reserved: 34` |
| `snake-1` `$3012`, `$30b0` | `zeroed: 15`, `1` | `reserved: 122`, `8` |
| `variables-in-memory-1` `$2008` | `zeroed: 0` | `reserved: 4` |

`ds` reserves its room and writes nothing at all now, which is what
`Directives/ds.htm` says it does ("unlike DC, no data is stored in the reserved
memory"), so the count is the whole block and the key is `reserved` rather than
`zeroed`: there are no zeros to count. The key had to change with the meaning —
a `reserved: 20` written as `zeroed: 20` would say the Program writes twenty
zeros there, which is exactly what it stopped doing.

**What the running program sees** is the other half of it, and step 8 is where
it moved: see "What step 8's Interpreter changed" below.

**Rules that changed and that no corpus program exercises.** These moved no
fixture, and are here so that the next one to move is recognised:

* **A word or a long data Directive is aligned.** `dc.w`, `dc.l`, `ds.w`,
  `ds.l`, `dcb.w` and `dcb.l` start on an even address, padding what comes
  before them, which is what `Directives/dc.htm` and `Directives/ds.htm`
  specify and what 1.4.2 did not do at all. No corpus program writes an odd
  number of bytes before a word one, so nothing moved; a program that does will
  see its data move up by a byte, and every Label after it with it.
* **Constants are Symbols.** `equ` is no longer a text substitution, so
  `x equ 5` no longer rewrites `next` into `ne5t`; the Constants are in the
  Program's symbol table and out of `labels`, as above. Every corpus program's
  Constants still reach the instructions that use them with the same values —
  `addi.l #$14,d0` for `add.l #TAX,d0` — which is why no fixture moved.
* **The Entry point can be written.** `end START` is read now, where 1.4.2
  answered "Unknown instruction: end". No corpus program writes `end`, so
  `entry` is unchanged in all 30;
  `entry_point_is_a_start_label_of_either_case_then_the_first_instruction`
  in `src/test/corpus.rs` is what holds the three sources apart.
* **A Local label's full name is `global:local`**, EASy68K's own
  (`quickStart.htm`, "Label Field"). No corpus program writes one.

**The Entry point is the one name read case insensitively**, which is what
`mouseWindowSize.X68` forced: it writes its Label as `start` and its last line as
`END START`, and Symbols are case sensitive here (CONTEXT.md, "Symbol"). That
file is byte-identical to the EASy68K distribution, so it is the primary
evidence that EASy68K's own look-up is not case sensitive, and ADR 0001 promises
that an EASy68K program assembles here unchanged. `end`'s operand and the
`START` fallback are therefore resolved by exact match first and then by an
ASCII-case-insensitive match over the Labels; the `end` form warns
(`entry_point_case_mismatch`) and the fallback, whose name is s68k's own
convention rather than a word the program wrote, says nothing. Every other use
of a name is compared exactly, and two Labels differing only in case are not
guessed between. The decision is recorded in ADR 0001; before it,
`mouseWindowSize.X68` was the one original that did not assemble.

### What step 8's Interpreter changed in these fixtures, and why

Step 8 gave the Interpreter the `Program` and deleted the old pipeline, which is
what moved the other two kinds of fixture. **Three `-run.snap` fixtures moved,
one line each, and it is the `ds` fix of the table above**:

| Fixture | What moved |
| --- | --- |
| `flappy-bird-run` | `memory` only |
| `number-to-string-1-run` | `memory` only |
| `snake-1-run` | `memory` only |

In all three the `status`, the `steps`, the sixteen registers, the `pc`, the
`flags` and the `output` are identical to what 1.4.2 left. Only the hash of
memory changed, and it changed because a `ds` block now keeps the `$ff` fill
where 1.4.2 wrote zeros over its first eighth. The other 27 programs, the seven
that reach the limit included, are byte for byte unchanged.

Two of the five programs with a `ds` in them did **not** move, and the reason is
worth keeping: `counting-loop-1` fills all twenty bytes of its `ds.w 10` before
it ends, so no byte of the block is left to differ; and `variables-in-memory-1`
reserves four bytes with `ds.l 1`, of which 1.4.2 zeroed an eighth rounded down,
which is none.

`ds_reserves_memory_without_writing_it` in `src/test/corpus.rs` is the program
that reads a `ds` block back byte by byte: it printed `0,255` on 1.4.2 and
prints `255,255` now.

**The `-errors.snap` fixtures were rewritten whole**, as "Diagnostics fixtures"
above says they would be: 88, 138 and 64 strings of the old checker became 16, 4
and 20 structured Diagnostics of the Assembler.

**Nothing else about a run changed.** The Interpreter reads the `Program` where
it read the compiler's output, the instruction at an address is found by binary
search rather than by a dense table, and every place that stepped by a literal
4 — the program counter, the end of the program, the instruction a return
address comes back from — uses the size stored with each instruction, which is
still 4. The execution fixtures are what says that swap changed no answer.

### What step 9's public API changed in these fixtures, and why

One key, in the 53 snapshots that hold a Location: **`end_column` is
`endColumn`**. Step 9 is the 2.0 WebAssembly API, and a Diagnostic crosses into
TypeScript as the object serde writes — so the shape the fixtures record is the
shape the editor receives, and it is camelCase there (the design record, "Public
API"). Renaming it in the fixtures instead of translating it in `ts-lib` keeps
one shape: a Location inside a Diagnostic, an assembled instruction, an undo
step or a call-stack frame is written the same way in all four.

Nothing else moved. `INSTA_UPDATE=always cargo test` rewrote the three
`-errors.snap` files and the 50 in `tests/diagnostics/snapshots`, and the
difference in every one of them is that key: the counts of Diagnostics per
fixture (16, 4 and 20) and every message, code, severity, line and column are
what step 8 left. The `.snap` files with no Location in them — every
`editor/` assembly and run fixture — are untouched.

### What the instruction table of the rewrite accepts and refuses differently

The Assembler's instruction table (`src/assembler/instructions/table.rs`, phase
1) replaces the two disagreeing lists of `src/semantic_checker.rs` and
`src/compiler.rs`. **No fixture in this directory moves because of it**: not one
line of the 30 `editor/` programs is judged differently, which
`the_analyzer_is_silent_on_every_editor_program` in `src/test/diagnostics.rs`
asserts program by program. What follows is the difference on code the corpus
does not contain, and every item is deliberate.

**Now accepted, where 1.4.2 refused:**

* **`extb.l d0`** — the checker did not know the Mnemonic, so the byte-to-long
  `ext` the compiler could build was unreachable (the bullet above).
* **A size on a branch**: `bra.b`, `bra.s`, `bcc.w`, `bsr.l`. The design record's
  "Instructions" asks for `.s`, `.w` and `.l`; `.b` is there because the help
  says so — "EASy68K will accept .B or .S to force 1-byte offsets and .W or .L
  to force 2-byte offsets" (`Reference/68ks9b.htm`, and the same sentence on
  `BRA`) — and it means exactly what `.s` means. All four are accepted and not
  range checked, because every instruction is still four bytes, and none of them
  reaches the encoded instruction: a branch carries no operand size.
* **A size on the memory form of a shift**: `asr.w (a0)`, the one size the
  68000 has there. `asr.l (a0)` is still refused, and now says `asr` takes `.w`
  there rather than "instruction is not sized".
* **The size the EASy68K help gives instructions 1.4.2 refused a size on
  altogether** (`Reference/68ks*.htm`, "DATA LENGTH"): `moveq.l`, `lea.l`,
  `pea.l`, `exg.l`, `swap.w`, `divs.w`/`divu.w`/`muls.w`/`mulu.w`, `dbra.w` and
  every `DBcc.w`, `Scc.b`, and `.b` or `.l` on `btst`, `bset`, `bclr` and
  `bchg`. The help's own `MOVEQ` example writes `MOVEQ.L #3,D0`, and ADR 0001
  says an EASy68K program assembles unchanged. No corpus program writes any of
  them, and the wrong size is still refused (`moveq.w`, `divu.l`).
* **`cmp (a0)+,(a1)+`** — the checker refused it while the compiler could build
  the `cmpm` the printer rules above already describe. It is the same
  checker/compiler disagreement as `extb`.
* **`moveq #128` to `#255`** — 1.4.2 refused anything above 127, although a
  `moveq` value is a byte pattern and `#$ff` is how a program writes -1. The
  range is now -128 to 255.
* **`btst #1,#$ff`** — `btst` only reads its destination, so it takes any data
  addressing mode, an immediate included; the checker held it to the same
  data-alterable set as `bset`, `bclr` and `bchg`, which do write theirs.
* **The PC-relative modes and the forced widths of an address** — `label(pc)`,
  `(d,pc)`, `label(pc,d1.w)`, `label.w` and `label.l`. The lexer of 1.4.2 had no
  reading for `pc` at all, so none of the first three could be written; the
  suffix on an address was read as an operand size and dropped. Phase 3's last
  part implements all five, and "Operands" above says how they print.

**Now refused, where 1.4.2 accepted:**

* **`tst a0`** — the 68000 has no address-register form of `tst`, and neither
  has EASy68K; the checker's rule only excluded an immediate.
* **`jmp (a0)+`, `jsr -(a0)`, `lea (a0)+,a1`, `pea -(a0)`** — these four take a
  *control* addressing mode, which is a place in memory that is not walked over:
  `(An)`, `d(An)`, `d(An,Xn)` and an absolute address. The checker allowed every
  memory mode.
* **`movem.l d0-d2,(a7)+` and `movem.l -(a7),d0-d2`** — registers go out through
  `-(An)` and come back through `(An)+`, and not the other way round. The
  checker allowed either mode on either side.
* **A bit number or a shift count read from memory**: `btst (a0),d0`,
  `asl (a0),d0`. A count is a data register or a literal.
* **A bit number above the destination's width**: `btst #40,d0` (a data register
  has 32 bits) and `btst #8,(a0)` (a byte in memory has 8). The checker's bound
  was 0 to 255 whatever the destination.

**Refused by both, and now said differently:** `trap #0` to `#14` ("s68k
simulates one trap, `#15`, which is its input and output" rather than "Only
implemented TRAP is 15 for IO"), `move usp,a0` ("`usp` is the user stack
pointer, which s68k does not assemble" rather than "Invalid absolute"), and the
three real 68000 instructions s68k does not implement — `rte`, `stop` and
`reset` — which are rows of the table carrying their reason rather than unknown
words. *(That list is corrected here rather than rewritten silently: it named
`addx`, `subx`, `negx`, `abcd`, `sbcd`, `nbcd`, `roxl` and `roxr` as well, and
phase 3's second half implemented all eight without updating the sentence.)*
`movep`, `tas`, `rtr`, `trapv`, `chk` and `illegal` were in it until phase 3's
first half, and so was every use of `sr` and `ccr`; the eight above went with
its second half, and the PC-relative modes with its last.

### What the review of phase 1 changed in these fixtures, and why

Two reviews read phase 1 and their findings were applied in one pass. **Two
`-errors.snap` fixtures moved, one entry each, and no other fixture did** —
`graphicSound-errors` and all 30 `editor/` assembly and run snapshots are
untouched, which `the_analyzer_is_silent_on_every_editor_program` and the
snapshots themselves hold:

| Fixture | What moved |
| --- | --- |
| `mouseWindowSize-errors` | the one `undefined_symbol` — `END START` against the Label `start` — is now the `entry_point_case_mismatch` **warning** of the section above, and the file assembles as far as its unimplemented features allow. Its 20 entries are otherwise what step 9 left |
| `clockDigital-errors` | the `unknown_mnemonic` on line 67, an invocation of the `DELAY` macro defined on line 20, is now `unimplemented_operation`: "`DELAY` is not implemented: it is a macro, and macros are not assembled yet", with a related Location on the `macro` line. The old message offered to make `DELAY` a label, which would have made the program worse. Its other 15 entries are unchanged |

Two messages were reworded and no fixture holds either: `address_register_byte_size`
now says an address register is never *used* one byte at a time rather than
never *written* one (the register is the operand read in `cmp.b a0,d1`, and
nothing is written to it), and the operand-count message of the data Directives
names the size and says "at least one value" for `dc`, which takes a list.

Four diagnostics are new and no corpus program raises one: the
`entry_point_case_mismatch` warning above, the `missing_comma_between_operands`
warning of `docs/grammar.md` 3.7, and the two warnings the design record always
listed and the evaluator did not raise — `character_literal_too_long` (EASy68K's
"ASCII constant exceeds 4 characters") and `constant_above_32_bits` (its
"Numeric constant exceeds 32 bits"). Each has a case in `tests/diagnostics/`.

In `tests/diagnostics/snapshots/` — the one case per Diagnostic, which is not a
1.4.2 fixture and moves whenever a message is improved — four snapshots are new
and four moved, each because its case program grew a line: the source form of
`address_register_byte_size` (`cmp.b a0,d1`, where nothing is written to the
register), the two dropped `equ` leniencies under `value_expected`, `d8` under
`undefined_symbol`, the data Directives under `wrong_operand_count`, and the
Macro invocation under `unimplemented_operation`.

### What phase 2's first half changed in these fixtures, and why

Phase 2 implements the Directives the design record's table calls "implement
with EASy68K meaning". Its first half is `reg`, `fail`, `simhalt` and the label
rules of §2.6 of `docs/grammar.md`. **Two `-errors.snap` fixtures moved, one
entry each, and no other fixture did**: all 30 `editor/` assembly and run
snapshots are untouched, because no program the editor ships writes any of the
three Directives.

| Fixture | What moved |
| --- | --- |
| `clockDigital-errors` | the `unimplemented_operation` on `SIMHALT` (line 124) is gone: `simhalt` assembles now. Its other 14 entries are unchanged, and the file is down from 16 to 15 |
| `graphicSound-errors` | the same, on line 206. Down from 4 to 3 |

Both files still hold errors — their macros, structured control and `rte` — so
neither builds a Program yet and neither has an assembly or a run fixture.
`the_easy68k_originals_raise_only_what_is_not_implemented` in
`src/test/diagnostics.rs` counts the same change by code and is what fails first
if one of them moves again.

**One EASy68K line forced a rule, and it is worth keeping.**
`SIMHALT                 Halt Simulator` is `Directives/simhalt.htm`'s own usage
line, `LABEL SIMHALT comment`; read as an ordinary Operation it would be the
Operand `Halt` and the Comment `Simulator`, because the Operand field ends at
the first whitespace that is not beside a comma (`docs/grammar.md` 1.5) and no
rule of the parser may consult a Directive's arity (ADR 0003). `simhalt`
therefore reads its whole Operand field as a Comment, which is what `page`,
`list` and `nolist` already do. Without it, implementing the Directive would
have *added* a `wrong_operand_count` to `graphicSound-errors`, which is the
opposite of what ADR 0001 promises.

In `tests/diagnostics/`, **six cases and six snapshots are new** and none moved:
`label_not_allowed`, `register_list_expected`, `not_a_register_list`,
`register_list_not_defined_yet` and `user_defined_error` are the new kinds, and
`register_list_in_expression` — listed in `src/test/diagnostics.rs` as having no
case since phase 1, because no File could define a Register list — has one now.
`unreadable_file` is the one code left without a case, and phase 4's `include`
owes it.

### What phase 2's second half changed in these fixtures, and why

Nothing. Phase 2's second half is `section` and `offset`, and **no fixture
moved**: no program of `editor/` or of `easy68k/` writes either Directive, so
the 30 assembly fixtures, the 30 run fixtures and the three `-errors.snap` are
exactly what the first half left them. `grep -ni 'offset\|section' tests/corpus`
finds the word only in this file.

Two rules of the format above were written for Directives no corpus program
uses, so that a fixture taken from a program that does use them is not a
surprise: a name defined inside an `offset` region is a **Constant** and so is
not in `labels` (the bullet in "Shape"), and a `section` never shows up in a
fixture at all — an address is an address, whichever counter it came from, so
the `memory` and `instructions` lists are sorted by address across all sixteen.
The two examples of the EASy68K help are held as fixture-style tests instead of
snapshots, in `src/test/corpus.rs`: `the_section_example_of_the_help` and
`the_offset_stack_frame_example_of_the_help`, each with every address worked out
by hand in the test rather than read back from the Assembler.

One sentence of this file was left out of date by the first half and is
corrected here rather than rewritten silently: the summary at the top said the
three `easy68k/` programs "now raise 16, 4 and 20" and listed `simhalt` among
the features they name. They raise **15, 3 and 20** since `simhalt` was
implemented, and the list no longer holds it. The count "16, 4 and 20" under
"What step 9's public API changed" is a statement about step 9 and is still
true.

In `tests/diagnostics/`, **one case and one snapshot are new** and none moved:
`no_bytes_in_an_offset_region`, the one kind this half adds, whose case holds
both the instruction and the `dc` that raise it. A section number outside 0–15
is the existing `value_out_of_range`, named by its subject, and needs no case of
its own.

### What phase 3's first half changed in these fixtures, and why

Phase 3's first half is the status register and the first group of missing
instructions: `move` and `andi`/`ori`/`eori` to and from `sr` and `ccr`,
`movep`, `tas`, `rtr`, `chk`, `trapv` and `illegal`. **One fixture moved, by one
entry**, and it is the shrinking of an `-errors.snap` that the design record
predicts as directives and instructions arrive:

| Fixture | What moved |
| --- | --- |
| `mouseWindowSize-errors` | the one `unimplemented_addressing_mode` is gone. Line 66 of the program is `andi.w  #$00,SR` — "put CPU in User mode" — which now assembles, and does exactly what the design record says it does: the system byte is stored, and it has no effect, because s68k runs every program as supervisor. The file raises **19** entries where it raised 20, all of them `unimplemented_operation` but the `entry_point_case_mismatch` warning |

The other two `-errors.snap` and all 30 `editor/` assembly and run fixtures are
byte for byte what phase 2 left: no `editor/` program writes any instruction of
this group — a grep over `editor/` for the six Mnemonics and for `,sr` and
`,ccr` matches one line, the Label `movepipes` in `flappy-bird.x68`, which is a
word and not a Mnemonic — and running them touches the new status register only
through the condition codes they already set, whose bits and whose `flags` line
are unchanged.

Three rules of the printer above are new, and are exercised by `printer_rules`
in `src/test/corpus.rs` rather than by any corpus program: `sr` and `ccr` as
Operands, the no-size rule for the ten instructions that name one, and `movep`
with its size and its direction. `usp` has no printed form at all, because
`move usp,an` is still not implemented.

In `tests/diagnostics/`, **no kind is new and two cases changed**:
`unimplemented_addressing_mode.asm` traded its `move.w sr,d0` for
`move.l usp,a0`, the one special register that is still not implemented, and
`unimplemented_operation.asm` traded its `movep.w d0,4(a0)` for `roxl.w #1,d0`.
Both files are cases for a code, and the lines in them had stopped raising it.
The first also reads differently: `unimplemented_addressing_mode` carries a
`planned` flag now, so `usp` is "which s68k does not assemble" and a PC-relative
operand is "which s68k does not assemble **yet**" — there is not going to be a
second stack pointer here, and the message no longer implies one.

### What phase 3's second half changed in these fixtures, and why

Phase 3's second half is the extend-flag and binary-coded-decimal group:
`addx`, `subx`, `negx`, `roxl`, `roxr`, `abcd`, `sbcd` and `nbcd`. **No fixture
moved at all.** No `editor/` program and none of the three `easy68k/` originals
writes any of the eight Mnemonics — a grep over both directories matches
nothing — so the 30 assembly and run fixtures and the three `-errors.snap` are
byte for byte what phase 3's first half left them, at **15, 3 and 19** entries.

Eleven printer lines are new and are exercised by `printer_rules` in
`src/test/corpus.rs` rather than by any corpus program: the four sized
instructions of the group, the three decimal ones that carry no size, and the
two rotates, whose memory form normalises to a count of one and a word exactly
as `asl (a0)` does.

In `tests/diagnostics/`, **one kind is new and one case changed**:
`invalid_operand_pair` is the message for the four instructions whose two Forms
are two whole shapes (`addx d0,-(a1)` is wrong in neither operand on its own),
and it has its own case; and `unimplemented_operation.asm` traded its
`roxl.w #1,d0`, which now assembles, for `reset`, which is refused for good and
whose reason carries no alternative — so the case covers a hint of `null` as
well. `unimplemented_addressing_mode.asm` is untouched this time: the
PC-relative modes and `usp` are what is left in it, and both are still refused.

### What phase 3's last part changed in these fixtures, and why

Phase 3's last part is the Addressing modes: PC-relative displacement and
index, and the widths `label.w` and `label.l` force on an absolute address.
**No `editor/` fixture moved** — no program the editor ships writes `(pc)` or a
forced width, which a grep over `editor/` confirms — and the three `-errors.snap`
moved by **two message lines**, which are the "yet" sweep below and not the
modes:

| Fixture | What moved |
| --- | --- |
| `clockDigital-errors` | two messages: "macros are not assembled **yet**" is "macros are not assembled", on the `macro` line and on the `DELAY` invocation. Its 15 entries, their codes, hints, lines and columns are otherwise what phase 3's first half left |
| `graphicSound-errors`, `mouseWindowSize-errors` | nothing at all: their diagnostics are structured control, which never said "yet" |

**Why the word went.** "Yet" says a later phase of this plan brings the feature
(the implementation notes, phase 3), and the design record has macros with
conditional assembly as "maybe a later milestone" rather than as a phase. Only
`include` and `incbin` still say it, and phase 4 is where they arrive. The
counts are unchanged: **15, 3 and 19**.

Seven printer lines are new and are exercised by `printer_rules` in
`src/test/corpus.rs` rather than by any corpus program: four PC-relative
Operands (a displacement forwards, one backwards, an index, and the
displacement-free `(pc,d1.w)`), and the two forced widths, which print as the
address and nothing else. The two rules above them in "Operands" say what they
mean.

In `tests/diagnostics/`, **one kind is new and four cases changed**:

* `invalid_address_width` is the new kind — `move.l table.b,d0` and
  `bra done.s` force the *address* to a width no address has — and its case
  holds both, with the three lines they were meant to be under them;
* `unimplemented_addressing_mode.asm` loses its `move.l greeting(pc),d0`, which
  now assembles, and is `usp` in both directions. The message also drops its
  "yet": with the PC-relative modes implemented there was nothing left for the
  `planned` flag to be true of, so the flag is gone and the sentence is
  "`usp` is the user stack pointer, which s68k does not assemble";
* `invalid_addressing_mode.asm` gains `move.l d0,greeting(pc)`, so that the new
  suggestion — "a PC-relative operand is read and never written" — has a
  snapshot to be read in, and its `divu` and `jmp` hints now offer `d(PC)` and
  `d(PC,Xn)` among what those positions take, which is the point of
  implementing them;
* `value_out_of_range.asm` gains the two new subjects of that code, the distance
  a `d(PC)` operand has to reach and an address forced to `.w`, because both are
  sentences a student has to be able to read.

`unimplemented_operation.snap`, `unterminated_macro_definition.snap` and
`label_not_allowed.snap` each lost a "yet" from a message about macros or
conditional assembly, and nothing else in them moved.

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
the whole corpus test, `bad-apple` included, takes about six seconds in an
unoptimised `cargo test` build — it took about half a minute on the pipeline the
fixtures were taken from.

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

### Two things about an execution fixture worth knowing before changing one

* **Memory starts as `$ff`, not as zeros.** Every byte no Directive and no
  instruction has written reads back as `$ff`, and the memory hash carries that
  fill. Since step 8 a `ds` block is such a byte throughout, because `ds`
  reserves and writes nothing (`ds_reserves_memory_without_writing_it` in
  `src/test/corpus.rs` is the program that shows it). Whether memory should
  start as zeros instead is a decision nobody has taken; taking it would move
  the `memory` hash of all 30 fixtures at once, which is what such a change
  should look like.
* **Seven programs never end.** They are event loops, and the policy answers
  "nothing was typed, nothing was clicked" for ever, so `limit` is the honest
  outcome rather than a fault. If a change makes one of them terminate, that is
  a change worth reading, not a snapshot to update blindly.
