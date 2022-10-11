# UNDER DEVELOPMENT - s68k
A rust interpreter for m68k written to be used in my editor website with syntax checking.
I am writing this to learn rust, so expect bad code and bad practices.

## Purpose
The purpose of this interpreter is to help people learn the basics of assembly, in this case m68k, by providing useful errors and hints like a modern language, in the hope to help understand the addressing modes and learn the different instructions/directives.

It wasn't made to assemble or make actual programs, but marely as a learning tool

## Workings
The interpreter is split into individual modules that can be used standalone for different purposes
- Lexer: Has the job to identify the lines and the operands, it does no semantic or syntax checks and is left as generic as possible so that whatever piece of text can be fed to it and parsed

- Semantic checker: Has the job to verify that the lexed code is valid and reports useful errors so that the programmer can quickly identify and solve the problem. An example of this is the addressing modes, it will see if the addressing mode is not available, and hint which are. The semantic checker does not do further parsing

- pre interpreter: it will do a final processing of the code, like converting the immediates to actual numbers, registers to indexes, prepares the table of labels, etc... 

- interpreter: the final piece of the project and probably the most complex, whose job will be to actually execute the code

**WARNING** as this is only an interpreter, it does not load the actual program in memory so it won't be possible to modify instructions at runtime

## Might do
- Assembler
- Disassembler (unlikely)

## Supported instructions
| Type                   |  Instructions                                                                    |
|------------------------|----------------------------------------------------------------------------------|
| Arithmetic             | add, sub, suba, adda, divs, divu, muls, mulu                                     |
| Comparison             | tst, cmp                                                                         |
| Branching and jumping  | beq, bne, blt, ble, bgt, bge, blo, bls, bhi, bhs, bsr, bra, jsr, rts             |
| Accessing the SR       | scc, scs, seq, sne, sge, sgt, sle, sls, slt, shi, smi, spl, svc, svs, sf, st     |
| Bitwise                | not, or, and, eor, lsl, lsr, asr, asl, rol, ror, btst, bclr, bchg, bset          |
| Other                  | clr, exg, neg, ext, swap, move                                                   |
| Interrupt              | trap #15, with implemented interrupts from 0 to 7

## Supported directives
equ, org, dc, ds, dcb

## Todo
- Add more instructions
- Add more directives
- Add END directive
- Add tests
- Refactor the LabelDirective 

## Known bugs
1. Argument splitting is not the best, for example `move.l #',', d1` will split the arguments at the comma #','
2. comment splitting is not the best, for example `move.l #';', d1 ; this is a comment` will split the line at the #';'

# How to run
Firstly make sure you have rust installed, [you can download it here](https://www.rust-lang.org/tools/install), once done, clone the repository on your machine and run `cargo run` in the root folder of the project. This will run the interpreter with the code inside of `code-to-run.asm` file.

# How to build WASM binary
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the project by running `wasm-pack build --out-dir ts-lib/pkg` in the root folder of the project. This will create a `pkg` folder with the compiled code.

# How to test WASM binary
Inside of the `web` folder there is a very basic website with the library imported from the `pkg` folder, created after the wasm build. You can test the package by running `npm install` to install dependencies and then `npm run start` to start the server. The website will be available at `http://localhost:3000`