import {
    Breakpoint,
    Condition,
    Cpu as RawCpu,
    Diagnostic,
    ExecutionStep,
    Flags,
    InstructionLine,
    Interpreter as RawInterpreter,
    InterpreterOptions,
    InterpreterStatus,
    Interrupt,
    InterruptResult,
    KeyStateRequest,
    KeyStateResult,
    LineSpan,
    Location,
    MutationOperation,
    ParsedComment,
    ParsedLabel,
    ParsedLine,
    ParsedLineKind,
    ParsedOperand,
    ParsedOperation,
    ParsedText,
    ProgramInfo,
    ProgramSymbol,
    Register as RawRegister,
    RegisterOperand,
    RelatedLocation,
    RuntimeError,
    Severity,
    Size,
    StackFrame,
    WasmAssembly as RawAssembly,
    wasm_assemble,
    wasm_parse_line
} from './pkg/s68k.js'

/** The path a bare source string is filed under, and the entry file's default. */
export const DEFAULT_ENTRY_PATH = 'main.m68k'

/**
 * The files of a project: a root-relative path with `/` separators, to the text
 * of a source file or to the bytes of a binary one.
 *
 * `include` reads a source file and `incbin` either kind, so the bytes of a
 * sprite or a table go in as a `Uint8Array` under the path the program names.
 */
export type SourceFiles = Record<string, string | Uint8Array>

/**
 * What to assemble: one source string, or the files of a project and the path
 * of the entry file to start from.
 *
 * Every other file is reached from the entry file through `include` or `incbin`
 * or is not read at all.
 */
export type AssemblySource = string | { files: SourceFiles, entry: string }

export type AssembleOptions = {
    /**
     * The path of the entry file. It names the file every diagnostic points at,
     * so pass the real name of the buffer when assembling one string. Defaults
     * to the project's own `entry`, or to `main.m68k` for a bare string.
     */
    entry?: string
}

/**
 * What the assembler made of a project: everything it found, and the program
 * when there is one.
 *
 * `program` is present exactly when no diagnostic is an error, so live checking
 * reads `diagnostics` and ignores the rest.
 */
export type AssemblyResult = {
    diagnostics: Diagnostic[]
    program?: Program
}

export enum RegisterType {
    Data,
    Address,
}

export class Register {
    private register: RawRegister

    constructor(register: RawRegister) {
        this.register = register
    }

    getLong() {
        return this.register.wasm_get_long()
    }

    getWord() {
        return this.register.wasm_get_word()
    }

    getByte() {
        return this.register.wasm_get_byte()
    }
}

export class Cpu {
    cpu: RawCpu

    constructor(cpu: RawCpu) {
        this.cpu = cpu
    }

    getRegistersValues(): number[] {
        const aReg = this.cpu.wasm_get_a_regs_value()
        const dReg = this.cpu.wasm_get_d_regs_value()
        return [...dReg, ...aReg]
    }

    getRegister(register: number, type: RegisterType): Register {
        if (type == RegisterType.Data) {
            return new Register(this.cpu.wasm_get_d_reg(register))
        } else {
            return new Register(this.cpu.wasm_get_a_reg(register))
        }
    }

    getRegisterValue(register: number, type: RegisterType): number {
        return this.getRegister(register, type).getLong()
    }

    /**
     * The whole status register as it was when the snapshot was taken; see
     * {@link Interpreter.getSr}.
     */
    getSr(): number {
        return this.cpu.wasm_get_sr()
    }
}

export type InterruptHandler = (interrupt: Interrupt) => Promise<InterruptResult> | void

/**
 * A program ready to run: the instructions with their addresses, the initial
 * contents of memory, the symbols and the entry point.
 *
 * It is a handle on the WebAssembly side and holds memory there, so call
 * {@link Program.dispose} when it is no longer needed. The same program can
 * build any number of interpreters, which is what restarting a run does.
 */
export class Program {
    private assembly: RawAssembly
    private info: ProgramInfo | null = null

    /** Wraps what {@link S68k.assemble} built; not meant to be called directly. */
    constructor(assembly: RawAssembly) {
        this.assembly = assembly
    }

    /** The handle the interpreter is built from. */
    getRaw(): RawAssembly {
        return this.assembly
    }

    /** Entry point, end address, instruction count and every symbol. */
    getInfo(): ProgramInfo {
        if (this.info === null) {
            this.info = this.assembly.wasm_get_program_info() as ProgramInfo
        }
        return this.info
    }

    /** The address the program starts running at. */
    getEntryPoint(): number {
        return this.getInfo().entryPoint
    }

    /** One past the last byte of the last instruction. */
    getEndAddress(): number {
        return this.getInfo().endAddress
    }

    getInstructionCount(): number {
        return this.getInfo().instructionCount
    }

    /** Every assembled instruction address, in ascending order. */
    getInstructionAddresses(): number[] {
        return this.assembly.wasm_get_instruction_addresses() as number[]
    }

    /** Every symbol of the program, by full name. */
    getSymbols(): Record<string, ProgramSymbol> {
        return this.getInfo().symbols
    }

    /** Give back the memory the program holds on the WebAssembly side. */
    dispose() {
        this.assembly.free()
    }
}

export class Interpreter {
    private interpreter: RawInterpreter

    /**
     * An interpreter over `program`, ready to run from its entry point.
     *
     * `options` defaults to a history of 100 steps, which is what undo needs;
     * pass `{ keep_history: false, history_size: 0 }` to run without one.
     */
    constructor(program: Program, options: InterpreterOptions = {keep_history: true, history_size: 100}) {
        this.interpreter = new RawInterpreter(program.getRaw(), options)
    }

    answerInterrupt(interruptResult: InterruptResult) {
        this.interpreter.wasm_answer_interrupt(interruptResult)
    }

    /**
     * Run one instruction and answer the resulting status. If `SIMHALT`
     * paused the program, this resumes at the following instruction.
     */
    step(): InterpreterStatus {
        return this.interpreter.wasm_step()
    }

    /** @deprecated the same call as {@link Interpreter.step}, which answers the status too. */
    stepGetStatus(): InterpreterStatus {
        return this.interpreter.wasm_step()
    }

    writeMemoryBytes(address: number, data: Uint8Array) {
        return this.interpreter.wasm_write_memory_bytes(address, data)
    }

    /** The instruction that has just run, or null before the first step. */
    getLastInstruction(): InstructionLine | null {
        return this.interpreter.wasm_get_last_instruction() as InstructionLine | null
    }

    undo(): ExecutionStep {
        return internalExecutionStepToExecutionStep(this.interpreter.wasm_undo())
    }

    getPreviousMutations(): MutationOperation[] | null {
        return this.interpreter.wasm_get_previous_mutations() as MutationOperation[] | null
    }

    /** Identity of the newest retained instruction, or 0 before execution. */
    getLastStepId(): number {
        return this.interpreter.wasm_get_last_step_id()
    }

    async stepWithInterruptHandler(onInterrupt: InterruptHandler): Promise<InterpreterStatus> {
        const status = this.interpreter.wasm_step() as InterpreterStatus
        if (status == InterpreterStatus.Interrupt) {
            let result = await onInterrupt(this.getCurrentInterrupt()!)
            if (result) this.answerInterrupt(result)
        }
        return status
    }

    getConditionValue(condition: Condition): boolean {
        return this.interpreter.wasm_get_condition_value(condition)
    }

    getCpuSnapshot(): Cpu {
        return new Cpu(this.interpreter.wasm_get_cpu_snapshot())
    }

    getCurrentInterrupt(): Interrupt | null {
        return this.interpreter.wasm_get_current_interrupt()
    }

    getPc(): number {
        return this.interpreter.wasm_get_pc()
    }

    getSp(): number {
        return this.interpreter.wasm_get_sp()
    }

    getFlagsAsArray(): boolean[] {
        return [...this.interpreter.wasm_get_flags_as_array()].map(v => v == 1)
    }

    getFlagsAsBitfield(): number {
        return this.interpreter.wasm_get_flags_as_number()
    }

    /**
     * The whole status register, `0x2700` before the program has run.
     *
     * Its high byte — trace, supervisor and the interrupt mask — is stored and
     * readable and has no effect: s68k runs every program as supervisor, as
     * EASy68K's simulator does. Its low byte is the condition codes as the
     * processor numbers them (extend 16, negative 8, zero 4, overflow 2,
     * carry 1), which is not the bitfield {@link Interpreter.getFlagsAsBitfield}
     * answers.
     */
    getSr(): number {
        return this.interpreter.wasm_get_sr()
    }

    readMemoryBytes(address: number, length: number): Uint8Array {
        return this.interpreter.wasm_read_memory_bytes(address, length)
    }

    getFlag(flag: Flags): boolean {
        return this.interpreter.wasm_get_flag(flag)
    }

    /**
     * Where the instruction the program counter is on was written, or null when
     * it is on none. Replaces 1.4.2's `getCurrentLineIndex`, which could only
     * ever answer a line of one file.
     */
    getCurrentLocation(): Location | null {
        return this.interpreter.wasm_get_current_location() as Location | null
    }

    /** The address of the instruction being executed, or 0 before the first step. */
    getCurrentInstructionAddress(): number {
        return this.interpreter.wasm_get_last_line_address()
    }

    canUndo(): boolean {
        return this.interpreter.wasm_can_undo()
    }

    getCallStack(): StackFrame[] {
        return this.interpreter.wasm_get_call_stack() as StackFrame[]
    }

    getUndoHistory(amount: number): ExecutionStep[] {
        return this.interpreter.wasm_get_undo_history(amount).map(internalExecutionStepToExecutionStep)
    }

    getInstructionAt(address: number): InstructionLine | null {
        return this.interpreter.wasm_get_instruction_at(address) as InstructionLine | null
    }

    getStatus(): InterpreterStatus {
        return this.interpreter.wasm_get_status()
    }

    getRegisterValue(register: RegisterOperand, size = Size.Long) {
        return this.interpreter.wasm_get_register_value(register, size)
    }

    setRegisterValue(register: RegisterOperand, value: number, size = Size.Long) {
        this.interpreter.wasm_set_register_value(register, value, size)
    }

    getNextInstruction(): InstructionLine | null {
        return this.interpreter.wasm_get_next_instruction() as InstructionLine | null
    }

    hasTerminated(): boolean {
        return this.interpreter.wasm_has_terminated()
    }

    hasReachedBottom(): boolean {
        return this.interpreter.wasm_has_reached_bottom()
    }

    /** Run until the program pauses, requests an interrupt, or terminates. */
    run(): InterpreterStatus {
        return this.interpreter.wasm_run()
    }

    /** Like {@link Interpreter.run}, but execute at most `limit` instructions. */
    runWithLimit(limit: number): InterpreterStatus {
        return this.interpreter.wasm_run_with_limit(limit)
    }

    /**
     * Run until one of `breakpoints` is reached, the program pauses, requests
     * an interrupt or ends, or `limit` instructions have run.
     *
     * A breakpoint is a line of a file: a breakpoint on a comment, a directive
     * or a label alone stops nothing, and neither does one on a line of a file
     * this program was not assembled from.
     */
    runWithBreakpoints(breakpoints: Breakpoint[], limit?: number): InterpreterStatus {
        return this.interpreter.wasm_run_with_breakpoints(breakpoints, limit)
    }

    async runWithInterruptHandler(onInterrupt: InterruptHandler): Promise<InterpreterStatus> {
        const status = this.interpreter.wasm_run() as InterpreterStatus
        if (status == InterpreterStatus.Interrupt) {
            let result = await onInterrupt(this.getCurrentInterrupt()!)
            if (result) this.answerInterrupt(result)
        }
        return status
    }

    /** Give back the memory the interpreter holds on the WebAssembly side. */
    dispose() {
        this.interpreter.free()
    }
}

/**
 * The assembler: the front end that turns source files into a program and
 * diagnostics.
 *
 * Both entry points are static; there is nothing to construct.
 */
export class S68k {
    private constructor() {
    }

    /**
     * Assemble a project and answer everything found, with the program when the
     * source builds one.
     *
     * ```ts
     * const {diagnostics, program} = S68k.assemble('    move.w #1,d0')
     * const withFiles = S68k.assemble({
     *     files: {'main.x68': source, 'lib/io.x68': library, 'data/sprite.bin': bytes},
     *     entry: 'main.x68'
     * })
     * ```
     *
     * A project is assembled from its entry file down: `include` assembles
     * another file of the project where the line is, `incbin` puts a file's
     * bytes in memory, and a file the project has not got is a diagnostic
     * naming the closest one it has. Nothing here reads a disk — the files are
     * the whole of what the assembler can see.
     */
    static assemble(source: AssemblySource, options: AssembleOptions = {}): AssemblyResult {
        const isText = typeof source === 'string'
        const entry = options.entry ?? (isText ? DEFAULT_ENTRY_PATH : source.entry)
        const files: SourceFiles = isText ? {[entry]: source} : source.files
        const assembly = wasm_assemble(files, entry)
        const diagnostics = assembly.wasm_get_diagnostics() as Diagnostic[]
        if (!assembly.wasm_has_program()) {
            // Nothing to run and nothing to hold on to: free the handle here so
            // that live checking, which calls this on every keystroke, leaks
            // nothing.
            assembly.free()
            return {diagnostics}
        }
        return {diagnostics, program: new Program(assembly)}
    }

    /**
     * Read one source line into its four fields, for hover and highlighting.
     *
     * It never throws and never reports anything: a line out of its file cannot
     * know a symbol, an address or an instruction's operand rules, so what comes
     * back is what was written, each part with the columns it covers. Replaces
     * 1.4.2's `lexOne`.
     */
    static parseLine(text: string): ParsedLine {
        return wasm_parse_line(text) as ParsedLine
    }
}

export enum Flag {
    Carry = 1 << 1,
    Overflow = 1 << 2,
    Zero = 1 << 3,
    Negative = 1 << 4,
    Extend = 1 << 5
}

export function ccrToFlags(ccr: number) {
    return {
        carry: ccr & Flag.Carry,
        overflow: ccr & Flag.Overflow,
        zero: ccr & Flag.Zero,
        negative: ccr & Flag.Negative,
        extend: ccr & Flag.Extend
    }
}

export function ccrToFlagsArray(ccr: number) {
    return [
        ccr & Flag.Carry,
        ccr & Flag.Overflow,
        ccr & Flag.Zero,
        ccr & Flag.Negative,
        ccr & Flag.Extend
    ].map(v => Number(Boolean(v)))
}


export type ExecutionStepInternal = {
    id: number,
    mutations: MutationOperation[],
    pc: number,
    old_ccr: string,
    new_ccr: string
    /** The whole status register before the step; a number, not a bitfield name. */
    old_sr: number,
    /** The whole status register after it. */
    new_sr: number,
    location?: Location
}


function internalExecutionStepToExecutionStep(step: ExecutionStepInternal): ExecutionStep {
    return {
        ...step,
        old_ccr: {
            bits: stringCCRBitfieldToNumber(step.old_ccr as string)
        },
        new_ccr: {
            bits: stringCCRBitfieldToNumber(step.new_ccr as string)
        }

    }
}


const ccrFlags = Object.keys(Flag)
function stringCCRBitfieldToNumber(bitfield: string): number {
    const fields = bitfield.split('|').map(v => v.trim()) // ['Carry', 'Overflow', 'Zero', 'Negative', 'Extend'] etc... the selected flags will be present
    let ccr = 0
    for (let field of fields) {
        ccr |= Flag[field as keyof typeof Flag]
    }
    return ccr
}

export {
    RawAssembly,
    RawInterpreter,
    RawCpu,
    RawRegister,
    Breakpoint,
    Condition,
    Diagnostic,
    ExecutionStep,
    Flags,
    InstructionLine,
    InterpreterOptions,
    InterpreterStatus,
    Interrupt,
    InterruptResult,
    KeyStateRequest,
    KeyStateResult,
    LineSpan,
    Location,
    MutationOperation,
    ParsedComment,
    ParsedLabel,
    ParsedLine,
    ParsedLineKind,
    ParsedOperand,
    ParsedOperation,
    ParsedText,
    ProgramInfo,
    ProgramSymbol,
    RegisterOperand,
    RelatedLocation,
    RuntimeError,
    Severity,
    Size,
    StackFrame,
}
