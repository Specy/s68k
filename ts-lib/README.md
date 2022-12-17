# s68k
A rust interpreter for m68k with semantic checker.
It compiles to WASM to be used with javascript and node.js, [available on npm](https://www.npmjs.com/package/s68k).

[Typescript library documentation](#docs)

## Purpose
The purpose of this interpreter is to help people learn the basics of assembly, in this case m68k, by providing useful errors and hints like a modern language, in the hope to help understand the addressing modes and learn the different instructions/directives.
**WARNING**
It wasn't made to assemble or make actual programs, but marely as a learning tool, don't expect 100% accuracy.

## Workings
The interpreter is split into individual modules that can be used standalone for different purposes
- Lexer: Has the job to identify the lines and the operands, it does no semantic or syntax checks and is left as generic as possible so that whatever piece of text can be fed to it and parsed

- Semantic checker: Has the job to verify that the lexed code is valid and reports useful errors so that the programmer can quickly identify and solve the problem. An example of this is the addressing modes, it will see if the addressing mode is not available, and hint which are. The semantic checker does not do further parsing

- Program compiler : it will do a final processing of the code, like converting the immediates to actual numbers, registers to indexes, prepares the table of labels, etc... 

- Interpreter: Fed the compiled program, it will execute the program, it also allows to step through it, in the future breakpoints will be added

**WARNING** as this is only an interpreter, it does not load the actual program in memory so it won't be possible to modify instructions at runtime, it is left to the developer to align the memory correctly as every instruction is 4bytes long and the the PC is incremented by 4 everytime.

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


## Known bugs
1. Not really a bug but a decision to make, characters are treated as UTF-8, so encoding and decoding might problematic for some front ends, alternative would be to allow only extended ASCII characters 0-255.

# How to run rust
Firstly make sure you have rust installed, [you can download it here](https://www.rust-lang.org/tools/install), once done, clone the repository on your machine and run `cargo run` in the root folder of the project. This will run the interpreter with the code inside of `code-to-run.asm` file.

# How to build WASM binary
The interpreter was made for WASM in mind, to build it you need [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) installed.
Once installed you can build the project by running `npm run build-wasm` in the `ts-lib` folder of the project. This will create a `pkg` folder in the ts-lib one with the compiled code.

# How to try the WASM binary locally
Inside of the `web` folder there is a very basic website with the library imported from the `pkg` folder **WARNING** not the ts-lib one, but in the root foler, to build it you need to run `wasm-pack build` in the root. You can test the package by running `npm install` to install dependencies and then `npm run start` to start the server. The website will be available at `http://localhost:3000`



# Docs

## Interfaces & Types
### CompilationResult
If the compilation was successful, it will return an [Interpreter](#interpreter), otherwise it will return a list of SemanticErrors
```typescript
type CompilationResult = 
    { ok: false, errors: SemanticError[] } | 
    { ok: true, interpreter: Interpreter }
```
### Interrupt handler
The callback that will be called whenever an interrupt is triggered, you will have to return the valid result for the interrupt to be handled correctly
```typescript
type InterruptResult = //this is the value that you will have to return, the type has to be the same as the current interrupt
    { type: "DisplayStringWithCRLF" } |
    { type: "DisplayStringWithoutCRLF" } |
    { type: "ReadKeyboardString", value: string } |
    { type: "DisplayNumber" } |
    { type: "ReadNumber", value: number } |
    { type: "ReadChar", value: string } |
    { type: "GetTime", value: number } |
    { type: "DisplayChar" } | 
    { type: "Terminate" }

type Interrupt = //this is the interrupt from the interpreter
    { type: "DisplayStringWithCRLF", value: string } |
    { type: "DisplayStringWithoutCRLF", value: string } |
    { type: "ReadKeyboardString" } |
    { type: "DisplayNumber", value: number } |
    { type: "ReadNumber" } |
    { type: "ReadChar" } |
    { type: "GetTime" } |
    { type: "Terminate" } | 
    { type: "DisplayChar", value: string }

//handler
type InterruptHandler = (interrupt: Interrupt) => Promise<InterruptResult> | void
```
### InterpreterOptions
```typescript
type InterpreterOptions = {
    keep_history: boolean //if true, the interpreter will keep a history of the executed instructions to execute the undo
    history_size: number   //the size of the history, if the history is full, the oldest change will be removed
}
```
### Parsing
Those are the types for the compilation result and execution.
```typescript
type ParsedLine = {
    line: string, //the original line
    line_index: number, //the index of the line in the file
    parsed: LexedLine //the parsed line
}

type RegisterOperand = //A single register
    { type: "Address", value: number } |
    {type: "Data", value: number}
type LexedLine = //The lexed instruction line
    {
        type: "Instruction"
        value: {
            name: string,
            operands: LexedOperand[],
            size: "Byte" | "Word" | "Long"
        }
    } | {
        type: "Label",
        value: {
            name: string
        }
    } | {
        type: "Directive",
        value: {
            args: string[]
        }
    } | {
        type: "Empty"
    } | {
        type: "Comment",
        value: {
            content: string
        }
    } | {
        type: "Unknown",
        value: {
            content: string
        }
    }

type LexedOperand = //The lexed operand
    {
        type: "Register",
        value: [type: LexedRegisterType, name: string]
    } | {
        type: "PreIndirect",
        value: LexedOperand
    } | {
        type: "Immediate"
        value: string
    } | {
        type: "PostIndirect",
        value: LexedOperand
    } | {
        type: "Absolute",
        value: string
    } | {
        type: "Label",
        value: string
    } | {
        type: "Other",
        value: string
    } | {
        type: "IndirectOrDisplacement",
        value: {
            offset: String,
            operand: LexedOperand
        }
    } | {
        type: "IndirectBaseDisplacement",
        value: {
            offset: String,
            operands: LexedOperand[]
        }
    }

enum Size { //Generic size used throughout the library
  Byte,
  Word,
  Long,
}

enum LexedRegisterType { //The type of the register
    LexedData = "Data",
    LexedAddress = "Address",
}

```


## S68k
This class is the main interface to the library, it has some utility methods to compile/lex/semantic check etc.

### Compilation
Given the m68k code and the size of the memory, it will return the CompilationResult, which, if successful, will contain the Interpreter. Optionally you can pass an InterpreterOptions object to configure the interpreter
```typescript
    static compile(code: string, memorySize: number, options?: InterpreterOptions): CompilationResult 
```
### Semantic check
It will execute lex and semantic checking to verify if there are any errors in the code, returns a list of SemanticErrors
```typescript
    static semanticCheck(code: string): SemanticError[]
```
### Lexing
It will execute the lexing on the given code, this does not check for semantic errors, it will return a list of tokens
```typescript
    static lex(code: string): ParsedLine[]
```
Alternatively you can lex a single line
```typescript
    static lexOne(line: string): ParsedLine
```

Instead of using the static methods, you can use the constructor to create a new S68k wrapper that will encapsulate the code and provide similar utility methods as the static ones.

## CompiledProgram
The WASM object that wraps the compiled code, it doesn't expose functionalities but is used to create the Interpreter

## Semantic Error
The error returned by the semantic check
```typescript
class SemanticError {
    getMessage(): string //the error message
    getLineIndex(): number //the index of the line in the file
    getMessageWithLine(): string //the formatted error message with the line too
    getLine(): ParsedLine //the parsed line which caused the error
    getError(): string //the error message
}
```
## Register
The register object, it has methods to get the value of the register in different sizes
```typescript
class Register {
    getLong(): number //32 bits value of the register
    getWord(): number //16 bits value of the register
    getByte(): number //8 bits value of the register
}
```

## Interpreter runtime classes

### InterpreterStatus
Current status of the interpreter, if running it can continue execution, if interrupted it will wait for an answer, if terminated, either normally or with exception, it will not continue execution and throw an exception if you try to step or run
```typescript
enum InterpreterStatus {
  Running,
  Interrupt,
  Terminated,
  TerminatedWithException,
}
```

### InstructionLine
The compiled instruction line, contains info about the original line too.
```typescript
type InstructionLine = {
    instruction: any //the compiled instruction
    address: number //the address of the instruction in memory
    parsed_line: ParsedLine //the original parsed line
}
```
### Step
The last executed instruction, plus the current status of the interpreter
```typescript
type Step = [instruction: InstructionLine, status: InterpreterStatus]
```

### MutationOperation
The mutation operation that was caused by the last instruction
```typescript
type MutationOperation = 
    {
        type: "WriteRegister",
        value: {
            register: RegisterOperand,
            old: number,
            size: Size
        }
    } | {
        type: "WriteMemory",
        value: {
            address: number,
            old: number,
            size: Size
        }
    } | {
        type: "WriteMemoryBytes",
        value: {
            address: number,
            old: number[]
        }
    }
```
### Condition
An enum representing the condition codes of the ccr
```typescript
enum Condition {
  True,
  False,
  High,
  LowOrSame,
  CarryClear,
  CarrySet,
  NotEqual,
  Equal,
  OverflowClear,
  OverflowSet,
  Plus,
  Minus,
  GreaterThanOrEqual,
  LessThan,
  GreaterThan,
  LessThanOrEqual,
}
```

## Cpu
A snapshot of the cpu status, it contains the [Registers](#register) and the ccr
```typescript
class Cpu {
    //returns a list of all the values of the registers
    //first 8 are the data registers, next 8 are the address registers
    getRegistersValues(): number[] 

    //given the number of the register and the type, returns the register object
    getRegister(register: number, type: RegisterType): Register 

    //given the number of the register and the type, returns the value of the register
    getRegisterValue(register: number, type: RegisterType): number
}
```

## Interpreter
The actual interpreter objecet that will execute the code, it holds the memory and [Cpu](#cpu) state, handles Interrupts.
```typescript
class Interpreter {
    //Use this to answer to the interrupt from the interpreter, 
    //it will not continue execution
    answerInterrupt(interruptResult: InterruptResult) 

    //Steps one instruction, returns the step object containing the 
    //last executed instruction and the current status
    step(): Step 

    //Runs the program until it is interrupted or terminated
    run(): InterpreterStatus

    //If the interpreter was set to allow for history, and there 
    //are operations to undo, it will undo the last instruction
    undo(): ExecutionStep 

    //Returns all the mutations that were caused by the last instruction
    getPreviousMutations(): MutationOperation[]

    //Given a condition, it tests whether the ccr flags satisfy it
    getConditionValue(condition: Condition): boolean 

    //Returns the current cpu snapshot
    getCpuSnapshot(): Cpu 

    //Returns the current interrupt, if present
    getCurrentInterrupt(): Interrupt | null 

    //Gets the program counter
    getPc(): number

    //Gets the stack pointer
    getSp(): number 

    //Returns an array of the ccr flags, where in order:
    //[X: Extend, N: Negative, Z: Zero, V: Overflow, C: Carry]
    getFlagsAsArray(): boolean[]

    //Returns the ccr flags as a bitfield number
    getFlagsAsBitfield(): number

    //Given a flag, it returns whether it is set or not
    getFlag(flag: Flags): boolean

    //Reads the memory at a certain address and length.
    //Will throw if the address is out of bounds
    readMemoryBytes(address: number, length: number): Uint8Array 

    //Returns the index of the last instruction that was executed
    getCurrentLineIndex(): number

    //Returns whether the interpreter can undo the last instruction
    canUndo(): boolean

    //Fetches the instruction at a certain address
    getInstructionAt(address: number): InstructionLine | null

    //Returns the current status of the interpreter
    getStatus(): InterpreterStatus

    //Given a register, and a size, it returns the value of the register
    getRegisterValue(register: RegisterOperand, size = Size.Long): number
    //Given a register, and a size, it sets the value of the register, this is not tracked as a mutation
    setRegisterValue(register: RegisterOperand, value: number, size = Size.Long)

    //Returns the next instruction to be executed
    getNextInstruction(): InstructionLine | null

    //Returns whether the interpreter has terminated execution, either normally or with exception
    hasTerminated(): boolean

    //Returns whether the interpreter has reached the end of the program
    hasReachedBottom(): boolean

    //Runs the program with an interrupt handler, it will call the handler when an interrupt
    //occurs, and will wait for the answer, once the code ends, the promise will resolve
    async runWithInterruptHandler(onInterrupt: InterruptHandler): Promise<InterpreterStatus>

    //Same as runWithInterruptHandler, but it will step one instruction at a time
    async stepWithInterruptHandler(onInterrupt: InterruptHandler): Promise<Step> 
}
```