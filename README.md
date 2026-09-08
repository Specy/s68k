# s68k
A rust assembler and interpreter for m68k, written to explain what is wrong with a program as much as to run it.
It compiles to WASM to be used with javascript and node.js, [available on npm](https://www.npmjs.com/package/@specy/s68k)

[Typescript library documentation](https://github.com/Specy/s68k/wiki)
It is part of a family of javascript assembly interpreters/simulators: 

- MIPS: [git repo](https://github.com/Specy/mars),  [npm package](https://www.npmjs.com/package/@specy/mips)
- RISC-V: [git repo](https://github.com/Specy/rars), [npm package](https://www.npmjs.com/package/@specy/risc-v)
- X86: [git repo](https://github.com/Specy/x86-js), [npm package](https://www.npmjs.com/package/@specy/x86)
- M68K: [git repo](https://github.com/Specy/s68k), [npm package](https://www.npmjs.com/package/@specy/s68k)


From my tests, it runs at 50/70mhz natively and 30/40mhz on the browser with WASM

## Purpose
The purpose of this interpreter is to help people learn the basics of assembly, in this case m68k, by providing useful errors and hints like a modern language, in the hope to help understand the addressing modes and learn the different instructions/directives.
**WARNING**
It wasn't made to assemble or make actual programs, but marely as a learning tool, don't expect 100% accuracy.

## Workings
There are two halves, and one thing between them.

- **Assembler** (`src/assembler/`): the whole front end. It reads the source files of a project, starting from the entry file, and answers with the diagnostics it found and, when none of them is an error, with the program. Inside it are a tokenizer and a hand-written parser (one line at a time, with a Pratt loop for expressions), a symbol table, an expression evaluator, the layout that gives every line an address, one instruction table that says which addressing modes and sizes each mnemonic takes, and an analyzer that measures every line against it. Assembly does not stop at the first mistake: every phase adds to one list of diagnostics, so a student sees the whole build.

- **Program**: what the assembler produces — the assembled instructions with their addresses, sizes and source locations, the initial contents of memory, the symbols, and the entry point. It is the only thing the interpreter reads, and it holds no source: an instruction reaches its line through its location, which is a file, a line and a range of columns.

- **Interpreter** (`src/interpreter.rs`): fed a program, it runs it — registers, memory, the status register, one step at a time or to the end, with an undo history, breakpoints given as `{ file, line }`, and a call stack that names the routine each frame is in. It never reads source and never parses anything.

A **diagnostic** carries a severity (error, warning or suggestion), a stable code, a message, often a hint saying what to write instead, and related locations; it is what the editor draws and what this project is really about.

**WARNING** as this is only an interpreter, it does not load the actual program in memory so it won't be possible to modify instructions at runtime. Every instruction takes four bytes whatever it would encode to on a real 68000; the program counter steps by the size stored with each instruction, so real sizes are a change to the assembler alone.

## Might do
- Real instruction encodings, and a program loaded in memory
- Macros and conditional assembly
- Disassembler (unlikely)


## Supported instructions
| Type                   | Instructions                                                                                                                                                                                                      |
|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Arithmetic             | add, sub, suba, adda, divs, divu, muls, mulu, addq, subq, addi, subi, addx, subx, negx                                                                                                                            |
| Comparison             | tst, cmp, cmpi, cmpa, cmpm                                                                                                                                                                                        |
| Branching and jumping  | bcc, bcs, beq, bne, blt, ble, bgt, bge, bls, bhi, bpl, bmi, blo, bhs, bvc, bvs, bsr, bra, jmp, jsr, rts, dbcc, dbcs, dbeq, dbne, dbge, dbgt, dble, dbls, dblt, dbhi, dbmi, dbpl, dbvc, dbvs, dbf, dbt, dbhs, dblo, dbra |
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo, and `move`/`andi`/`ori`/`eori` with `sr` or `ccr` as an operand                                                           |
| Bitwise                | not, or, ori, and, andi, eor, eori, lsl, lsr, asr, asl, rol, ror, roxl, roxr, btst, bclr, bchg, bset                                                                                                              |
| Binary coded decimal   | abcd, sbcd, nbcd                                                                                                                                                                                                 |
| Other                  | clr, exg, neg, ext, extb, swap, move, link, unlk, lea, pea, moveq, movea, movem, movep, tas, nop                                                                                                                  |
| Exceptions             | chk, trapv, illegal, and rtr, which returns and restores the condition codes                                                                                                                                      |
| Interrupt              | trap #15, with the I/O tasks 0-9, 11, 13-15, 17-20, 23, 24 and 33 in `d0`, the mouse task 61 and the graphics tasks 80-96 (the `Interrupt` enum of `src/instructions.rs` is the list)                             |

`move <ea>,ccr`, `move <ea>,sr` and `andi`/`ori`/`eori` into either take the modes and the sizes of the 68000: a data operand and a word into `sr` or `ccr`, a byte for the immediate into `ccr`. `move sr,<ea>` writes any data alterable operand. `move ccr,<ea>` is the one instruction here a real 68000 does not have — it belongs to the 68010 — and s68k assembles it because reading the condition codes back is worth more to a student than the distinction.

`addx`, `subx`, `abcd` and `sbcd` take two data registers or two predecrement operands and nothing else, which is what the 68000 gives them; `negx` and `nbcd` take one data alterable operand, and `roxl` and `roxr` the three shapes of the other shifts. The three decimal instructions work on one byte. All eight carry the extend flag, and the six that do arithmetic with it — `addx`, `subx`, `negx`, `abcd`, `sbcd` and `nbcd` — carry the Z flag rule that makes a number of any width testable: **Z is cleared when the result is not zero and left alone when it is**, so a program sets Z, works up from the least significant piece and reads Z at the end. `roxl` and `roxr` set Z from the result, as every other shift does. `abcd`, `sbcd` and `nbcd` leave N and V exactly where they were, which is what the reference calls undefined for them.

## Supported addressing modes
Every one the 68000 has: `d0`, `a0`, `(a0)`, `(a0)+`, `-(a0)`, `4(a6)`, `4(a6,d1.w)`, `label(pc)`, `label(pc,d1.w)`, an absolute address (`$2000`, `label`, and `label.w` or `label.l` to force a width), `#5`, and `sr` and `ccr` where an instruction reaches the status register. Both spellings of every displaced mode are accepted, `4(a6)` and `(4,a6)` alike, and `(a0,d1.w)` and `(pc,d1.w)` are a displacement of zero. Which modes an instruction takes where is the instruction table's answer, and a rejection names what was found, what is allowed there and, where the mistake has a name, what was probably meant.

**A PC-relative operand is written as the address it reaches**, as in EASy68K: `move.l data(pc),d0` reads `data`. The assembler works out the distance from the instruction to it — from the extension word, which is the instruction's address plus two — and the interpreter adds the two back, so the operand reaches the same place wherever it is written. The distance is a signed word for `label(pc)` and a signed byte for `label(pc,xn)`, and a label too far away is an error saying how far it is. Nothing is written through the program counter: a PC-relative operand is refused as a destination, with the reason.

**`label.w` and `label.l` name the same address here.** s68k stores addresses and encodes no instruction words, so forcing a width says nothing about the program, and `.l` is accepted in silence; `.w` is a claim that the address fits the sixteen bits of an absolute short reference, and it is checked (EASy68K's "Absolute address exceeds 16 bits"). Unlike EASy68K, s68k does not sign extend a short address, which is why the check is an error rather than the warning EASy68K gives — `$8000.w` reads `$ff8000` there and would read `$8000` here. A `.b` or `.s` after an address is refused with the reminder that the size the instruction works at goes after the mnemonic.

## Supported directives
| Directive | What it does |
|---|---|
| `org` | sets the address the following lines are laid out at, forwards or backwards; the default origin is `$1000` |
| `equ` | names a value, once |
| `set` | names a value that may be set again; each use sees the latest definition above it |
| `dc` | puts values or a string in memory, `.b`, `.w` or `.l` |
| `dcb` | puts a value in memory a given number of times |
| `ds` | reserves room and writes nothing to it |
| `end` | ends the program and, with an operand, sets the entry point |
| `reg` | names a `movem` register list, `AllRegs reg d0-d7/a0-a6`, for `movem.l AllRegs,-(sp)` |
| `fail` | reports the rest of the line as an error of the program's own; the assembly carries on |
| `simhalt` | ends the run where it stands, modifying no register |
| `section` | switches between the sixteen location counters, 0 to 15, each going on from where it was left |
| `offset` | opens a region that produces no bytes, where `ds` names the fields of a structure by their offsets; `org *` ends it |
| `include` | assembles another file of the project here, as if its lines had been pasted at this line |
| `incbin` | puts another file's bytes in memory here, as a `dc.b` of the whole file would |
| `opt`, `list`, `nolist`, `page` | accepted and ignored: they are about the listing file, which there is none of |

Without `end`, the entry point is a label named `START`, and failing that the first instruction. There is no `even`: `ds.w 0` is EASy68K's idiom for it, and word and long data align on their own anyway.

`equ`, `set` and `reg` need a label, and so does a `section` with no number, which it sets to the number of the section in force; `page` and the conditional directives take none. A `reg` list has to be defined above the `movem` that reads it, and it may not appear in an expression. Unlike EASy68K, s68k offers no way to resume a run after a `simhalt`.

A program starts in section 0 at the default origin `$1000`; the other fifteen sections start at 0, as in EASy68K. A name defined inside an `offset` region is a constant and not a label: it stands for an offset, which may be negative, and no line of the program is laid out at it. Anything that would produce bytes inside a region — an instruction, a `dc`, a `simhalt` — is an error saying so.

`memory`, the macro directives and conditional assembly are refused with a diagnostic naming the feature, and macros are the one feature that may come back later.

## Projects of several files

A **project** is a map from a root-relative path to a file's text or to its bytes, plus the path of the **entry file** to assemble. Every other file is reached from the entry file through `include` or `incbin`, or is not read at all; nothing on a disk is ever opened by the assembler, which is what lets the same code run in a browser over the editor's buffers.

```ts
S68k.assemble({
    files: {'main.x68': source, 'lib/io.x68': library, 'data/sprite.bin': bytes},
    entry: 'main.x68'
})
```

`include` is **textual**: the included file's lines are assembled where the `include` line is, in the same section, at the same address, in one symbol namespace, with the local label scopes running across the boundary. The file name may be quoted with either quote or not quoted at all, `\` is a separator like `/`, and the path is looked for beside the file that wrote it first and at the project root second. A file may be included more than once — a name it defines is then defined twice, and the error says which two `include` lines did it — but not inside itself, which is an error showing the chain. A file the project has not got is an error naming the closest paths it does have. `end` belongs in the entry file and nowhere else.

`incbin` takes either kind of file: a binary one contributes its bytes and a text one its Latin-1 bytes. It aligns nothing and a label on it names its first byte.

Every location carries the file it is in, so a diagnostic, a breakpoint (`{file, line}`), the current line and a call-stack frame all name one file of the project. A diagnostic raised in an included file carries the `include` lines it was reached through as related locations, and so does every assembled instruction (`includeChain`, innermost first): a file included twice has one location per line and two addresses, and the chain is the only thing that tells the two copies apart.

## Todo
- The directives above that are still refused
- Real instruction sizes

## Known limitations
1. Characters are one byte, read and written as Latin-1; a source character with no byte of its own is an assembly error. This is a decision and not a bug — a program that writes `dc.b 'é'` has to put one byte in memory.
2. Every instruction is four bytes wide whatever it encodes to on a real 68000, so an address computed from instruction sizes will not match the hardware. It is also why a PC-relative operand is resolved while the program is assembled: there is no extension word in memory to read the displacement from, so the assembler stores it and the interpreter adds it to the address of the instruction being executed.
3. The program runs as supervisor, always. The status register is a 16-bit register whose low byte is the condition codes and whose high byte — trace, supervisor and the interrupt mask — is stored, readable (`getSr()`, and the `SR:` line of the command line) and of no effect at all. It starts at `$2700`, as in EASy68K, so `andi #$00,sr` "puts the CPU in user mode" and changes nothing that runs. `move usp,an` and `move an,usp` are refused for the same reason: there is one stack pointer, `a7`.
4. `chk`, `trapv` and `illegal` raise their exception by ending the run, as an address error does: there are no exception vectors, no supervisor stack frame and no `rte`, so a program cannot handle one. The runtime error names the instruction and its cause.

# How to run rust
Firstly make sure you have rust installed, [you can download it here](https://www.rust-lang.org/tools/install), once done, clone the repository on your machine and run `cargo run` in the root folder of the project. This will assemble and run the code inside of the `code-to-run.asm` file; name another file to run that one instead, `cargo run -- my-program.asm`.

The **directory the named file is in is the project**: every file under it is one file of it, named by its path inside it, so `cargo run -- dir/main.asm` assembles `dir/main.asm`'s `include 'lib/io.x68'` from `dir/lib/io.x68`. A file named `.asm`, `.x68`, `.m68k`, `.s` or `.inc` is read as source and every other file as bytes, for `incbin`; a directory whose name starts with `.`, and `target` and `node_modules`, are not read at all; and no more than 1000 files and 32 MiB are read, source first, with a message when something is left out — a directory is not really a project.

Every diagnostic is printed as `file:line:column: severity: message`, with its hint and its related locations under it — the `include` lines a file was reached through among them — and a program with an error in it is not run. `--step` steps through it (D, A, S and Q for step, undo, print and quit), `--show-program` prints the assembled instructions, `--benchmark` runs it with no undo history and times it, and `--no-debug` leaves out the registers at the end.

# How to build WASM binary
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the project by running `npm run build-wasm` in the `ts-lib` folder of the project. This will create a `pkg` folder in the ts-lib one with the compiled code.

# How to try the WASM binary locally
`ts-lib` is the supported way to use the library, and `ts-lib/test/smoke.mjs` is a working example of the 2.0 API: `npm ci`, `npm run build-lib` and `npm test` in `ts-lib` assemble and run a program through the built package.

The `web` folder holds a small webpack demo which **has not been ported to the 2.0 API** and does not build: it calls `new S68k(code)`, `wasm_semantic_check` and `wasm_compile`, which the rewrite removed. `web/README.md` says what porting it needs. Neither it nor the `pkg/` folder it imports is in any CI job, and `pkg/` is no longer committed — it is `wasm-pack build`'s output and is now ignored, like `ts-lib/src/pkg`.

