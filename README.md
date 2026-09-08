# s68k

s68k is an M68K assembler and interpreter written in Rust. The assembler reports useful errors and warnings to help with learning assembly. The emulator runs the assembled program (not in memory) and implements useful debugging features to step, undo, add breakpoints through the program

The library compiles to WebAssembly for use from JavaScript and Node.js. It is
[available on npm](https://www.npmjs.com/package/@specy/s68k).

It is part of a family of JavaScript assembly interpreters and simulators:

- MIPS: [git repo](https://github.com/Specy/mars),  [npm package](https://www.npmjs.com/package/@specy/mips)
- RISC-V: [git repo](https://github.com/Specy/rars), [npm package](https://www.npmjs.com/package/@specy/risc-v)
- X86: [git repo](https://github.com/Specy/x86-js), [npm package](https://www.npmjs.com/package/@specy/x86)
- M68K: [git repo](https://github.com/Specy/s68k), [npm package](https://www.npmjs.com/package/@specy/s68k)
- Z80: [git repo](https://github.com/Specy/trs-80), [npm package](https://www.npmjs.com/package/@specy/z80)

## Purpose
s68k is a teaching tool for M68K assembly. Its diagnostics explain invalid
syntax, addressing modes, and directives. It is not intended to produce
instruction binaries for real 68000 systems.

## Architecture

The library has three parts:

- **Assembler** (`src/assembler/`): reads the source files of a project, starting from the entry file, and returns diagnostics and, when there are no errors, a program. It contains a tokenizer, a parser, a symbol table, an expression evaluator, the layout pass, the instruction table, and the analyzer. Assembly reports all of the diagnostics it finds.

- **Program**: the assembler's output, a program containing assembled instructions with their addresses, sizes, and source locations; the initial memory; the symbols; and the entry point.

- **Interpreter** (`src/interpreter.rs`): runs a program using registers, memory, and the status register. It supports stepping, running to the end, undo history, breakpoints and a call stack.

The interpreter does not load encoded instructions into memory, so instructions
cannot be modified at runtime. Every instruction occupies four bytes regardless
of its size on a real 68000.

## Future work

- Real instruction encodings and programs loaded in memory
- Macros and conditional assembly
- A disassembler


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

## Supported directives
| Directive | What it does |
|---|---|
| `org` | sets the address the following lines are laid out at; the default origin is `$1000` |
| `equ` | names a value, once |
| `set` | names a value that may be set again; each use sees the latest definition above it |
| `dc` | puts values or a string in memory, `.b`, `.w` or `.l` |
| `dcb` | puts a value in memory a given number of times |
| `ds` | reserves room and writes nothing to it |
| `end` | ends the program and, with an operand, sets the entry point |
| `reg` | names a `movem` register list, `AllRegs reg d0-d7/a0-a6`, for `movem.l AllRegs,-(sp)` |
| `fail` | reports the rest of the line as an error of the program's own; the assembly carries on |
| `simhalt` | ends the run |
| `section` | switches between the sixteen location counters, 0 to 15, each going on from where it was left |
| `offset` | opens a region that produces no bytes, where `ds` names the fields of a structure by their offsets; `org *` ends it |
| `include` | assembles another file of the project here, as if its lines had been pasted at this line |
| `incbin` | puts another file's bytes in memory here, as a `dc.b` of the whole file would |
| `opt`, `list`, `nolist`, `page` | accepted and ignored: they are about the listing file, which there is none of |

Without `end`, the entry point is a label named `START`, and failing that the first instruction.

`equ`, `set` and `reg` need a label, and so does a `section` with no number, which it sets to the number of the section in force; `page` and the conditional directives take none. A `reg` list has to be defined above the `movem` that reads it, and it may not appear in an expression. 

## Projects of several files

A **project** is a map from a root-relative path to a file's text or to its bytes, plus the path of the **entry file** to assemble. Every other file is reached from the entry file through `include` or `incbin`, or is not read at all; nothing on a disk is ever opened by the assembler.

```ts
S68k.assemble({
    files: {'main.x68': source, 'lib/io.x68': library, 'data/sprite.bin': bytes},
    entry: 'main.x68'
})
```

`include` is **textual**: the included file's lines are assembled where the `include` line is, in the same section, at the same address, in one symbol namespace, with the local label scopes running across the boundary. The file name may be quoted with either quote or not quoted at all, the path is looked for beside the file that wrote it first and at the project root second. A file may be included more than once. `end` can only appear in the entry file.

`incbin` takes either kind of file: a binary one contributes its bytes and a text one its Latin-1 bytes. Adding a label on it names its first byte.

Every location carries the file it is in, so a diagnostic, a breakpoint (`{file, line}`), the current line and a call-stack frame all name one file of the project. A diagnostic raised in an included file carries the `include` lines it was reached through as related locations, and so does every assembled instruction (`includeChain`, innermost first): a file included twice has one location per line and two addresses, and the chain is the only thing that tells the two copies apart.

## Known limitations
1. Every instruction is four bytes wide whatever it encodes to on a real 68000, so an address computed from instruction sizes will not match the hardware. It is also why a PC-relative operand is resolved while the program is assembled: there is no extension word in memory to read the displacement from, so the assembler stores it and the interpreter adds it to the address of the instruction being executed.
2. The program runs as supervisor, always. The status register is a 16-bit register whose low byte is the condition codes and whose high byte is trace, supervisor and the interrupt mask, readable (`getSr()`, and the `SR:` line of the command line) and of no effect at all. It starts at `$2700`, so `andi #$00,sr` "puts the CPU in user mode" and changes nothing that runs. `move usp,an` and `move an,usp` are refused for the same reason: there is one stack pointer, `a7`.
4. `chk`, `trapv` and `illegal` raise their exception by ending the run. there are no exception vectors, no supervisor stack frame and no `rte`, so a program cannot handle one. The runtime error names the instruction and its cause.

## Running the command-line interpreter

Install [Rust](https://www.rust-lang.org/tools/install), clone the repository,
and run `cargo run` from its root. This assembles and runs `code-to-run.asm`.
To run another file, pass its path: `cargo run -- my-program.asm`.

## Building the WebAssembly package

Install [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/), then run
`npm run build-wasm` in the `ts-lib` directory. This creates the compiled
package in `ts-lib/src/pkg`.
