import {
    Access,
    AutoIrq,
    Breakpoint,
    CodeWrite,
    Condition,
    Cpu as RawCpu,
    DeviceEvent,
    Diagnostic,
    DisassembledLine,
    ExecutionStep,
    ExecutionStepKind,
    FileDialogMode,
    FileExistence,
    Flags,
    InputSettings,
    InstructionForm,
    InstructionInfo,
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
    MachineState,
    MemoryRange,
    MemoryRangeKind,
    MutationOperation,
    OpenedFile,
    ParsedComment,
    ParsedLabel,
    ParsedLine,
    ParsedLineKind,
    ParsedOperand,
    ParsedOperation,
    ParsedText,
    PokeWrite,
    Processor,
    ProgramInfo,
    ProgramSymbol,
    Register as RawRegister,
    RegisterOperand,
    RelatedLocation,
    RuntimeError,
    Severity,
    Size,
    SoundOutcome,
    StackFrame,
    SuggestedFix,
    SystemRegisters,
    Termination,
    WasmAssembly as RawAssembly,
    wasm_assemble,
    wasm_get_instruction_table,
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

    /** Throws an `InvalidArgument` for a register number past 7. */
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

    /** The user stack pointer when the snapshot was taken: A7 in user mode. */
    getUsp(): number {
        return this.cpu.wasm_get_usp()
    }

    /** The supervisor stack pointer when the snapshot was taken: A7 in supervisor mode. */
    getSsp(): number {
        return this.cpu.wasm_get_ssp()
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

    /**
     * Entry point, end address, instruction count, every symbol, and the
     * processor the program is written for.
     */
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

    /**
     * One past the last byte of the last instruction: where the code ends. A run
     * goes on past it, into whatever follows, and ends where it meets
     * `$FFFF $FFFF` the program never placed.
     */
    getEndAddress(): number {
        return this.getInfo().endAddress
    }

    getInstructionCount(): number {
        return this.getInfo().instructionCount
    }

    /**
     * The processor the program is written for: the one its `opt p=` line
     * chose, `'68000'` when it chose none. Its code decodes for this processor,
     * and an instruction of a later one is refused when it is assembled.
     */
    getProcessor(): Processor {
        return this.getInfo().processor
    }

    /** Whether the program has the 68020's bit-field instructions, through `opt bit` or `opt p=68020`. */
    hasBitFields(): boolean {
        return this.getInfo().bitFields
    }

    /**
     * Whether {@link Interpreter.getCycles} counts the processor's exact time,
     * as for the 68000 and the 68010, or an estimate, as for the 68020: the
     * cache case of the M68020 User's Manual's tables, which say that the real
     * time depends on the instruction cache and on how neighbouring
     * instructions overlap. A host shows the 68020's count as an estimate, and
     * that of a program for the 68000 or the 68010 under `opt bit` that has a
     * bit-field instruction, whose bit fields take EASy68K's time, the
     * 68020's cache case.
     */
    hasExactCycles(): boolean {
        return this.getInfo().cyclesExact
    }

    getLayoutItems(): Uint32Array {
        return this.assembly.wasm_get_layout_items()
    }

    /** Every assembled instruction address, in ascending order. */
    getInstructionAddresses(): number[] {
        return this.assembly.wasm_get_instruction_addresses() as number[]
    }

    /** Every symbol of the program, by full name. */
    getSymbols(): Record<string, ProgramSymbol> {
        return this.getInfo().symbols
    }

    /**
     * The memory map the program's `memory` lines declare: at most one range of
     * each kind, in EASy68K's order, `rom`, `read`, `protected` and `invalid`.
     * Empty when the program declares none. The interpreter enforces it: a
     * write to ROM is ignored, and the other kinds answer a `BusError`,
     * vector 2; a poke and the host's reads go past it.
     */
    getMemoryMap(): MemoryRange[] {
        return this.assembly.wasm_get_memory_map() as MemoryRange[]
    }

    /**
     * The program as the S-record file EASy68K's assembler writes, a `.S68`
     * that EASy68K's simulator loads and runs: the `68KPROG` header, the code
     * and data in S1, S2 or S3 records of up to 32 bytes, the memory map as
     * EASy68K's extra S0 records, and the entry point in an S8 record. Each
     * record ends with CR LF.
     */
    toSRecord(): string {
        return this.assembly.wasm_to_srecord() as string
    }

    /** Give back the memory the program holds on the WebAssembly side. */
    dispose() {
        this.assembly.free()
    }
}

/**
 * One store into the range {@link Interpreter.watchMemory} watches, as
 * {@link Interpreter.takeWatchedWrites} answers it.
 */
export type WatchedWrite = {
    /**
     * The address of the first byte stored, in the 24 bits the 68000 puts on
     * its bus. A store that straddles the start of the range begins before it.
     */
    address: number,
    /** How many bytes were stored: 1, 2 or 4. A run of bytes is recorded a byte at a time. */
    size: 1 | 2 | 4,
    /** The value stored, `size` bytes of it, most significant byte first as memory holds it. */
    value: number
}

/**
 * What {@link Interpreter.runWithBreakpoints} does with the breakpoint the
 * program counter is on, and whether it stops on a write into code.
 */
export type RunWithBreakpointsOptions = {
    /**
     * `true` (the default) runs the instruction the program counter is on even
     * when a breakpoint names it, which is what "continue" from a breakpoint
     * needs; `false` stops before it, having run nothing.
     */
    skipBreakpointAtPc?: boolean
    /**
     * `true` stops the run after an instruction that wrote into code, with
     * `InterpreterStatus.CodeWrite`: {@link Interpreter.getCodeWrite} says what
     * it overwrote, and the next run goes on, the overwritten instruction
     * running as its new bytes decode. `false` (the default) records the
     * write and runs on.
     */
    stopOnCodeWrite?: boolean
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

    /**
     * Answer the pending interrupt, which finishes the instruction that raised
     * it.
     *
     * Every interrupt is answered by the `InterruptResult` of its own `type`. A
     * display task has nothing to answer but that `type`: the text it carries is
     * ready to display, decoded from Windows-1252 and formatted the way EASy68K
     * formats it. A read task is answered with what was typed, as it was typed —
     * the line for `ReadKeyboardString`, `ReadNumber` and
     * `DisplayStringAndReadNumber`, the key for `ReadChar` — and the interpreter
     * stores and reads it with EASy68K's rules. A file task is answered with
     * what the host's file system did, and the interpreter writes EASy68K's
     * result code for it. `{type: 'Terminate'}` answers any task and ends the
     * program.
     *
     * Throws a `RuntimeError` object, and changes nothing, when no interrupt is
     * pending (`NoPendingInterrupt`), when the answer is not the pending
     * interrupt's or not an answer at all (`InvalidAnswer`), and when what it
     * writes does not fit in memory (`OutOfBounds`); the interrupt then still
     * waits.
     */
    answerInterrupt(interruptResult: InterruptResult) {
        this.interpreter.wasm_answer_interrupt(interruptResult)
    }

    /**
     * Why the program ended, or null while it has not: task 9, running into
     * memory it never placed (`EndOfProgram`, with the address), a `Terminate`
     * answer, the runtime error it ended with (`Exception`, with the vector
     * that held no handler), or a double fault (`Halted`). Undoing the step that
     * ended it takes the end back.
     */
    getTermination(): Termination | null {
        return this.interpreter.wasm_get_termination() as Termination | null
    }

    /**
     * How the read tasks show what is typed: the echo of task 12, the input
     * prompt and the line feed after Enter of task 16, all on when a program
     * starts. A host reads them when a read task waits.
     */
    getInputSettings(): InputSettings {
        return this.interpreter.wasm_get_input_settings() as InputSettings
    }

    /**
     * Run one instruction and answer the resulting status. If `SIMHALT`
     * paused the program, or the last instruction stopped on a watched write,
     * this resumes at the following instruction.
     *
     * An IRQ ready to be taken, a pending level above the interrupt mask or at
     * level 7, is the step instead: a step of `kind: 'irq'` that pushes the
     * frame and jumps to the autovector's handler. While the processor is
     * `Stopped`, waiting, a step runs nothing and answers `Stopped`; a request
     * that can wake it makes the status `Running`, and the next step takes it.
     */
    step(): InterpreterStatus {
        return this.interpreter.wasm_step()
    }

    /** @deprecated the same call as {@link Interpreter.step}, which answers the status too. */
    stepGetStatus(): InterpreterStatus {
        return this.interpreter.wasm_step()
    }

    /**
     * Write bytes into memory.
     *
     * Inside a poke ({@link Interpreter.beginPoke}) the bytes it overwrites are
     * journaled, so undo puts them back; outside one it is a direct write that
     * records nothing, which is what a testcase's preset memory needs. Either
     * way, the bytes that land in the watched range are watched writes
     * ({@link Interpreter.watchMemory}), and stop nothing. Code takes the bytes
     * like data, and runs as they decode; a host's write is never a code write.
     */
    writeMemoryBytes(address: number, data: Uint8Array) {
        return this.interpreter.wasm_write_memory_bytes(address, data)
    }

    /**
     * Open a poke: everything {@link Interpreter.setRegisterValue} and
     * {@link Interpreter.writeMemoryBytes} write until {@link Interpreter.endPoke}
     * becomes one step of the same history the instructions use, undone by one
     * {@link Interpreter.undo} like any other.
     *
     * Throws when a poke is already open and when an instruction is executing,
     * which includes an interrupt waiting for its answer.
     */
    beginPoke(): void {
        this.interpreter.wasm_begin_poke()
    }

    /**
     * Close the open poke and answer whether it recorded a step.
     *
     * A poke that wrote nothing, or only values that were already there,
     * records none and answers false; so does one made by an interpreter that
     * keeps no history. Throws when no poke is open.
     */
    endPoke(): boolean {
        return this.interpreter.wasm_end_poke()
    }

    /**
     * The line the program laid out where the instruction that has just run
     * is, or null before the first step and when it ran on none.
     */
    getLastInstruction(): InstructionLine | null {
        return this.interpreter.wasm_get_last_instruction() as InstructionLine | null
    }

    /**
     * Revert the newest step of the history, instruction or poke alike, and
     * answer it. Undoing a poke puts back every value it wrote and touches
     * nothing else. Undo empties the log of watched writes: read the watched
     * range again instead. An instruction the step overwrote runs as its old
     * bytes again, and the step's code write goes with it.
     */
    undo(): ExecutionStep {
        return internalExecutionStepToExecutionStep(this.interpreter.wasm_undo())
    }

    getPreviousMutations(): MutationOperation[] | null {
        return this.interpreter.wasm_get_previous_mutations() as MutationOperation[] | null
    }

    /**
     * Identity of the newest retained step, instruction or poke, or 0 before
     * execution. A poke has an id of its own, so this moves past it.
     */
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

    /**
     * Watch the addresses from `start` up to `end`, `end` not included: every
     * store into them is recorded from now on, and the instruction that makes
     * one ends the run or step that ran it with `InterpreterStatus.WatchedWrite`,
     * so the host sees what was written before the next instruction runs.
     *
     * A store counts when any of its bytes is in the range: each register of a
     * `movem`, the store of an `addq` or `bset` to memory, a `bsr`, `jsr` or
     * `pea` push, a read task's answer and {@link Interpreter.writeMemoryBytes}
     * alike. One range at a time; watching replaces the range and forgets the
     * writes not taken. Throws an `InvalidArgument` for an empty range or one
     * past the 16 MB of memory.
     */
    watchMemory(start: number, end: number): void {
        this.interpreter.wasm_watch_memory(start, end)
    }

    /** Stop watching, and forget the writes not taken. */
    unwatchMemory(): void {
        this.interpreter.wasm_unwatch_memory()
    }

    /**
     * The stores into the watched range since the last call, in the order they
     * were made, and the log emptied. Take them after every `WatchedWrite`
     * stop, and after writing into the range yourself.
     */
    takeWatchedWrites(): WatchedWrite[] {
        const flat = this.interpreter.wasm_take_watched_writes()
        const writes: WatchedWrite[] = []
        for (let i = 0; i < flat.length; i += 3) {
            writes.push({address: flat[i], size: flat[i + 1] as WatchedWrite['size'], value: flat[i + 2]})
        }
        return writes
    }

    getConditionValue(condition: Condition): boolean {
        return this.interpreter.wasm_get_condition_value(condition)
    }

    getCpuSnapshot(): Cpu {
        return new Cpu(this.interpreter.wasm_get_cpu_snapshot())
    }

    /** The interrupt waiting for its answer, or null. A file write's bytes are a `Uint8Array`. */
    getCurrentInterrupt(): Interrupt | null {
        return this.interpreter.wasm_get_current_interrupt() as Interrupt | null
    }

    getPc(): number {
        return this.interpreter.wasm_get_pc()
    }

    /**
     * The top of the stack A7 is on, which the Stack region runs up to from
     * A7: the highest A7 has been, or where it last jumped to. The USP and the
     * SSP each keep their own, swapped with them whenever S changes.
     */
    getStackTop(): number {
        return this.interpreter.wasm_get_stack_top()
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
     * The whole status register, `0x2000` before the program has run, as in
     * EASy68K: supervisor mode, interrupt mask 0.
     *
     * Its high byte is trace, supervisor and the interrupt mask: T traces every
     * instruction, and S chooses the mode and which stack pointer A7 is. Its low
     * byte is the condition codes as the processor numbers them (extend 16,
     * negative 8, zero 4, overflow 2, carry 1), which is not the bitfield
     * {@link Interpreter.getFlagsAsBitfield} answers.
     */
    getSr(): number {
        return this.interpreter.wasm_get_sr()
    }

    /**
     * The System register file: the status register and the stack pointers,
     * in a program for the 68010 or the 68020 the control registers VBR, SFC
     * and DFC, which a 68000 program's has no field for, and in one for the
     * 68020 its interrupt and master stack pointers ISP and MSP and its cache
     * registers CACR and CAAR. A7 is the USP while S is clear and the SSP
     * while it is set; the 68020's SSP is its ISP, `isp` naming the same
     * register, and A7 is the MSP instead while M is set. A program starts
     * with SR `0x2000`, the SSP at `0x01000000`, the USP at `0x00FF0000`, and
     * the MSP and the control registers 0.
     */
    getSystem(): SystemRegisters {
        return this.interpreter.wasm_get_system() as SystemRegisters
    }

    /**
     * Write the status register, as `move #n,sr` would: reduced to the bits
     * the processor has (`0xA71F`, and on a 68020, which adds T0 and M,
     * `0xF71F`), and a change of S, or on a 68020 of M, makes A7 another stack
     * pointer. Inside a poke ({@link Interpreter.beginPoke}) it is journaled, so
     * one undo puts it back with the stacks; outside one it is direct and
     * records nothing.
     */
    setSr(value: number) {
        this.interpreter.wasm_set_sr(value)
    }

    /**
     * Write the user stack pointer, which is A7 in user mode. Inside a poke it is
     * journaled; outside one it is direct.
     */
    setUsp(value: number) {
        this.interpreter.wasm_set_usp(value)
    }

    /**
     * Write the supervisor stack pointer, which is A7 in supervisor mode: the
     * 68020's interrupt stack pointer, A7 in supervisor mode with M clear.
     * Inside a poke it is journaled; outside one it is direct.
     */
    setSsp(value: number) {
        this.interpreter.wasm_set_ssp(value)
    }

    /**
     * Write the vector base register, where the vector table starts, which
     * every exception reads its vector through: the 68010's, as `movec d0,vbr`
     * would. Inside a poke it is journaled; outside one it is direct. It throws
     * an `InvalidArgument` in a program for the 68000, which has none.
     */
    setVbr(value: number) {
        this.interpreter.wasm_set_vbr(value)
    }

    /**
     * Write the source function code, its three low bits, as `movec d0,sfc`
     * would: the 68010's. Inside a poke it is journaled; outside one it is
     * direct. It throws an `InvalidArgument` in a program for the 68000.
     */
    setSfc(value: number) {
        this.interpreter.wasm_set_sfc(value)
    }

    /**
     * Write the destination function code, its three low bits, as
     * `movec d0,dfc` would: the 68010's. Inside a poke it is journaled;
     * outside one it is direct. It throws an `InvalidArgument` in a program for
     * the 68000.
     */
    setDfc(value: number) {
        this.interpreter.wasm_set_dfc(value)
    }

    /**
     * Write the 68020's interrupt stack pointer, the SSP under its 68020 name,
     * as `movec d0,isp` would. Inside a poke it is journaled; outside one it
     * is direct. It throws an `InvalidArgument` in a program for an earlier
     * processor.
     */
    setIsp(value: number) {
        this.interpreter.wasm_set_isp(value)
    }

    /**
     * Write the 68020's master stack pointer, A7 in supervisor mode with M
     * set, as `movec d0,msp` would. Inside a poke it is journaled; outside one
     * it is direct. It throws an `InvalidArgument` in a program for an earlier
     * processor.
     */
    setMsp(value: number) {
        this.interpreter.wasm_set_msp(value)
    }

    /**
     * Write the 68020's cache control register as `movec d0,cacr` would: its E
     * (bit 0) and F (bit 1) are kept, and the other bits read as 0, as s68k
     * has no cache. Inside a poke it is journaled; outside one it is direct.
     * It throws an `InvalidArgument` in a program for an earlier processor.
     */
    setCacr(value: number) {
        this.interpreter.wasm_set_cacr(value)
    }

    /**
     * Write the 68020's cache address register as `movec d0,caar` would.
     * Inside a poke it is journaled; outside one it is direct. It throws an
     * `InvalidArgument` in a program for an earlier processor.
     */
    setCaar(value: number) {
        this.interpreter.wasm_set_caar(value)
    }

    /**
     * The interrupt state: the IRQ levels pending, whether `stop` has stopped the
     * processor, the timers task 32 set (which the host runs on the program's
     * time, calling {@link Interpreter.raiseIrq} at each firing), and the levels
     * tasks 62 and 60 set for the keyboard's and the mouse's events. Read it
     * after every call that ran the program: a step can change any of it, and
     * undo puts it back.
     */
    getMachineState(): MachineState {
        return this.interpreter.wasm_get_machine_state() as MachineState
    }

    /**
     * Latch an interrupt request at `level`, 1 to 7: what a timer of the
     * program's does when it fires. It is taken before the next instruction
     * when it is above the interrupt mask or is level 7, and waits until then.
     * One that can wake a processor `stop` stopped makes the status `Running`
     * at once, `getMachineState().stopped` staying true until the next step
     * takes it. A
     * request is the host's input, journaled nowhere: undo makes a request a
     * step took pending again, and leaves one the program has not seen pending.
     * It does nothing after the program has ended. Throws an `InvalidArgument`
     * for a level that is not 1 to 7.
     */
    raiseIrq(level: number): void {
        this.interpreter.wasm_raise_irq(level)
    }

    /**
     * Report a key or mouse event on the Screen, and latch the interrupt
     * request the program asked for on it with task 62 or 60, as
     * {@link Interpreter.raiseIrq} does. Answers the level it latched, or null
     * when the program asked for none on that event. The event's own data, the
     * key and the mouse's state, is still read by the program's tasks. Throws an
     * `InvalidArgument` for a kind that is not a `DeviceEvent`.
     */
    deviceEvent(kind: DeviceEvent): number | null {
        return this.interpreter.wasm_device_event(kind) ?? null
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

    /**
     * The frames entered and not returned from, oldest first: each `bsr` or
     * `jsr` (`kind: 'call'`, which `rts` returns from) and each exception taken
     * (`kind: 'exception'` with its `vector`, which `rte` returns from).
     */
    getCallStack(): StackFrame[] {
        return this.interpreter.wasm_get_call_stack() as StackFrame[]
    }

    /**
     * The newest `amount` steps of the history, newest first. A poke is one of
     * them, in its place among the instructions, told apart by its `kind`.
     */
    getUndoHistory(amount: number): ExecutionStep[] {
        return this.interpreter.wasm_get_undo_history(amount).map(internalExecutionStepToExecutionStep)
    }

    /**
     * The line the program laid out at `address`, as it was assembled: its
     * bytes are what the build placed, and `assembledAs` says what it assembled
     * as when that differs from what the line wrote. A line whose bytes the
     * program or a poke changed keeps its location and is still answered as
     * assembled; {@link Interpreter.disassembleAt} reads what runs there now.
     * Null where no line was laid out.
     */
    getInstructionAt(address: number): InstructionLine | null {
        return this.interpreter.wasm_get_instruction_at(address) as InstructionLine | null
    }

    /**
     * The instruction memory holds at `address` now, decoded for the program's
     * processor and printed in EASy68K's syntax with the program's labels: what
     * runs there, whatever the build laid out. Words that are no instruction of
     * that processor print as `DC.W`, so `$42C0`, `move ccr,d0`, is one in a
     * 68000 program.
     */
    disassembleAt(address: number): DisassembledLine {
        return this.interpreter.wasm_disassemble_at(address) as DisassembledLine
    }

    /**
     * The code write of the instruction that has just run, or of the one whose
     * interrupt was just answered: the first instruction its write overwrote,
     * or null. Null before the first step and after an undo. Each step of the
     * history carries its own as `wroteCode`.
     */
    getCodeWrite(): CodeWrite | null {
        return this.interpreter.wasm_get_code_write() as CodeWrite | null
    }

    getStatus(): InterpreterStatus {
        return this.interpreter.wasm_get_status()
    }

    /** Throws an `InvalidArgument` for a register that is not `{type: 'Data' | 'Address', value: 0 to 7}`. */
    getRegisterValue(register: RegisterOperand, size = Size.Long) {
        return this.interpreter.wasm_get_register_value(register, size)
    }

    /**
     * Write a register.
     *
     * Inside a poke ({@link Interpreter.beginPoke}) the write is journaled into
     * it; outside one it is direct and records nothing.
     */
    setRegisterValue(register: RegisterOperand, value: number, size = Size.Long) {
        this.interpreter.wasm_set_register_value(register, value, size)
    }

    /** The line the program laid out where the program counter is, as {@link Interpreter.getInstructionAt} reads it. */
    getNextInstruction(): InstructionLine | null {
        return this.interpreter.wasm_get_next_instruction() as InstructionLine | null
    }

    hasTerminated(): boolean {
        return this.interpreter.wasm_has_terminated()
    }

    /**
     * Whether the next step ends the program: the program counter is on
     * `$FFFF $FFFF` none of whose bytes the program placed, which is where a
     * program that does not end itself gets to, and no IRQ is ready to be
     * taken first.
     */
    hasReachedBottom(): boolean {
        return this.interpreter.wasm_has_reached_bottom()
    }

    /**
     * Run until the program pauses, requests an interrupt, stores into the
     * watched range, stops with `stop` and nothing to wake it (`Stopped`), or
     * terminates. Every IRQ ready on the way is taken, a step of its own, the
     * one that woke a stopped processor first. It does not stop on a
     * write into code; {@link Interpreter.runWithBreakpoints} can.
     */
    run(): InterpreterStatus {
        return this.interpreter.wasm_run()
    }

    /**
     * How many instructions have run since this Interpreter was made, the one
     * that failed or raised an interrupt included; an IRQ taken runs none and
     * is not counted. Undo does not take them back. A run that stops early, at
     * an interrupt, a pause or a watched write, says nothing of how far it got:
     * the difference of two readings does. The `limit` of
     * {@link Interpreter.runWithLimit} and {@link Interpreter.runWithBreakpoints}
     * counts the same instructions.
     */
    getExecutedInstructions(): number {
        return this.interpreter.wasm_get_executed_instructions()
    }

    /**
     * The cycle counter: the clock cycles the program has taken since it
     * started, or since trap #15 task 30 last cleared it. Each instruction
     * counts its processor's own time, its data included (a multiply by its
     * bits, a shift by its count, a branch taken or not), and so does each
     * exception and each IRQ (44 on a 68000, 46 on a 68010, 26 on a 68020);
     * the 68010's loop mode included; trap #15's tasks take none, as in
     * EASy68K. The 68000's and the 68010's are exact, and the 68020's an
     * estimate, the cache case of its manual's tables
     * ({@link Program.hasExactCycles}). Undo takes a
     * step's cycles back, and each step of the history says what it took, as
     * `cycles`. Task 31 reads the counter into D1.L, 0 once it is past 32 bits.
     */
    getCycles(): bigint {
        return this.interpreter.wasm_get_cycles()
    }

    /**
     * Like {@link Interpreter.run}, but run at most `limit` instructions, as
     * {@link Interpreter.getExecutedInstructions} counts them: an IRQ taken on
     * the way is a step and no instruction, and is not one of them. It throws
     * an `ExecutionLimit` when they have run and
     * the program would run on; a stop made by the last of them is answered as
     * that status, and a processor `stop` stopped answers `Stopped` at once
     * while nothing wakes it.
     */
    runWithLimit(limit: number): InterpreterStatus {
        return this.interpreter.wasm_run_with_limit(limit)
    }

    /**
     * Run until one of `breakpoints` is reached, the program pauses, requests
     * an interrupt, stores into the watched range or ends, writes into code
     * when `stopOnCodeWrite` asks for it, or `limit` instructions have run.
     *
     * A breakpoint is a line of a file: a breakpoint on a comment, a directive
     * or a label alone stops nothing, and neither does one on a line of a file
     * this program was not assembled from.
     *
     * `skipBreakpointAtPc` says what a breakpoint on the instruction the
     * program counter is *already* on does, and only that one: it defaults to
     * `true`, which runs it anyway and is what makes "continue" from a
     * breakpoint move. A caller that resumes mid-program — after answering an
     * interrupt, after a `WatchedWrite` stop, or after its own budget ran
     * out — passes `false`, because the instruction it is about to run has
     * not run yet and a breakpoint on it has not been reported. Every
     * breakpoint the run reaches later stops it before the instruction
     * executes, whichever value this takes.
     *
     * A stop made by the last of the `limit` instructions — an interrupt, a
     * pause, a watched write, a code write, the end — is answered as that
     * status; the `ExecutionLimit` is thrown only when the program would run on.
     *
     * With `stopOnCodeWrite`, an instruction that wrote into code stops the run
     * after it, with `InterpreterStatus.CodeWrite`, and the next call resumes,
     * with `skipBreakpointAtPc: false` like any resume mid-program. A watched
     * write by the same instruction stops it as `WatchedWrite` first.
     *
     * An IRQ ready to be taken is taken before the breakpoint on the program
     * counter is looked at, since the handler's instruction is the next to run:
     * a breakpoint in the handler stops the run there, and the one it came
     * before stops it when the handler returns, its instruction being about to
     * run. `skipBreakpointAtPc` is about the run's first step, which is then the
     * IRQ, so continuing from a breakpoint with a request pending runs the
     * handler and stops at the same breakpoint again. A `stop` stops the run
     * with `Stopped`, and a run of a processor that waits answers `Stopped` at
     * once.
     */
    runWithBreakpoints(
        breakpoints: Breakpoint[],
        limit?: number,
        {skipBreakpointAtPc = true, stopOnCodeWrite = false}: RunWithBreakpointsOptions = {}
    ): InterpreterStatus {
        return this.interpreter.wasm_run_with_breakpoints(breakpoints, limit, skipBreakpointAtPc, stopOnCodeWrite)
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
 * Every entry point is static; there is nothing to construct.
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

    /**
     * Every mnemonic the assembler accepts, with the forms it may be written
     * in: per form, the addressing modes of each operand position, the sizes,
     * whether the 68000 reserves it for supervisor mode, and the first
     * processor that has it.
     *
     * ```ts
     * S68k.instructionTable().find(({mnemonic}) => mnemonic === 'andi')?.forms[2]
     * // {operands: [["Im"], ["sr"]], sizes: ["word"], privileged: true, processor: "68000"}
     * ```
     *
     * The mnemonics come in the table's order and the forms in the order the
     * assembler tries them. Privileged is the 68000's rule: s68k runs every
     * program in supervisor mode, as EASy68K does, so a privileged form still
     * runs. Each call builds a new array.
     */
    static instructionTable(): InstructionInfo[] {
        return wasm_get_instruction_table() as InstructionInfo[]
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
    /** An instruction the program ran, a poke the host made between two of them, or an IRQ taken. */
    kind: ExecutionStepKind,
    mutations: MutationOperation[],
    /** What a poke wrote, old and new; empty on an instruction. */
    writes: PokeWrite[],
    pc: number,
    old_ccr: string,
    new_ccr: string
    /** The whole status register before the step; a number, not a bitfield name. */
    old_sr: number,
    /** The whole status register after it. */
    new_sr: number,
    /** The cycles the step took; 0 for a poke. */
    cycles: number,
    location?: Location,
    /** The code write the instruction made; absent when it made none. */
    wroteCode?: CodeWrite,
    /** What the step ran, from its words as they were then; absent on a poke. */
    ran?: DisassembledLine
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
    Access,
    AutoIrq,
    Breakpoint,
    CodeWrite,
    Condition,
    DeviceEvent,
    Diagnostic,
    DisassembledLine,
    ExecutionStep,
    ExecutionStepKind,
    FileDialogMode,
    FileExistence,
    Flags,
    InputSettings,
    InstructionForm,
    InstructionInfo,
    InstructionLine,
    InterpreterOptions,
    InterpreterStatus,
    Interrupt,
    InterruptResult,
    KeyStateRequest,
    KeyStateResult,
    LineSpan,
    Location,
    MachineState,
    MemoryRange,
    MemoryRangeKind,
    MutationOperation,
    OpenedFile,
    ParsedComment,
    ParsedLabel,
    ParsedLine,
    ParsedLineKind,
    ParsedOperand,
    ParsedOperation,
    ParsedText,
    PokeWrite,
    Processor,
    ProgramInfo,
    ProgramSymbol,
    RegisterOperand,
    RelatedLocation,
    RuntimeError,
    Severity,
    Size,
    SoundOutcome,
    StackFrame,
    SuggestedFix,
    SystemRegisters,
    Termination,
}
