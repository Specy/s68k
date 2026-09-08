# The source grammar

This is the specification the Assembler's tokenizer and parser follow, the
document [ADR 0002](adr/0002-hand-written-parser-with-a-grammar-document.md)
promised: *a change to what the parser accepts is a change to this document
first*. The dialect is EASy68K's ([ADR
0001](adr/0001-easy68k-is-the-reference-dialect.md)); where the
[design record](design/assembler-rewrite.md) is silent this document follows
the EASy68K help, and where that is silent too it says so and says which way
the choice went and why. Terms are the glossary's ([CONTEXT.md](../CONTEXT.md)).

Every production has a name. The parser tests are named after those names
(`source_line`, `pc_index`, `shift_expression`, …), so a rule and its tests can
be found from each other; section 5 is the index of every name in the document.

The parser accepts *shapes*, not meanings. Any well-formed Operand may stand at
any position of any Operation ([ADR
0003](adr/0003-operands-are-parsed-independently-of-the-instruction.md)); it is
the analyzer that decides whether `(a0)` may be a `divu` destination. A grammar
rule that rejects a well-formed Operand shape because no instruction wants it
is a bug in this document.

---

## 1. Lexical rules

### 1.1 `character_set` — what a source File is made of

A text File is a sequence of lines. A **character** is one Latin-1 byte ([ADR
0004](adr/0004-characters-are-latin-1-bytes.md)): the source arrives as UTF-8
text, and every character whose code point is 255 or less is that Latin-1 byte.

* A character above 255 is an **error** (`character_above_latin1`), named in the
  message. When it is one of the typographic look-alikes — `‘ ’ “ ” – —` — the
  hint names the plain character to write instead (`'`, `"`, `-`), because the
  cause is nearly always a paste from a web page or a word processor.
* `line_end` is LF, CRLF, or the end of the File. A File that does not end in a
  line terminator ends its last line all the same:
  `tests/corpus/editor/bad-apple.x68` is such a File and must keep assembling.
* `whitespace` is one or more of space (`$20`), tab (`$09`), form feed (`$0C`)
  and vertical tab (`$0B`). Tab stops play no part in the grammar: a tab is one
  character wide, and a Location's column counts characters.
* A no-break space (`$A0`) is **not** whitespace. It gets its own error
  (`non_breaking_space`) naming it, because it is invisible, it is what a paste
  from a web page leaves behind, and every other diagnosis of it would be a lie.
* Any other character that starts no token is `unexpected_character`: a control
  character below `$20`, a lone `<` or `>` (they come only in pairs, 1.13), and
  the characters no rule of this document uses at all — `=`, `?`, `` ` ``, `[`,
  `{` and their like. A lone `.` where a name may begin, with no name character
  after it, is the same error: there is no name to read and no size to report
  (1.7).

Those three errors are raised only where the character could reach the Program:
nowhere inside a `comment_line` or a `comment_field`, which are not tokenized at
all (1.6), and everywhere else. A pasted em-dash or accented letter in a Comment
produces no byte, and refusing a File for it would be the kind of message ADR
0003 exists to avoid; the same character in a `dc.b` string is refused, because a
byte has to be written for it and there is none.
The help has no equivalent — EASy68K reads a byte at a time and has no notion of
a character it cannot store — so the choice is s68k's and is recorded here.

Inside a quoted literal the same test says which of the three applies, and it
lets two of them through: a no-break space and a control character **are** bytes
(`$A0`, `$09`), so inside quotes they are ordinary characters and only
`character_above_latin1`, the one character with no byte at all, is raised there.
Outside quotes nothing is a byte yet and all three hold. An earlier draft of this
section said "everywhere else, a quoted literal included", which contradicted the
rule it had just given: a `dc.b` string holding a no-break space writes one
byte and is not an error.

### 1.2 `case_rule` — what is case sensitive

Case **insensitive**: Mnemonics, Directive names, size suffixes, register names
(`d0`, `A7`, `sp`, `PC`, `sr`, `Ccr`, `usp`), the `opt` option names, and the
hexadecimal digits `a`–`f`. `MOVE.B` and `move.b` are one Operation.

Case **sensitive**: every Symbol name. `Count` and `count` are two Symbols. This
is EASy68K's rule and the corpus depends on it in both directions:
`tests/corpus/editor/bad-apple.x68` writes `START:` while five other `.x68`
programs write `start:`, and only the first is the Entry point fallback.

### 1.3 `source_line_fields` — the four fields

A Source line is read as up to four fields in this order:

| Field | Holds | Introduced by |
| --- | --- | --- |
| `label_field` | a Label | see `label_rule` (1.4) |
| `operation_field` | a Mnemonic or Directive name with an optional `size_suffix` | the first word that is not a Label |
| `operand_field` | the comma separated Operands | whitespace after the operation |
| `comment_field` | free text, ignored | see `comment_rule` (1.6) |

Every field is optional. A line with none of them is a `blank_line`; a line
whose first non-blank character is `*` or `;` is a `comment_line`; a line with a
`label_field` and nothing else is legal and marks the address of the next line
that produces bytes.

One construct spans lines, and it is the only one: the body of a `macro` …
`endm` definition, which the parser skips whole and never tokenizes
(`macro_definition`, 2.6). Every other construct is read within its line.
Recovery is therefore "report, skip to `line_end`, carry on with the next line",
and a diagnostic never swallows the rest of a File.

### 1.4 `label_rule` — when a word is a Label

An identifier is a Label if it is followed by `:`, wherever it sits, or if it
starts in column 1 and is not the name of a Mnemonic, a Directive or (later) a
Macro. **A word carrying a `size_suffix` is never a Label.** The colon is not
part of the name.

| Word | Column | Followed by `:` | Is a Mnemonic / Directive name | Carries a `size_suffix` | Read as |
| --- | --- | --- | --- | --- | --- |
| `loop:` | any | yes | no | – | Label `loop` |
| `end:` | any | yes | **yes** | – | Label `end` |
| `.retry:` | any | yes | no | – | Local label `.retry` |
| `loop` | 1 | no | no | no | Label `loop` |
| `clr` | 1 | no | yes | no | Operation `clr` |
| `dc.b` | 1 | no | yes | yes | Operation `dc.b` |
| `foo.b` | 1 | no | no | yes | Operation `foo.b` (unknown Mnemonic) |
| `loop` | > 1 | no | no | no | Operation `loop` (unknown Mnemonic) |
| `d0:` | any | yes | no | – | **error** `reserved_name_as_symbol` |

Three of those rows are the ones that surprise, and each carries its own hint.
An indented unknown word (row 8) gets "start it in column 1, or end it with a
colon, if `loop` is a Label"; a word in column 1 that names an instruction (row
5) gets "write `clr:` if `clr` is meant as a Label"; a register name (row 9) may
never be a Symbol at all, because `move.l d0,d1` would stop meaning what it
says.

Only one Label to a line: a second colon-terminated identifier in the operation
position is `two_labels_on_one_line`.

The corpus proves every interesting row. `tests/corpus/editor/*.asm` open with
`count equ 10` and `COLS equ 32` in column 1 (row 4);
`tests/corpus/editor/bad-apple.x68` writes 6526 `dc.b` lines in column 1 (row 6)
and closes with `END:` (row 2), and three `.asm` programs write `end:` as a
Label and `bra end` as its use; `tests/corpus/easy68k/graphicSound.X68` writes
`START` alone on a line and `play0 move #0,d1` with the Label in column 1.

### 1.5 `operand_field_extent` — where the Operand field ends

Scanning from the first non-blank character after the operation field:

1. Inside a quoted literal nothing ends the field but `line_end`, which reports
   `unterminated_string`.
2. A `;` outside a quoted literal ends the field, **whatever the parenthesis
   depth** — a comment marker outweighs an unclosed `(`, and reporting the
   unclosed parenthesis beats swallowing the comment.
3. `line_end` ends the field.
4. A run of whitespace ends the field **unless** the parenthesis depth is
   greater than zero, or the character immediately before the run is `,`, or the
   first non-blank character after the run is `,`.

Every Operation's Operand field is read this way except those named in
`text_operation` (2.6) — `fail`, `include`, `incbin` and the refused keywords —
whose Operand field is text and is not tokenized at all.

`include` and `incbin` end their field exactly where this section ends it, rules
1 to 4 and all — rule 1 is how `include 'input output macros.x68'` keeps its
spaces — and only what is inside is read differently, as one raw filename rather
than as a list of Operands. `fail` and the refused keywords end theirs at a `;`
(rule 2) or at the `line_end` (rule 3) and at nothing else: whitespace never
ends such a field, and a quote inside it is an ordinary character, so
`FAIL don't call foo without an argument` is one message and
`if <cs> then.s   ; if set` still keeps its Comment. A `;` therefore starts a
Comment on every line of this language, with no exception at all.

Rule 4 is the whole of EASy68K's bare Comment field: `move.b #23,d0   trap task
23` stops at `#23,d0`. It is also why `lea greeting, a1` (every `.asm` in the
corpus), `move.l d0 ,d1` and `move.w (a0, d5), d6` (line 16 of
`tests/corpus/editor/binary-search-1.asm`) all keep working. Whitespace inside
quotes is rule 1: `dc.b '    X     Y   Left  Rght  …',CR,LF` is one Operand
(`tests/corpus/easy68k/mouseWindowSize.X68` line 247) and a bare
Comment can hold an apostrophe (`noon in 100's of a second`,
`tests/corpus/easy68k/clockDigital.X68`) because the field ended before it.

An unclosed `(` therefore runs the field to the `;` or the `line_end`, and the
parser reports against the `(` that opened it: `malformed_operand` when the
tokens after the `(` had already committed the Operand to an Addressing mode,
`unclosed_parenthesis` when they had not. Section 3.11 works both and section 4
states the precedence.

### 1.6 `comment_rule` — the three kinds of Comment

* `comment_line` — the first non-blank character of the line is `*` or `;`. The
  rest of the line is not tokenized at all. Both indented and column 1 forms
  occur throughout the corpus.
* `explicit_comment` — a `;` anywhere outside a quoted literal, or a `*` that is
  the first non-blank character of the comment field.
* `bare_comment` — anything else left on the line after `operand_field_extent`
  has ended the Operand field. It is EASy68K's own Comment field and it is
  accepted; the **first** one in a File raises the `bare_comment` *suggestion*
  ("EASy68K's comment field; `;` says so"), once per File and never again, so a
  program written in EASy68K's style gets one line of advice rather than two
  hundred.

`*` is a Comment marker only in the two positions above. Everywhere else it is
an Expression term or the multiplication operator:

| Where the `*` sits | Read as |
| --- | --- |
| first non-blank character of a line | `comment_line` marker |
| first non-blank character of the comment field | `explicit_comment` marker |
| where an Expression term may begin — the start of an Operand, after `(`, `#`, `,` or a binary operator | `current_address` |
| after a complete Expression term inside the Operand field | multiplication |

So `lea *,a0`, `org *`, `org (*+1)&-2` and `NOON equ 12*60*60*100` all mean what
EASy68K means by them. Section 3.4 has the one case this costs.

The Operation field is optional (`code_line`, 2.2), so a line that holds a Label
and then a Comment reaches the second row of that table: in `start: * entry
point` and `start * entry point` the `*` is the first non-blank character of the
Comment field, because there is no Operand field for it to open, and it marks an
`explicit_comment` exactly as `;` would.

### 1.7 `identifier`, `global_identifier`, `local_identifier`

```
identifier   = global_identifier | local_identifier ;
global_identifier = letter { letter | digit | "_" } ;
local_identifier  = "." ( letter | digit | "_" ) { letter | digit | "_" } ;
letter       = "A" … "Z" | "a" … "z" | "_" ;
digit        = "0" … "9" ;
```

An identifier holds no `.` after its first character. That is EASy68K's rule
("global labels should start with a letter and be followed by letters, numbers
or underscores; local labels must start with a dot") and it is what makes
`size_suffix` unambiguous: in `dc.b` the `.` can only be a size marker, and in
`.loop` it can only be a local label marker. The disposition of a `.` is
positional and is decided by the tokenizer:

* a `.` **immediately after** an identifier, a number, a `)` or a `'`-literal,
  with no whitespace between, opens a `size_suffix`;
* a `.` anywhere else — at the start of the Operand field, after whitespace,
  `,`, `(`, `#` or a binary operator, or in column 1 — opens a
  `local_identifier`. With no name character after it, there is no
  `local_identifier` to read and no size to report, and the `.` is
  `unexpected_character` (1.1).

`.l` in column 1 is therefore the Local label `.l`, not a bare size suffix, and
`bra .loop` is a branch to a Local label. A name may open with `_` as well as
with a letter, which EASy68K's prose does not promise and its assembler accepts;
it costs nothing and `_start` is a name students write. Only the first 32
characters of a name are significant in EASy68K; s68k keeps the whole name and
does not truncate, which can only make two names that EASy68K confuses stay
apart. No corpus program has a name of more than 32 characters or a Local label
at all.

Where a Local label's *scope* begins and ends — between the Global label above
it and the next one, as EASy68K, so that the same local name may be reused under
different Global labels — is not the parser's rule. This section gives the
syntax; the scoping is the symbol table's, in phase 2 (the design record,
"Symbols and expressions"; CONTEXT.md, "Local label"). The parser records
`.retry` as a Local label and nothing more.

### 1.8 `number`

```
number             = hexadecimal_number | binary_number | octal_number | decimal_number ;
hexadecimal_number = "$" hex_digit { hex_digit } ;
binary_number      = "%" ( "0" | "1" ) { "0" | "1" } ;
octal_number       = "@" octal_digit { octal_digit } ;
decimal_number     = digit { digit } ;
hex_digit          = digit | "A" … "F" | "a" … "f" ;
octal_digit        = "0" … "7" ;
```

The four prefixes are EASy68K's (`Directives/operators.htm` lists `$` hex, `%`
binary, `@` octal). There is no trailing-letter form (`0FFh`) and no `0x`; a
leading zero means nothing, `010` is ten.

A number token takes the longest run of letters, digits and underscores after
its prefix and the parser then reports which characters are not digits of that
base, so `$1G` is "`G` is not a hexadecimal digit" rather than a hexadecimal `$1`
followed by a stray symbol `G`. A prefix with no digit at all
(`$`, `%`, `@`) is the same diagnostic, `invalid_number`. A `decimal_number`
takes the same run although it has no prefix, so `12ab` is one bad number and
not a number followed by a name: the run is where the mistake is, and splitting
it would report a missing comma instead.

Values are computed in 64 bits (`i64`) and range-checked later against the
Operand's size; a literal above 32 bits is EASy68K's "numeric constant exceeds
32 bits" *warning*, `constant_above_32_bits`, raised by the evaluator
(`src/assembler/expr.rs`) and not by the parser. A literal that
does not fit in the 64 bits themselves has no value to carry on with and is the
error `number_too_large`, raised by the parser as it reads the digits. The help
has none: EASy68K works in 32 bits and its own warning covers everything above
them.

### 1.9 `character_literal` and `string_literal`

```
string_literal      = single_quoted | double_quoted ;
single_quoted       = "'" { character_except_quote | "''" } "'" ;
double_quoted       = '"' { character_except_double_quote | '""' } '"' ;
character_except_quote        = ? any character but "'" and line_end ? ;
character_except_double_quote = ? any character but '"' and line_end ? ;
character_literal   = string_literal ;
```

One token, two readings decided by where it stands:

* as an Expression term it is a **character literal**: `'A'` is 65, `'AB'` is
  `$4142`, up to four characters. Beyond four the evaluator warns
  (`character_literal_too_long`), as EASy68K does ("ASCII constant exceeds 4
  characters"), and the value is the literal's last four bytes. The message
  names the literal as it was written, because the number it packs to is one
  nobody typed; for the same reason the analyzer does not then range-check an
  immediate that holds one.
* as a `dc_item` it is a **string**: `dc.b 'Hello, world!',0` stores one byte per
  character with no length limit. What `dc.w` and `dc.l` do with a string is the
  Directive's business, not the grammar's.

`''` inside a single-quoted literal is one `'`. Nothing else is an escape: a
`\` is a backslash, which is what makes `include "..\lib\io.x68"` readable.

Double quotes are the one place this document is more lenient than the help,
which documents `'` for strings and both quotes for `include`/`incbin`
filenames. Accepting `"Hello"` costs nothing, is what a student who has written
C will try, and refusing it would teach nothing; the first double-quoted literal
in a File raises the `double_quoted_string` *suggestion* naming EASy68K's `'`,
once per File, on the same footing as `bare_comment`.

The suggestion is about strings and character literals only. A
`file_specification` (2.6) never raises it: `include` and `incbin` are the one
place the help documents both quotes ("must be enclosed in single (') or double
(") quotes if any part of the file path or name includes spaces",
`Directives/include.htm`), and both its examples are double quoted, so
`INCLUDE "C:\EASy68K\macros\input output macros.x68"` copied from the help would
otherwise be told that its quoting is un-EASy68K. A filename is not tokenized as
a string in any case (2.6).

### 1.10 `register`

```
register          = data_register | address_register ;
data_register     = "d0" | "d1" | "d2" | "d3" | "d4" | "d5" | "d6" | "d7" ;
address_register  = "a0" | "a1" | "a2" | "a3" | "a4" | "a5" | "a6" | "a7" | "sp" ;
program_counter   = "pc" ;
special_register  = "sr" | "ccr" | "usp" ;
```

Case insensitive. `sp` is `a7` and nothing else — the printer never writes `sp`
back out (`tests/corpus/README.md`, "Registers"). `pc` is a register only in the
PC-relative modes. All twenty-one names above are reserved: none of them may be
a Symbol.

*Which* diagnostic a reserved name gets depends on what is being done with it,
and the two are not interchangeable: `reserved_name_as_symbol` where a name is
**defined** (a Label, an `equ`, a `set`, a `reg`), whose hint offers another
name; `register_in_expression` (2.7) where one is **used** where an Expression
term must stand, whose hint says that an Expression is computed before any
register has a value. `move.l pc,d0` and `move.l #a0+4,d0` are the second. An
earlier draft of this section said that a `pc` outside a PC-relative mode was
`reserved_name_as_symbol`, which contradicted 2.7 and would have answered a
misused register with an offer to rename it.

`sr`, `ccr` and `usp` are Operands in their own right, not Addressing modes, and
the parser accepts them at any position of any Operation.
`tests/corpus/easy68k/mouseWindowSize.X68` writes `andi.w #$00,SR`.

### 1.11 `size_suffix`

```
size_suffix = "." ( "b" | "w" | "l" | "s" ) ;
```

Case insensitive, and it may follow an operation, an absolute Operand or an
index register. `.b` byte, `.w` word, `.l` long, `.s` short — `.s` is a branch
displacement size and the parser accepts it anywhere, leaving "`.s` is not a
size for `move`" to the analyzer, which can name the sizes that *are* allowed.

The token the tokenizer makes of a `.` in suffix position (1.7) takes the whole
run of letters, digits and underscores after the dot — the same "longest run,
then diagnose" convention as `number` (1.8) — so a bad suffix is one token and
never a cascade of an identifier and its neighbours. The parser then reads the
run by the field it stands in:

* in the operation field, a run that is not `b`, `w`, `l` or `s` is
  `unknown_size_suffix`: `move.q d0,d1` is "`.q` is not a size" with the four
  letters in the hint, and `move.ll d0,d1` is "`.ll` is not a size". An
  operation name holds no dot of its own (1.7), so the run can be nothing else.
* in the Operand field, a run of one character is the size suffix (`label.w`,
  `d1.l`) or, when it is not one of the four letters, `unknown_size_suffix`. A
  run of two or more characters is a dot inside a name and is `dot_in_name`:
  `move.l array.length,d0` is "a name holds no dot after its first character",
  pointing at the dot, rather than a size `.l` with `ength` left over.
  `.length` cannot be a Local label here either, because a `.` directly after an
  identifier is in suffix position (1.7).

In either field an empty run — a `.` with nothing at all after it, `move.` or
`move.l label.,d0` — is `unknown_size_suffix`, and the message reads "`.` is not
a size".

The help is silent on all of them — EASy68K answers a dotted name with "Invalid
size code" or "Undefined symbol" — and this is the reading that names what is
wrong.

### 1.12 `register_list`

```
register_list      = register_list_item { "/" register_list_item } ;
register_list_item = register_range | register ;
register_range     = register "-" register ;
```

`d0-d3/a0-a2` is the shape EASy68K documents for `movem` and `reg`
(`Reference/68ks4f.htm`: "a series of registers separated by a slash … one of
many intervals (shown with a `-`)"). Both ends of a range are registers of
`register` — never `pc`, `sr`, `ccr` or `usp`, which that production does not
hold — and the first may not come after the second in `movem`'s own mask order,
`d0`…`d7`, `a0`…`a7`: `d5-d2` and `a2-d5` are `register_range_out_of_order`.

A range **may** cross from `d7` into `a0`. `d0-a6` is bits 0 to 14 of a `movem`
mask, which is one contiguous sixteen-bit field over exactly that order; the
help neither documents the crossing nor forbids it, and `tests/corpus/README.md`
already specifies that the canonical *printed* form splits such a range
("a range never crossing from `d7` into `a0`"), which is only meaningful if one
can be written. Accepting it is the lenient direction of ADR 0001 and needs no
deviation recorded there. An earlier draft of this document made it the error
`register_range_crosses_register_kinds`; that kind is withdrawn.

A single register is a one-register list, so `movem.l d1,-(a7)` (line 178 of
`tests/corpus/easy68k/clockDigital.X68`) parses as `data_register_direct` and is
read as a list of one by the analyzer.

A `reg` Symbol standing in for a list — EASy68K's `AllRegs REG D0-D7/A0-A6`,
then `movem.l AllRegs,-(sp)` — is *not* a grammar form. A bare identifier is
always parsed as `absolute`; it becomes a Register list when the Symbol is
resolved, which is where the "symbol is not a register list" and "register list
symbol not previously defined" diagnostics belong. This is ADR 0003 exactly: the
parser cannot know, and does not guess.

### 1.13 `operator` and `punctuation`

```
unary_operator  = "-" | "~" ;
binary_operator = "+" | "-" | "*" | "/" | "\" | "&" | "!" | "|" | "^" | "<<" | ">>" ;
punctuation     = "#" | "," | "(" | ")" | "/" | ":" | "." | "-" | "+" ;
```

The set is EASy68K's `Directives/operators.htm`, verbatim. There is **no** `**`:
assemblers that have one spell exponentiation with it and EASy68K does not, its
own reading of `2**3` being `2` times the current address times `3`, because the
second `*` lands where an Expression term may begin (1.6).

s68k does not reproduce that reading, and does not have to: under the layered
grammar of 2.7 the multiplicative loop consumes `2 * *`, the second `*` being the
`current_address`, and then stops at the `3`, which is not an operator;
`primary_expression` has no juxtaposition alternative, so the `3` is left over
and the Operand is reported as `unexpected_token_in_operand`. That is the better
answer of the two: a student who writes `2**3` means a power, and being told
that the `3` was not expected — and that `*` is the current address — says so,
where quietly assembling `2 * here * 3` says nothing.

There is no unary `+`. `#+5` is `plus_is_not_a_unary_operator`, whose hint is
"write `5`"; EASy68K does not list one either.

`/` is a register list separator inside a `register_list` and division
everywhere else; the two never meet, because a register may not appear in an
Expression.

### 1.14 The token kinds

The tokenizer of one line produces, for `src/assembler/token.rs`:

`Identifier`, `LocalIdentifier`, `Number`, `StringLiteral`, `SizeSuffix`,
`Hash`, `Comma`, `LeftParen`, `RightParen`, `Colon`, `Plus`, `Minus`, `Star`,
`Slash`, `Backslash`, `Ampersand`, `Bang`, `Pipe`, `Caret`, `Tilde`,
`ShiftLeft`, `ShiftRight`, `Whitespace`, `Comment`, `EndOfLine`, `Error`.

`LocalIdentifier` is its own kind because the tokenizer has already decided the
disposition of the `.` that opens it (1.7) and throwing that away would make the
parser decide it twice. A `Number` carries the base its prefix asked for and a
`StringLiteral` the quote it was written with, which is all a later phase needs
to read the run of characters the span covers. Registers are **not** a kind:
`d0` is an `Identifier`, and 1.10 is what recognises one.

Every token carries the `Span` it covers, so the column of a diagnostic is never
recomputed from the text. The tokenizer knows the column of the first token,
which is what makes `label_rule` decidable, and it never crosses a `line_end`.

---

## 2. The grammar

### 2.1 Notation

`=` defines, `;` ends a rule, `|` alternates, `[ x ]` is optional, `{ x }` is
zero or more, `( )` groups, `"x"` is a literal (case insensitive when it names a
Mnemonic, a Directive, a register or a size), and `? prose ?` is a production
this document defines in words. Whitespace between fields is written
explicitly; whitespace *within* the Operand field obeys 1.5 and is not written.

### 2.2 `source_file` and `source_line`

```
source_file  = { source_line } ;
source_line  = blank_line | comment_line | code_line ;
blank_line   = [ whitespace ] line_end ;
comment_line = [ whitespace ] ( "*" | ";" ) { character } line_end ;
code_line    = [ label_field ] [ whitespace ]
               [ operation_field ]
               [ comment_field ]
               line_end ;

comment_field    = explicit_comment | bare_comment ;
explicit_comment = ( ";" | "*" ) { character } ;
bare_comment     = { character } ;
character        = ? one Latin-1 character other than a line terminator ? ;
```

`code_line` admits `label_field` only under `label_rule` (1.4), which is
positional and therefore stated as a table rather than as syntax.
`src/assembler/ast.rs` mirrors this as
`Line { label, operation, comment, bare_comment }`.

### 2.3 `label_field`

```
label_field      = colon_label | column_one_label ;
colon_label      = identifier ":" ;
column_one_label = ? an identifier beginning in column 1, carrying no
                     size_suffix, not followed by ":", and not the name of a
                     Mnemonic, Directive or Macro — label_rule, 1.4 ? ;
```

The `:` follows its name directly. Whitespace *between* fields is written
explicitly in this document and a field holds none (2.1), so `loop :` is not a
`colon_label`: it is a `column_one_label` and then a `:` with no name before it,
`empty_label`. Indented, `  loop : nop` is the Operation `loop` and the `:` is
`unexpected_token_in_operand`.

### 2.4 `operation_field`

```
operation_field = operation [ whitespace operand_field ] ;
operation       = operation_name [ size_suffix ] ;
operation_name  = identifier ;
```

The `whitespace` before the Operand field is written because EASy68K requires
it. s68k accepts a line without it — `move.l#5,d0`, `dc.b'x'`, `loop:move.l
d0,d1` — which is the lenient direction of ADR 0001 and costs nothing: the
tokenizer has already separated the two fields, and a missing space between two
things that are *both* names merges them into one token rather than making a
second reading possible (`move.ld0` is the operation `move` with the unknown
size `.ld0`), so no shape becomes ambiguous. The alternative would be an error
message about a space rather than about the program.

The parser does not classify `operation_name`. It hands the name, the size and
the parsed Operands to the analyzer, which looks the name up in the one
instruction table and answers with a did-you-mean, the Label hint, or the reason
a real feature is not implemented.

The parser puts exactly two questions about a name, and these are the only two.
`label_rule` (1.4) asks the instruction table "is this word a Mnemonic or a
Directive name?", and nothing else (ADR 0003). The Operand field asks a second,
*closed* list — `text_operation` (2.6), written out in this document and not in
the instruction table — "does this Operation take its Operand field as text
instead of as an `operand_list`?", because the answer decides whether that text
is tokenized at all and no later phase can undo the tokenizing. Neither question
reaches the operand rules, the sizes or the value ranges of the instruction
table: those stay the analyzer's.

### 2.5 `operand_field` and the Addressing modes

```
operand_field      = operand_list | text_operand_field ;
operand_list       = operand { "," operand } ;
text_operand_field = file_specification | message_text | raw_operand_field ;
```

Which of the two alternatives applies is decided by the Operation's name and by
nothing else: an Operation named in `text_operation` (2.6) takes the text form
that production gives it — `file_specification` for `include` and `incbin`,
`message_text` for `fail`, `raw_operand_field` for the refused keywords — and
every other Operation, known or unknown, takes an `operand_list`. This is the
parser's second table question (2.4), and it has to be the parser's because a
text Operand field is never tokenized: read as an `operand_list`,
`include io.x68` would take `.x` for a `size_suffix` (1.11) and leave `68` over,
and `fail ERROR, Argument missing` would split at its comma.

```
operand = immediate
        | register_list
        | data_register_direct
        | address_register_direct
        | special_register
        | predecrement
        | indirect
        | postincrement
        | displacement
        | index
        | pc_displacement
        | pc_index
        | absolute ;

immediate               = "#" expression ;
data_register_direct    = data_register ;
address_register_direct = address_register ;
indirect                = "(" address_register ")" ;
postincrement           = "(" address_register ")" "+" ;
predecrement            = "-" "(" address_register ")" ;

displacement    = expression "(" address_register ")"
                | "(" expression "," address_register ")" ;
index           = expression "(" address_register "," index_register ")"
                | "(" expression "," address_register "," index_register ")"
                | "(" address_register "," index_register ")" ;
pc_displacement = expression "(" program_counter ")"
                | "(" expression "," program_counter ")" ;
pc_index        = expression "(" program_counter "," index_register ")"
                | "(" expression "," program_counter "," index_register ")"
                | "(" program_counter "," index_register ")" ;

absolute        = expression [ size_suffix ] ;
index_register  = register [ size_suffix ] ;
```

A lone register is `data_register_direct` or `address_register_direct`;
`register_list` is taken only when a `-` or a `/` follows the first register
(1.12). `special_register` is `sr`, `ccr` or `usp`.

Both spellings of every displaced mode are EASy68K's own: `x(An)` and `(x,An)`,
`x(An,Xn.s)` and `(x,An,Xn.s)`, `x(PC)` and `(x,PC)`, `x(PC,Xn.s)` and
`(x,PC,Xn.s)` (`Reference/68ks1e.htm`, `Directives/offset.htm`). The
displacement-free `(An,Xn.s)` is accepted as a displacement of zero. An
`index_register` with no `size_suffix` is `.w`, which is the 68000's default and
what the fixture printer writes out.

`absolute` covers the bare Expression (`move.l $1000,d0`, `bra done`), the
parenthesised Expression EASy68K also accepts (`move.l ($1000),d0`), and the
forced widths `label.w` and `label.l` (`Reference/68ks1e.htm`, "Forcing Absolute
Short Addressing").

`parenthesised_operand` is not a mode but the decision procedure for an Operand
that starts with `(`, and it is where a recursive-descent parser has to be told
what to do. Having consumed the `(`:

1. if the next token is a `register` or `pc` **and** the one after it is
   `)`, `,`, or the end of the Operand field, the Operand is `indirect`,
   `postincrement`, `index`, or the `pc_index` form. A *data* register matches
   this rule although no mode takes one as its base: what follows is
   `malformed_operand` naming the shape (below), because `(d0)` is a mistake
   with a name and reading it as a `grouped_expression` would answer
   `register_in_expression`, which describes a program nobody wrote;
2. otherwise parse an `expression`, and then
   * `,` follows — the Operand is `(d,An)`, `(d,An,Xn)`, `(d,PC)` or
     `(d,PC,Xn)`; an `address_register` or `pc` is required after the comma;
   * `)` follows and the token after it is a `binary_operator` — the `( … )` was
     a `grouped_expression`, so the Pratt loop continues with it as its left
     term and the Operand is an `absolute`;
   * `)` follows and the token after it is `(` — the `( … )` was the
     displacement of a `displacement` or `index` Operand;
   * `)` follows and nothing else does — the Operand is an `absolute`, with an
     optional `size_suffix`.

Which of the two failures a missing `)` becomes is decided here as well: once
rule 1 has matched, or rule 2's first case has seen the `,` after the
Expression, an Addressing mode has been recognised and the diagnostic is
`malformed_operand`, which can name the shape it tried to be; when neither has,
the `(` opened a `grouped_expression` and the diagnostic is
`unclosed_parenthesis` (3.11, and the precedence note under section 4).

Rule 2's second case is not decoration: `ORIGINX equ (640-COLS*SCALE)/2` is line
30 of `tests/corpus/editor/bad-apple.x68`, and a parser that commits to
"indirect" at the `(` loses it.

Two of rule 1's outcomes are not modes at all, and both are `malformed_operand`
because the mode *has* been recognised — which is the whole point of the rule
naming the shape rather than the token:

* **the register is the last token of the Operand field** — `move.l (a0` —
  "this looks like an indirect operand, `(a0)`, but the `)` is missing". Rule
  1's third case exists for this: falling through to rule 2 would parse `a0` as
  an Expression term and report that `a0` is a register, which describes a
  program nobody wrote. The `;` case of 3.11 is the same shape with a Comment
  after it. `move.l (a0,` ends the same way but names the other shape: the `,`
  is evidence that an index was meant, so the report is "this looks like an
  indexed operand, `4(a0,d1.w)`, but the index register is missing".
* **`(pc)`** — a PC-relative Operand carries a displacement (`pc_displacement`,
  above), so this is "this looks like a PC-relative operand, `label(pc)`, but
  the displacement is missing".
* **the register is a data register** — `move.l (d0),d1`, `move.l (d0,d1),d2` —
  "this looks like an indirect operand, `(a0)`, but `d0` is not an address
  register". It is the same sentence `4(d0)` already produced through
  `base_and_index`, and it is the one that says what to write.

A leading `-` is `predecrement` only when a `(` and a `register` follow it;
otherwise it is a unary minus opening an `expression`, so `-4(a6)`, `-(4)` and
`-(a1)` all land where they should
(`tests/corpus/editor/subroutine-with-stack-arguments-1.asm`). A data register
there is rule 1's case again: `-(d1)` is "this looks like a predecrement
operand, `-(a0)`, but `d1` is not an address register". A `-` `(` and an
`address_register` with no `)` after it — `move.l d0,-(a7` — is
`malformed_operand` naming the predecrement shape, for the same reason as rule
1's third case: no other Operand begins that way, and an unclosed
`grouped_expression` holding a register is not a reading anyone meant.

### 2.6 The Directives

Bucketed as the design record has them. `[label]` marks a Directive that accepts
a Label; `label` marks one that requires it.

These productions are the **Directives phase's**, not the parser's. The parser
reads every line by 2.2 to 2.5 into one `Line` and never by the rules below;
each `[ label_field ]` restated here is a requirement phase 2 checks against
that Line's label field, not a second parse of the same text, and `dc_item` is
how phase 2 reads an Operand the parser has already parsed (a string reaches it
as an `absolute` whose Expression is one `string_literal`). The one part of this
section the parser does enforce is `text_operation`, because it decides how the
Operand field is tokenized (2.4, 2.5).

**The label field is a rule of three values**, and the productions below carry
it: `label` requires one, `[label]` accepts one, and a production written with
no label field at all forbids one. The two that are not the default are
EASy68K's own errors and its own lists:

| Rule | Directives | EASy68K |
| --- | --- | --- |
| Required | `equ`, `set`, `reg`, and `section` with no number | "Label required with this directive" |
| Forbidden | `page`, and `ifeq`, `ifne`, `iflt`, `ifle`, `ifgt`, `ifge`, `ifc`, `ifnc`, `ifarg`, `endc` | "Label is not allowed" |
| Accepted | every other Directive; the Label names the address the line sits at | — |

`page` is "No label is permitted" (`Directives/page.htm`) and the conditional
ones are "IFxx and ENDC directives may not be labeled"
(`Directives/conditional.htm`); the help says nothing about `macro` — whose
label field holds the Macro's name and which is therefore not in the list — nor
about the structured-control keywords, and silence is answered the lenient way
(ADR 0001). A Directive s68k refuses whole is checked all the same, so
`skip ifeq debug` is answered twice: the label rule is about the shape of the
line and holds whether or not the feature is implemented, and the two
Diagnostics point at two different fields. A Label that is not allowed is
**still defined** at the address the line sits at, because the line is already
an error and an undefined name would be reported again at every use of it.

```
org_directive     = [ label_field ] "org" whitespace expression ;
equ_directive     =   label_field   whitespace "equ" whitespace expression ;
set_directive     =   label_field   whitespace "set" whitespace expression ;
dc_directive      = [ label_field ] "dc" [ size_suffix ] whitespace dc_item { "," dc_item } ;
dc_item           = string_literal | expression ;
ds_directive      = [ label_field ] "ds" [ size_suffix ] whitespace expression ;
dcb_directive     = [ label_field ] "dcb" [ size_suffix ] whitespace expression "," expression ;
end_directive     = [ label_field ] "end" [ whitespace expression ] ;
include_directive = [ label_field ] "include" whitespace file_specification ;
incbin_directive  = [ label_field ] "incbin" whitespace file_specification ;
reg_directive     =   label_field   whitespace "reg" whitespace register_list ;
fail_directive    = [ label_field ] "fail" [ whitespace message_text ] ;
simhalt_directive = [ label_field ] "simhalt" ;
opt_directive     = [ label_field ] "opt" whitespace option_name { "," option_name } ;
list_directive    = [ label_field ] ( "list" | "nolist" ) ;
page_directive    = "page" ;
offset_directive  = [ label_field ] "offset" whitespace expression ;
section_directive = [ label_field ] "section" [ whitespace expression ] ;

file_specification = quoted_file_name | bare_file_name ;
quoted_file_name   = ? a single- or double-quoted name, recognised in the raw
                       text of the Operand field and not as a string_literal
                       token ? ;
bare_file_name     = ? every character to the end of the Operand field (1.5) ? ;
message_text       = ? every character of the Operand field to the first `;` or
                       to the end of the line: whitespace, commas and quotes are
                       ordinary characters and nothing in it is tokenized ? ;
option_name        = identifier ;
raw_operand_field  = ? the same text as message_text, under the name it carries
                       on a refused Operation ? ;

text_operation     = "fail" | "include" | "incbin" | refused_operation ;
refused_operation  = "memory" | "macro" | "endm" | "mexit"
                   | "ifeq" | "ifne" | "iflt" | "ifle" | "ifgt" | "ifge"
                   | "ifc" | "ifnc" | "ifarg" | "endc"
                   | "if" | "else" | "endi" | "while" | "endw"
                   | "for" | "endf" | "repeat" | "until"
                   | "dbloop" | "unless" ;
macro_definition   = ? the line whose Operation is "macro", every line after it,
                       and the line whose Operation is "endm": skipped whole and
                       never tokenized (1.3) ? ;
```

`text_operation` is matched on the Operation's name alone, with any
`size_suffix` set aside, because the corpus writes `if.l` and `for.b`.
Its members are also Directive names for `label_rule` (1.4), so an `endm` in
column 1 is an Operation and not a Label. The conditional-assembly and
structured-control names are the help's own lists
(`Directives/conditional.htm`, `StrucControl/Introduction.htm`); the words that
only appear *inside* such a line — `then`, `do`, `to`, `downto`, `by` — are not
in the set, because the line they sit on is raw text by the time they are
reached.

Notes the shapes do not carry:

* `org` sets the current address, forwards or backwards. An **odd** address is
  EASy68K's warning (`odd_origin`) and is rounded up — but only when the `org`
  *moves* the address: an `org` to the address already in force moves nothing,
  so there is nothing to round up and nothing to say, whatever it is written as.
  So the same line, `ORG $1001`, warns and places at `$1002` after `ORG $1000`
  and is silent after `ORG $1000` and a `dc.b`. The rule is about the address
  and not about the text, and it is what keeps `org *` — the one documented way
  to end an `offset` region (`Directives/offset.htm`) — from warning about an
  address the program is legitimately at.
* `dc`, `ds` and `dcb` default to `.w` when no `size_suffix` is written, as the
  help says for `ds` and `dcb`; `dc` without a size is undocumented there and
  takes the same default, which is the only reading that makes `dc` and `dcb`
  agree.
* `ds.w 0` is EASy68K's idiom for alignment and is a legal Directive with a zero
  count, not an empty reservation to warn about. It is line 155 of
  `tests/corpus/easy68k/clockDigital.X68`.
* `end` takes an Expression, and it is optional: EASy68K warns "address
  expected" when it is missing rather than refusing the line.
* `fail` takes the **rest of the line** raw, so `FAIL ERROR, Argument missing in
  call to foo macro.` is one message and its commas and spaces are text
  (`Directives/fail.htm`). Its Operand field ignores rules 1 and 4 of 1.5, as a
  refused keyword's does — quotes and whitespace are ordinary characters in it —
  and keeps rule 2, so a `;` still starts a Comment there as it does everywhere
  else (1.6). The help says nothing about a `;` in a message,
  and one uniform Comment marker is worth more than a semicolon inside one.
* `include` and `incbin` take a quoted or a bare filename, and it is read from
  the raw text of the Operand field rather than from tokens, so a `.` in it is
  never a `size_suffix` and `include io.x68` needs no quotes. Quotes are
  required only when the path holds spaces
  ("must be enclosed in single (') or double (") quotes if any part of the file
  path or name includes spaces", `Directives/include.htm`), either kind of quote
  will do, neither is part of the name, a doubled quote inside a quoted name is
  one quote, and neither raises the `double_quoted_string` suggestion of 1.9. A
  backslash inside is never an escape: it is a path separator, like `/`.
* **The name is resolved against the Project, beside the including File first
  and at the project root second**, with `.` and `..` segments resolved on the
  way and a `..` that would climb above the root dropped, since a Project has no
  above-the-root. A name that resolves to no File of the Project is
  `unreadable_file`, whose hint is the closest existing paths — a File of the
  same *name* in another directory before any spelling distance — and which says
  so plainly when the Project holds no other File at all. An `include` of a
  binary File is the same kind, pointing at `incbin`; `incbin` takes either kind
  of File, a text one contributing its Latin-1 bytes (ADR 0004).
* **`include` is textual**: the included File's lines are assembled where the
  `include` line is, in the same section, at the same current address, in one
  Symbol namespace, with the Local label scopes running across the boundary
  (CONTEXT.md, "Include"). A Label on the `include` line names the current
  address, which is where the first included byte goes, exactly as a Label on a
  line of its own does. A File may be included more than once; one that would be
  included inside itself is `include_cycle`, and `include_too_deep` is the
  backstop under the nesting depth and the number of lines an assembly may take
  in. `incbin` places the File's bytes at the current address, aligning nothing,
  as a `dc.b` of the whole File would.
* **`end` belongs in the Entry file.** In an included File it is
  `end_in_an_included_file`, an error, where EASy68K would stop assembling there
  and drop the rest of the Entry file (ADR 0001); the line is refused and the
  lines below it are assembled as they would have been.
* `page` takes neither a Label nor Operands ("no label is permitted and any
  comments are ignored").
* `reg` takes a `register_list`, and a single `register` is a list of one, as it
  is for `movem` (1.12). The Symbol it defines holds the `movem` mask and not a
  value: a Register list may not appear in an Expression, which is EASy68K's
  "Register list symbol used in an expression" and this document's
  `register_list_in_expression`. A bare name where `movem` expects a list is
  read as the Symbol it names (1.12): a `reg` Symbol defined **above** the line
  becomes exactly the list it stands for, one defined below is "Register list
  symbol not previously defined" — the one forward reference refused in an
  instruction Operand, because a register list is not a value the second pass
  can fill in but part of how the instruction is encoded — and a Symbol of
  another kind is "Symbol is not a register list symbol". A name that is
  defined nowhere is left to `undefined_symbol` and to the analyzer's
  "what `movem` takes here", which between them are the diagnosis of a missing
  `reg` line. The last two are said only where **nothing else fits**: both of
  `movem`'s positions hold the list in one of its two directions, so
  `movem.l table,d0-d2` reads `table` as the address it is and says nothing.
  A name that *is* a `reg` Symbol is read as the list wherever it stands, since
  it has no value that could be an address.
* **The Layout ignores `simhalt`'s Operand field**, which is the help's own
  usage line, `LABEL SIMHALT comment` (`Directives/simhalt.htm`), and what
  `page`, `list` and `nolist` already do. The field is still *read* by the
  parser, as every field of every line but a `text_operation`'s is (2.5), so
  what follows `simhalt` is tokenized and a mistake inside it — an unclosed
  quote, a bad number — is still reported; what the Directive does is never ask
  for the Operands, so `SIMHALT                 Halt Simulator` (line 206 of
  `tests/corpus/easy68k/graphicSound.X68`) is not a `wrong_operand_count`
  although rule 4 of 1.5 makes `Halt` an Operand and `Simulator` a Comment, and
  no rule of the parser may consult the Directive's arity (ADR 0003). It is the one Directive that produces an executable item: four
  bytes at an even address, like an instruction, and a Label on it names that
  address.
* `section` takes a number 0–15, which may be written as a Symbol — the help's
  own example is `SECTION DATA` against `DATA EQU 1` — and switches to that
  section's location counter. A program has sixteen of them, each "restored to
  the address following the last location allocated in the indicated section (or
  to zero if used for the first time)" (`Directives/section.htm`). It begins in
  section 0, whose counter starts at s68k's default origin `$1000`; the other
  fifteen start at zero, as the help says, so a program that writes `section 1`
  and no `org` lays its data out from 0. An `org` inside a section sets that
  section's counter and no other. The number decides the Layout, so a forward
  reference is refused in it, and a number outside 0–15 is `value_out_of_range`
  naming `section`, the kind the address of an `org` and the count of a `ds` are
  already answered with. Two sections laid out over one address are the ordinary
  `address_used_twice`: EASy68K "does not check for overlapping sections" and
  s68k's check is a deliberate deviation (ADR 0001), and an address is an address
  whichever section wrote it. With no Operand `section` requires a Label and sets
  it to the number of the section in force — a **Constant**, because a section
  number is a value and not an address — which is the one label rule that depends
  on the Operand rather than on the name of the Directive.
* `offset` takes an Expression and opens a region that produces nothing: "no
  machine code is generated by instructions or directives following an OFFSET
  directive" (`Directives/offset.htm`). Inside it a `ds` moves a temporary
  counter that starts at the Expression, so the names in the label fields below
  are the offsets of the fields of a structure. They are **Constants** and not
  Labels: no line of the program is laid out at them and the value may be
  negative, the help's own stack frame counting from `-3*4`. Nothing is placed,
  so a line that would have produced bytes — an instruction, a `simhalt`, a `dc`
  — is `no_bytes_in_an_offset_region`, while a `ds` is what the region is made of
  and is silent. The Expression decides the Layout, so a forward reference is
  refused in it. An `org` ends the region; so does `end`, and so does a
  `section`, which sets the current address as an `org` does and about which the
  help is silent (ADR 0001's lenient reading). **`org *` inside a region is the
  address the region shadowed** — "ORG * restores the code to the address in use
  prior to the OFFSET" — and it is the one place where `*` is not the current
  address: every other `*` inside a region, `here equ *` included, is the
  region's own counter. An `org` that lands where the address already is moves
  nothing and is therefore neither rounded up nor answered with `odd_origin`,
  which is what keeps `org *` silent when a `dc.b` above the `offset` left the
  address odd.
* The refused Directives — `refused_operation` above — parse as
  `unimplemented_operation`, an `operation` followed by a `raw_operand_field`
  that is **not** tokenized. That is what keeps one "not implemented" diagnostic
  on `if.l d1 <hs> #NOON then.s` (line 48 of
  `tests/corpus/easy68k/clockDigital.X68`) instead of five syntax errors about
  `<`, and one on `for.b d3 = #1 to #7 do.s` (line 221 of
  `tests/corpus/easy68k/mouseWindowSize.X68`), whose `=` is not a token kind of
  1.14 at all. A `;` on such a line still ends the raw field and starts a
  Comment, so the corpus's `if <cs> then.s                ; if set` (line 223 of
  the same File) keeps its Comment. It is what the three
  `tests/corpus/easy68k/` fixtures are supposed to show.
* The lines between a `macro` and its `endm` are skipped whole, and this is the
  one line-range construct of the document (1.3): the parser enters the skip on
  an Operation named `macro`, leaves it on a line whose Operation is `endm`, and
  tokenizes nothing in between, not even a Label. The corpus's `move.l #\1,d1`
  (line 22 of `tests/corpus/easy68k/clockDigital.X68`) holds a Macro parameter
  and no grammar here describes `\1`. The `macro` line carries the single
  `unimplemented_operation` for the whole definition; the body and the `endm`
  line raise nothing. A `macro` never terminated would otherwise swallow the
  rest of the File in silence, so it raises `unterminated_macro_definition`
  against the `macro` line — EASy68K's own "ERROR: ENDM expected"
  (`errors.htm`) — and the skip ends at the end of the File.
* **Forward references** are not the parser's business. A Symbol used above its
  definition is allowed in an instruction Operand and in `dc` data and refused
  in `org`, `ds`, `dcb`, `equ` and `set`, where the value decides the Layout
  (the design record, "Symbols and expressions"; `Directives/org.htm`, "Forward
  references are not permitted"; EASy68K's "ERROR: Forward references not
  allowed with this directive"). The parser parses the same `expression` in all
  of them; phase 2 raises the diagnostic.

```
unimplemented_operation = operation [ whitespace raw_operand_field ] ;
```

### 2.7 `expression`

EASy68K's precedence table (`Directives/operators.htm`), highest first:
`>> <<`, then `& ! | ^`, then `* / \`, then `+ -`; equal precedence associates
left. As a layered grammar the lowest precedence sits outermost:

```
expression                = additive_expression ;
additive_expression       = multiplicative_expression
                            { ( "+" | "-" ) multiplicative_expression } ;
multiplicative_expression = bitwise_expression
                            { ( "*" | "/" | "\" ) bitwise_expression } ;
bitwise_expression        = shift_expression
                            { ( "&" | "!" | "|" | "^" ) shift_expression } ;
shift_expression          = unary_expression
                            { ( "<<" | ">>" ) unary_expression } ;
unary_expression          = [ unary_operator ] primary_expression ;
primary_expression        = number
                          | character_literal
                          | symbol_reference
                          | current_address
                          | grouped_expression ;
grouped_expression        = "(" expression ")" ;
current_address           = "*" ;
symbol_reference          = ? an identifier that is not one of the twenty-one
                              reserved register names of 1.10 ? ;
```

`unary_expression` writes one optional `unary_operator`; the parser accepts a
chain of them, so `~-5` and `--5` parse. That is the lenient direction of ADR
0001, it costs one recursive call, and no corpus program writes either.

A register name where an Expression term must stand is `register_in_expression`,
not a Symbol: `move.l #a0+4,d0` and `add.l 4+d0,d1` are that error. This is what
3.8 and 3.9 rest on, and it is the parser's counterpart of EASy68K's "Register
list symbol used in an expression", which is about a `reg` Symbol and stays the
evaluator's.

The parser implements this as a Pratt loop whose binding powers are the four
levels above; the layered rules are the specification and the loop is the
implementation. Worked from the corpus: `move.l #(800<<16+600),d1` is
`(800<<16)+600` (line 79 of `tests/corpus/easy68k/mouseWindowSize.X68`),
`add.l #COLS*2, a1` is a product (line 21 of
`tests/corpus/editor/two-dimensional-array-1.asm`), and `ORG (*+1)&-2` rounds
the current address up to a word (`Directives/org.htm`). `2**3` has no
derivation at all under these rules, and 1.13 says what is reported instead.

An Expression holds no whitespace: rule 4 of 1.5 has already ended the Operand
field at the first space that is not beside a comma. `#2 * 3` is therefore the
Immediate `#2` and a Comment; section 3.5 says what is reported.

---

## 3. Ambiguities, and how each is resolved

### 3.1 `loop move.l d0,d1` — a Label in column 1 with no colon

`loop` starts in column 1 and is not a Mnemonic or a Directive name, so it is a
Label and `move.l` is the Operation (`label_rule` row 4). This is the shape that
1.4.2 answered with "Unknown instruction" and the reason the front end is being
rewritten. Indent it and it becomes an Operation instead —
`    loop move.l d0,d1` is an unknown Mnemonic `loop`, with the hint that says
how to make it a Label again.

### 3.2 `clr` as a Label

`clr` in column 1 with no colon is the Operation `clr`, whatever the author
meant, because a lenient reading would silently turn a real instruction into a
Label. `clr:` is the Label — the colon is how EASy68K users who indent their
code never meet this case at all, and how the corpus writes `end:` in three
programs while `end` remains a Directive name.

### 3.3 `mvoe.l d0,d1` — a misspelt Mnemonic

`mvoe.l` carries a `size_suffix`, so `label_rule` refuses to read it as a Label
however it is indented. It reaches the analyzer as the Operation `mvoe` with
size `.l` and two well-formed Operands, which is what lets the answer be "did
you mean `move`?" and not "unknown instruction". Had the parser guessed a Label
in column 1, the two Operands would have become a second unknown Operation and
the diagnostic would have described a program nobody wrote.

### 3.4 `lea *,a0` and `nop * done` — `*` as a term or a marker

At the start of the Operand field, `*` is the `current_address`
(1.6). `lea *,a0`, `org *` and `org (*+1)&-2` are the forms that must work, and
EASy68K's own `Directives/offset.htm` writes `ORG    *    Restore previous code
origin` — an Operand `*` followed by a bare Comment.

The cost is that `nop * done here` also reads `*` as an Operand: the two lines
are the same shape and only the instruction table could separate them, which the
parser, `label_rule` aside, may not consult (ADR 0003). The analyzer resolves
it where it *is* allowed to look: an Operation that takes no Operands, given a
single `current_address` Operand, gets "`nop` takes no operands — the `*` was
read as the current address; start a comment with `;`" instead of a bare arity
error. The corpus never writes it: no program has a `*` in the Operand field
outside an Expression.

### 3.5 `#2 * 3` — an Expression with spaces in it

The Operand field ends at the space before `*` (1.5 rule 4), leaving the
Immediate `#2` and the Comment `* 3`. Nothing about that is a syntax error, so
the parser explains it rather than reporting it: it raises
`expression_split_by_space`, a **warning** — "the operand field ended at the
space before `*`; an expression contains no whitespace", hinted "write `#2*3`".

The trigger has to be narrow, because a `*` at the start of the comment field is
*also* the `explicit_comment` marker of 1.6: `move.l d0,d1   * copy` is ordinary
EASy68K style, and telling it to start its comment with `;` when it already
started one would be a diagnostic about nothing. The warning therefore fires
only when the comment field reads as the continuation of the Operand's
Expression — a `binary_operator`, then a number or a character literal, then the
end of the field, a `,`, or another `binary_operator`:

| Comment field | Fires | Why |
| --- | --- | --- |
| `* 3` after `#2` | yes | operator, literal, end of field |
| `* 3,d0` after `move.l #2` | yes | operator, literal, `,` — the shape that bites |
| `* 60` after `NOON equ 12*60` | yes | operator, literal, end of field |
| `* copy` | no | the term is a name, not a literal |
| `* one byte` after `dc.b 1` | no | likewise |
| `* 2 registers saved` | no | a word follows the literal |

A warning and not an error because the line may be perfectly deliberate, and
because in the shape that actually bites — `move.l #2 * 3,d0` — the analyzer's
"`move` needs a destination" is already the error and this warning is the
sentence that explains it. The hint names `;` only when the comment field is a
bare one, since a field that already carries a `*` or a `;` has its marker. No
line of the corpus has a comment field beginning with a binary operator at all
(section 6).

### 3.6 `d0 ,d1` — a space before a comma

The Operand field runs on: the whitespace run is beside a comma, so rule 4 does
not end the field and the Operand list is `d0,d1`, exactly as `d0, d1` would be.
The design record and the glossary both say "beside", symmetrically, and the
corpus writes `lea greeting, a1` in every `.asm` program.

That symmetry has a consequence the design record did not spell out: a **bare
Comment can never begin with a comma**, because a comma after the whitespace is
precisely what keeps the Operand field open. The hint the design record asks for
therefore attaches to the reachable form of the same mistake — a space *before* a
comma. `space_before_comma` is a *suggestion*, raised once per line when the
Operand field is continued across whitespace because the next non-blank
character is `,`: "the operand field continues past this space because a comma
follows it", hinted with "remove the space; if the comma starts a comment, write
`;` first". It fires on the harmless `d0 ,d1` as well, which is the price of
catching `trap #15   , display Y`, where the Operand list silently becomes
`#15,display`. No corpus program has a space before a comma anywhere.

### 3.7 A Comment field that is not marked

`move.b #23,d0   trap task 23` is EASy68K's bare Comment and it assembles; the
first one in a File raises the `bare_comment` suggestion and no later one does
(1.6). `tests/corpus/easy68k/` is written entirely in this style — `trap #15
draw line from X1,Y1 to X2,Y2` has commas in its prose and still ends where it
should, because the field ended at the space before `draw`.

The same rule swallows the commonest first-year typo, a space written where the
comma goes: in `move.l  d0 d1` the Operand field ends at the space, the line
genuinely has one Operand and the `d1` is a bare Comment. The analyzer's
`wrong_operand_count` says the count is wrong; `missing_comma_between_operands`,
a **warning** raised beside it, says where the second Operand went — "the operand
field ended at the space before `d1`, and `d1` was read as a comment", hinted
"write `d0,d1`". The `bare_comment` suggestion cannot do that job: it is raised
once per File and is spent on the first real comment.

Like 3.5's, the trigger is narrow — the Operand count has to be one the
Operation does not take *and* short of one it does, the Comment field has to be a
bare one, and its first word has to read as an Operand *and* be the whole of the
field:

| Line | Fires | Why |
| --- | --- | --- |
| `move.l  d0 d1` | yes | one Operand of two, and `d1` is a register |
| `add.w   d0 #4` | yes | `#` begins an Immediate |
| `move.l  d0 (a1)` | yes | `(` begins an indirect Operand |
| `move.l  d0 count` | yes | `count` is a Symbol the program defines |
| `move.l  d0 d1 ; copy` | yes | an explicit Comment after the word is still the same mistake |
| `move.l  d0 d1 is the target` | no | a word of prose follows the term |
| `move.l  d0 copy` | no | `copy` is no register, no Symbol and no Operand |
| `move.l  d0,d1 copy it` | no | the Operands are all there |

### 3.8 `(a0)` against `(label)` against `(640-COLS*SCALE)/2`

All three start with `(` and 2.5's `parenthesised_operand` separates them by the
two tokens after it: a register or `pc` followed by `)` or `,` is an Addressing
mode, anything else is an Expression, and an Expression whose `)` is followed by
a `binary_operator` keeps going. A Symbol named like a register cannot arise —
the twenty-one register names are reserved (1.10) and `symbol_reference`
excludes them (2.7) — so rule 1 never steals a `(label)` that meant an absolute
address.

### 3.9 `d0-d3` against subtraction

A register may not appear in an Expression (`symbol_reference`, 2.7; a register
where a term must stand is `register_in_expression`), so a `-` between two
registers is always a `register_range` and never a subtraction; there is no
reading of `d0-d3` in which the parser has to choose. `d0-3` is
`register_expected_in_register_list`, pointing at the `3`.

### 3.10 `dc.b` in column 1 against a Label named `dc`

A word carrying a `size_suffix` is never a Label (1.4), so the 6526 column 1
`dc.b` lines of `tests/corpus/editor/bad-apple.x68` are Operations. A Label
really named `dc` has to be written `dc:`, and the same File proves the
companion case by ending with `END:`.

### 3.11 A `;` inside an unclosed parenthesis

`move.l (a0,d1   ; save it` ends its Operand field at the `;` (1.5 rule 2) and
reports against the `(`: letting the `(` win would make the Operand
`(a0,d1 ; save it` and the diagnostic would be about the comment.

*Which* diagnostic follows the precedence of 2.5. Here `parenthesised_operand`
rule 1 has matched — `a0`, then `,` — so an Addressing mode has been recognised
and the report is `malformed_operand`, "this looks like an indexed operand,
`4(a0,d1.w)`, but the `)` is missing", with a related Location on the `(`.
`unclosed_parenthesis` is the other case, a `(` that opened a
`grouped_expression` with no mode recognised: `org (*+1&-2   ; word align` is
that one, "this `(` is never closed".

### 3.12 Lines after `end`

`end` ends the assembly of the Entry file; lines after it are ignored, wherever
they come from — an `include` below an `end` is still read, and a mistake in the
File it names is still reported, but nothing it brings in is assembled. Only a
line that *carries a Label or an Operation* raises the one `code_after_end`
warning, which the Directives phase raises and not the parser. Blank lines and
Comment lines never do, because all three `tests/corpus/easy68k/` programs close
with `END START` followed by EASy68K's own `*~Font name~Courier New~` editor
Comments, and warning about those would be noise about a File the editor wrote
itself.

---

## 4. The diagnostics the parser raises

These are the parser's own. All but three are raised while one line is being
read; `bare_comment`, `double_quoted_string` and `unterminated_macro_definition`
need a whole File — "the first one in a File", "no `endm` before the end of the
File" — and are raised by the pass over its lines. They are distinct
from the analyzer's (which judge Operands against the instruction table, and
which include the `missing_comma_between_operands` warning of 3.7, the one
diagnostic about the Comment field that the parser cannot raise because only the
instruction table knows an Operand is missing), from
the evaluator's (undefined Symbol, division by zero, constant over 32 bits,
character literal over four characters, a `reg` Symbol used in an Expression —
EASy68K's "Register list symbol used in an expression", whose counterpart for a
bare register name is the parser's `register_in_expression` below), from the
Directives' "not implemented", and from the four the Assembler raises about the
Files a Project is made of rather than about any line of one
(`unreadable_file`, `include_cycle`, `include_too_deep` and
`end_in_an_included_file`; 2.6). Codes are stable snake_case; `DiagnosticKind`
carries the data each message needs.

| Code | Severity | When | Message sketch | Hint sketch |
| --- | --- | --- | --- | --- |
| `character_above_latin1` | error | a source character above code 255 | "`’` cannot be stored: a character is one byte" | "write `'`" — names the plain look-alike when there is one |
| `non_breaking_space` | error | a `$A0` where whitespace was expected | "this is a no-break space, not a space" | "replace it with a space; it usually comes from a paste" |
| `unexpected_character` | error | a character that starts no token | "`?` cannot start anything here" | — |
| `unterminated_string` | error | a quoted literal reaches `line_end` | "this string is not closed before the end of the line" | "add the closing `'`; `''` writes a quote inside a string" |
| `invalid_number` | error | a digit that is not of the number's base, or a prefix with no digits | "`G` is not a hexadecimal digit" | "hexadecimal digits are 0-9 and A-F" |
| `number_too_large` | error | a number that does not fit in the 64 bits a value is computed in (1.8) | "`$ffffffffffffffffff` does not fit in the 64 bits a value is computed in" | "a value is computed in 64 bits and checked against the operand's size" |
| `unknown_size_suffix` | error | a `.` suffix whose run is one character and is not `b`, `w`, `l` or `s`, or any run of it in the operation field (1.11) | "`.q` is not a size" | "sizes are `.b`, `.w`, `.l`, and `.s` on branches" |
| `dot_in_name` | error | in the Operand field, a `.` in suffix position followed by two or more identifier characters (1.11) | "a name holds no dot after its first character" | "write `array_length`; a dot after a name is a size, `.b`, `.w`, `.l` or `.s`" |
| `reserved_name_as_symbol` | error | a Label, `equ`, `set` or `reg` name that is a register name | "`d0` is a register name and cannot be a symbol" | "pick another name, such as `d0_value`" |
| `two_labels_on_one_line` | error | a second colon-terminated identifier in the operation position | "`bar` is a second label on this line" | "one label to a line; put `bar:` on its own" |
| `operation_expected` | error | the operation position holds something that is not an identifier: after a Label (`loop: 5`), or at the start of a line that has none (`  #5`) | "`5` is not a mnemonic or a directive" | — |
| `empty_label` | error | a `:` with no identifier before it | "there is no name before this `:`" | "a label is a name, then the colon" |
| `operand_expected` | error | a `,` with no Operand after it, or an empty Operand before one | "an operand was expected after this comma" | "remove the comma if the operand list ends here" |
| `unclosed_parenthesis` | error | a `(` that opened a grouped Expression and is never closed, no Addressing mode having been recognised | "this `(` is never closed" | "add the `)`" — the Location *is* the `(`, so it carries no related Location; `malformed_operand` is the one that points back at it |
| `nesting_too_deep` | error | an Operand that nests more than 64 levels deep — nested `(`, or unary operators chained; a backstop against a pasted or machine-generated line, not a rule of the language | "this operand nests more than 64 levels deep and is not read further" | "no expression needs to nest that deep; check for a `)` that is missing" |
| `malformed_operand` | error | an Operand that started as a known Addressing mode and did not finish, an unclosed `(` after the mode was recognised included | "this looks like an indexed operand, `4(a0,d1.w)`, but the `)` is missing" | names the shape it tried to be, never "invalid syntax" |
| `unexpected_token_in_operand` | error | a token left over after a complete Operand, where a `,` or the end of the Operand field was expected | "`)` was not expected here" | "an operand ends at a comma or at the end of the operand field"; for a stray `)`, "there is no `(` for this `)`" |
| `register_expected_in_register_list` | error | a `-` or `/` in a register list not followed by a register | "a register was expected after `/`" | "a register list reads `d0-d3/a0-a2`" |
| `register_range_out_of_order` | error | `d5-d2`, `a2-d5` | "a range runs from the lower register to the higher one, in the order `d0`-`d7`, `a0`-`a7`" | "write `d2-d5`" |
| `register_in_expression` | error | a register name where an Expression term must stand | "`a0` is a register; an expression holds no registers" | "an expression is computed while assembling, when no register has a value yet" |
| `unterminated_macro_definition` | error | a `macro` with no `endm` before the end of the File | "this macro definition is never closed" | "macro definitions end with `endm`" — related Location on the `macro` |
| `expression_expected` | error | a `#`, an operator or a `(` with no term after it | "an expression was expected after `+`" | — |
| `plus_is_not_a_unary_operator` | error | a `+` where a term must begin | "`+` is not a unary operator" | "write `5`; the unary operators are `-` and `~`" |
| `expression_split_by_space` | warning | the comment field reads as the continuation of the Operand's Expression: a `binary_operator`, a number or character literal, then the end of the field, a `,`, or another operator (3.5) | "the operand field ended at the space before `*`; an expression contains no whitespace" | "write `#2*3`", and "start a comment with `;`" only when the comment field is a bare one (3.5) |
| `bare_comment` | suggestion | the first bare Comment field in a File | "this is EASy68K's comment field; s68k reads it as a comment" | "start comments with `;` to say so" |
| `space_before_comma` | suggestion | the Operand field is continued across whitespace because a `,` follows it | "the operand field continues past this space because a comma follows it" | "remove the space; if the comma starts a comment, write `;` first" |
| `double_quoted_string` | suggestion | the first double-quoted string or character literal in a File; never a `file_specification` (1.9) | "EASy68K writes strings in single quotes" | "`'Hello'` assembles in both" |

Two of those overlap and the order matters. **`malformed_operand` wins whenever
an Addressing mode has been recognised** — `parenthesised_operand` rule 1
matched, or its rule 2 saw the `,` after the Expression (2.5) — because it can
name the shape the Operand was trying to be. `unclosed_parenthesis` is left with
the case where the `(` opened a `grouped_expression` and no mode was ever
recognised. 3.11 works one of each.

`unexpected_token_in_operand` is the general "the operand does not end here",
EASy68K's "ERROR: Comma expected — The operand is not complete", and it is what
`2**3` (1.13), `move.l d0),d1` and any other trailing token reach. It is not
`malformed_operand`: nothing was recognised and left unfinished, so no shape can
be named, and a diagnostic that named one would describe a program nobody wrote.

---

## 5. The rule index

Every name this document defines, in the order it is introduced. The parser
tests carry these names.

**Lexical** — `character_set`, `line_end`, `whitespace`, `case_rule`,
`source_line_fields`, `label_rule`, `operand_field_extent`, `comment_rule`,
`identifier`, `global_identifier`, `local_identifier`, `letter`, `digit`,
`number`, `hexadecimal_number`, `binary_number`, `octal_number`,
`decimal_number`, `hex_digit`, `octal_digit`, `string_literal`, `single_quoted`,
`double_quoted`, `character_except_quote`, `character_except_double_quote`,
`character_literal`, `character`, `register`, `data_register`,
`address_register`, `program_counter`, `special_register`, `size_suffix`,
`register_list`, `register_list_item`, `register_range`, `unary_operator`,
`binary_operator`, `punctuation`.

**Lines** — `source_file`, `source_line`, `blank_line`, `comment_line`,
`code_line`, `explicit_comment`, `bare_comment`, `comment_field`, `label_field`,
`colon_label`, `column_one_label`, `operation_field`, `operation`,
`operation_name`.

**Operands** — `operand_field`, `operand_list`, `text_operand_field`, `operand`,
`immediate`, `data_register_direct`, `address_register_direct`, `indirect`,
`postincrement`, `predecrement`, `displacement`, `index`, `pc_displacement`,
`pc_index`, `absolute`, `index_register`, `parenthesised_operand`.

**Directives** — `org_directive`, `equ_directive`, `set_directive`,
`dc_directive`, `dc_item`, `ds_directive`, `dcb_directive`, `end_directive`,
`include_directive`, `incbin_directive`, `reg_directive`, `fail_directive`,
`simhalt_directive`, `opt_directive`, `list_directive`, `page_directive`,
`offset_directive`, `section_directive`, `file_specification`, `bare_file_name`,
`message_text`, `option_name`, `raw_operand_field`, `unimplemented_operation`,
`quoted_file_name`, `text_operation`, `refused_operation`, `macro_definition`.

The Directive rules are the one group whose tests are not named after them:
§2.6 belongs to the Directives phase, which implements them in
`src/assembler/layout.rs`, and its tests are named after the behaviour each rule
decides. The map, so that a rule can be found from its name:

| Rule | Test in `src/assembler/layout.rs` |
| --- | --- |
| `org_directive` | `org_moves_the_address_anywhere`, `an_odd_origin_warns_and_rounds_up`, `an_org_that_moves_nothing_says_nothing_about_an_odd_address`, `org_reads_the_current_address_before_it_moves` |
| `equ_directive` | `equ_names_a_value_and_the_program_keeps_it`, `equ_without_a_name_says_so`, `a_constant_is_defined_even_when_its_value_cannot_be_worked_out` |
| `set_directive` | `a_set_variable_may_be_redefined` |
| `dc_directive`, `dc_item` | `dc_lays_strings_out_in_latin_1_and_pads_them_to_its_size`, `a_character_above_latin_1_in_data_is_an_error`, `a_data_item_that_does_not_fit_its_size_says_so` |
| `ds_directive` | `ds_reserves_its_room_and_writes_nothing`, `ds_w_zero_is_the_alignment_idiom` |
| `dcb_directive` | `dcb_fills_its_block`, `a_count_that_does_not_fit_in_memory_says_so` |
| `end_directive` | `the_entry_point_is_end_then_start_then_the_first_instruction`, `end_without_an_address_warns_and_falls_back`, `end_finds_a_label_that_differs_only_in_case_and_warns`, `a_line_after_end_is_not_assembled_and_says_so_once`, `a_comment_after_end_is_what_every_easy68k_program_has` |
| `opt_directive`, `list_directive`, `page_directive` | `the_ignored_directives_are_ignored_in_silence` |
| `reg_directive` | `reg_names_a_register_list_and_movem_reads_it_in_both_directions`, `reg_takes_a_single_register_as_a_list_of_one`, `reg_without_a_name_says_so`, `reg_takes_a_register_list_and_nothing_else`, `a_register_list_in_an_expression_is_refused`, `a_register_list_has_to_be_defined_above_the_movem_that_reads_it`, `a_name_that_is_not_a_register_list_says_so`, `a_name_that_is_defined_nowhere_keeps_its_own_message` |
| `fail_directive` | `fail_reports_its_message_word_for_word_and_the_assembly_carries_on`, `fail_without_a_message_uses_easy68ks_default`, `a_label_on_a_fail_names_the_address_of_the_line` |
| `simhalt_directive` | `simhalt_is_an_instruction_of_four_bytes`, `simhalt_ignores_the_rest_of_its_line`, and `simhalt_ends_the_run_where_it_stands_and_touches_no_register` in `src/test/test.rs` |
| the label field of every rule above | `the_directives_that_give_a_name_to_something_need_a_label`, `the_directives_that_take_no_label_say_so`, `every_other_directive_takes_a_label_or_no_label` |
| `section_directive` | `a_program_starts_in_section_zero_at_the_default_origin`, `a_label_on_a_section_names_the_address_it_goes_on_from`, `section_switches_between_sixteen_location_counters`, `org_inside_a_section_moves_that_sections_counter`, `a_section_that_is_used_for_the_first_time_starts_at_zero`, `two_sections_over_one_address_are_still_an_overlap`, `a_section_number_may_be_a_symbol_and_may_not_be_a_forward_reference`, `a_section_number_outside_the_sixteen_says_so`, `section_with_no_number_names_the_section_in_force`, `section_with_no_number_needs_a_label`, and `the_section_example_of_the_help` in `src/test/corpus.rs` |
| `offset_directive` | `offset_moves_an_address_and_places_nothing`, `a_name_defined_in_an_offset_region_is_a_constant`, `org_star_restores_the_address_the_offset_region_shadowed`, `an_org_with_an_address_ends_an_offset_region_too`, `a_section_ends_an_offset_region_as_an_org_does`, `end_closes_an_offset_region`, `a_line_that_would_produce_bytes_in_an_offset_region_says_so`, `an_offset_region_opens_whatever_its_expression_says`, `a_negative_offset_is_the_stack_frame_of_the_help`, `a_word_in_an_offset_region_aligns_up_from_a_negative_offset`, and `the_offset_stack_frame_example_of_the_help` in `src/test/corpus.rs` |
| `include_directive`, `incbin_directive` | `src/test/include.rs` whole, and `the_included_lines_are_assembled_where_the_include_line_is`, `there_is_one_symbol_namespace_and_it_reaches_both_ways`, `local_label_scopes_run_across_the_boundary`, `a_label_on_an_include_line_names_the_first_included_byte`, `end_belongs_in_the_entry_file`, `incbin_is_a_dc_b_of_the_whole_file` in particular; the resolution, the cycle and the two backstops are in `src/assembler/include.rs` |
| `macro_definition` | `macro_definition_is_skipped_whole`, `macro_definition_ends_on_an_operation_named_endm`, `unterminated_macro_definition_is_reported_against_the_macro_line` (in `src/assembler/parser.rs`), and `a_macro_invocation_names_the_macro_and_not_a_label` |
| every one of them, on the shapes they share | `a_directive_given_an_addressing_mode_says_a_value_was_expected`, `a_directive_with_the_wrong_number_of_operands_says_so`, `a_directive_that_carries_no_size_says_so`, `a_forward_reference_is_refused_where_the_value_decides_the_layout` |

**Expressions** — `expression`, `additive_expression`,
`multiplicative_expression`, `bitwise_expression`, `shift_expression`,
`unary_expression`, `primary_expression`, `grouped_expression`,
`current_address`, `symbol_reference`.

---

## 6. What the corpus was checked against

Every rule above was run against the 33 programs of `tests/corpus/` — the 30 the
asm-editor ships and the 3 EASy68K originals — as a throwaway model of 1.4, 1.5,
2.5 and 2.7: split each of their 9672 lines into the four fields, then parse every
Operand field that is not a Macro body or a structured-control line. Every line
split, and all 8711 Operand fields parsed. What the pass turned up is in this document
already: the parenthesised Expression of `bad-apple.x68` line 30 (3.8), the
`END:` Label (3.10), the 6526 column 1 `dc.b` lines (3.10), `end` used as a
Label in three programs (3.2), the space inside `(a0, d5)` (1.5), the apostrophe
in a bare Comment (1.5), `andi.w #$00,SR` (1.10), and the EASy68K editor
Comments after `END START` (3.12). No program in the corpus writes a Local
label, a binary or octal number, a doubled `''`, a double-quoted string, a
space before a comma, or a `*` in the Operand field outside an Expression; those
rules are specified from the EASy68K help alone and their tests have to be
written by hand.

The pass was run again when this document was reviewed, to measure two rules it
had taken for granted. With the text Operand fields of 2.6 in place — `fail`,
`include`, `incbin`, the refused keywords and the `macro` … `endm` skip — 0 of
the 8711 Operand fields fail; without them exactly two lines do, `move.l  #\1,d1`
(line 22 of `tests/corpus/easy68k/clockDigital.X68`, inside a Macro body) and
`if <cs> then.s` (line 223 of
`tests/corpus/easy68k/mouseWindowSize.X68`), which is the whole of what those
rules buy on this corpus. And 0 of the 9672 lines have a comment field whose
first token is a `binary_operator`, so neither the wide form of
`expression_split_by_space` nor the narrow one of 3.5 fires anywhere here: that
warning is specified from the help and from reasoning alone, and its tests are
hand written too.

The pass was run a third time with the parser itself, once it existed
(`src/assembler/parser.rs`, and the implementation notes' step 4). The
throwaway model's numbers hold: all **8957** lines of the 30 `editor/` programs
parse with **no Diagnostic at all**, and so do the same lines with their
indentation removed, each giving the same Label, Operation and Operands as the
indented original — which is the measure of `label_rule` resting on column 1 and
on nothing else. The 3 `easy68k/` originals (**715** lines) raise **one**
Diagnostic each in two of them, the `bare_comment` suggestion of 1.6 on the
first bare Comment field of the File, and none at all in the third, which is
written in `;` comments throughout; their Macro definitions, conditional
assembly and structured control raise nothing, because 2.6 keeps all of it out
of the tokenizer. Read line by line *without* the Macro skip the same three
programs raise one error, `expression_expected` on the `move.l #\1,d1` of
`clockDigital.X68` line 22, which is exactly what section 6's second measurement
predicted.
