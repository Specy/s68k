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

- **Interpreter** (`src/interpreter.rs`): fed a program, it runs it — registers, memory, flags, one step at a time or to the end, with an undo history, breakpoints given as `{ file, line }`, and a call stack that names the routine each frame is in. It never reads source and never parses anything.

A **diagnostic** carries a severity (error, warning or suggestion), a stable code, a message, often a hint saying what to write instead, and related locations; it is what the editor draws and what this project is really about.

**WARNING** as this is only an interpreter, it does not load the actual program in memory so it won't be possible to modify instructions at runtime. Every instruction takes four bytes whatever it would encode to on a real 68000; the program counter steps by the size stored with each instruction, so real sizes are a change to the assembler alone.

## Might do
- Real instruction encodings, and a program loaded in memory
- Macros and conditional assembly
- Disassembler (unlikely)


## Supported instructions
| Type                   | Instructions                                                                                                                                                                                                      |
|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Arithmetic             | add, sub, suba, adda, divs, divu, muls, mulu, addq, subq, addi, subi                                                                                                                                              |
| Comparison             | tst, cmp, cmpi, cmpa, cmpm                                                                                                                                                                                        |
| Branching and jumping  | bcc, bcs, beq, bne, blt, ble, bgt, bge, bls, bhi, bpl, bmi, blo, bhs, bvc, bvs, bsr, bra, jmp, jsr, rts, dbcc, dbcs, dbeq, dbne, dbge, dbgt, dble, dbls, dblt, dbhi, dbmi, dbpl, dbvc, dbvs, dbf, dbt, dbhs, dblo, dbra |
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st, shs, slo                                                                                                                            |
| Bitwise                | not, or, ori, and, andi, eor, eori, lsl, lsr, asr, asl, rol, ror, btst, bclr, bchg, bset                                                                                                                          |
| Other                  | clr, exg, neg, ext, extb, swap, move, link, unlk, lea, pea, moveq, movea, movem, nop                                                                                                                              |
| Interrupt              | trap #15, with the I/O tasks 0-9, 11, 13-15, 17-20, 23, 24 and 33 in `d0`, the mouse task 61 and the graphics tasks 80-96 (the `Interrupt` enum of `src/instructions.rs` is the list)                             |

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
| `opt`, `list`, `nolist`, `page` | accepted and ignored: they are about the listing file, which there is none of |

Without `end`, the entry point is a label named `START`, and failing that the first instruction. There is no `even`: `ds.w 0` is EASy68K's idiom for it, and word and long data align on their own anyway.

`include`, `incbin`, `reg`, `fail`, `simhalt`, `offset` and `section` are recognised and refused with a diagnostic naming the feature and, where there is one, what to write instead; they are the next piece of work. `memory`, the macro directives and conditional assembly are refused the same way, and macros are the one feature that may come back later.

## Todo
- The directives above that are still refused
- The instructions the table carries a "not implemented" reason for: `movep`, `addx`, `subx`, `negx`, `abcd`, `sbcd`, `nbcd`, `roxl`, `roxr`, `tas`, `rtr`, `chk`, `trapv`, `illegal`
- `sr`, `ccr` and `usp`, and the PC-relative addressing modes
- Real instruction sizes

## Known limitations
1. Characters are one byte, read and written as Latin-1; a source character with no byte of its own is an assembly error. This is a decision and not a bug — a program that writes `dc.b 'é'` has to put one byte in memory.
2. Every instruction is four bytes wide whatever it encodes to on a real 68000, so an address computed from instruction sizes will not match the hardware.
3. The program runs as supervisor, always: the status register is stored and readable, and its trace, supervisor and interrupt-mask bits have no effect.

# How to run rust
Firstly make sure you have rust installed, [you can download it here](https://www.rust-lang.org/tools/install), once done, clone the repository on your machine and run `cargo run` in the root folder of the project. This will assemble and run the code inside of the `code-to-run.asm` file; name another file to run that one instead, `cargo run -- my-program.asm`.

Every diagnostic is printed as `file:line:column: severity: message`, with its hint under it, and a program with an error in it is not run. `--step` steps through it (D, A, S and Q for step, undo, print and quit), `--show-program` prints the assembled instructions, `--benchmark` runs it with no undo history and times it, and `--no-debug` leaves out the registers at the end.

# How to build WASM binary
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the project by running `npm run build-wasm` in the `ts-lib` folder of the project. This will create a `pkg` folder in the ts-lib one with the compiled code.

# How to try the WASM binary locally
`ts-lib` is the supported way to use the library, and `ts-lib/test/smoke.mjs` is a working example of the 2.0 API: `npm ci`, `npm run build-lib` and `npm test` in `ts-lib` assemble and run a program through the built package.

The `web` folder holds a small webpack demo which **has not been ported to the 2.0 API** and does not build: it calls `new S68k(code)`, `wasm_semantic_check` and `wasm_compile`, which the rewrite removed. `web/README.md` says what porting it needs. Neither it nor the `pkg/` folder it imports is in any CI job, and `pkg/` is no longer committed — it is `wasm-pack build`'s output and is now ignored, like `ts-lib/src/pkg`.

