import { add } from './a/test';
import { Flags, S68k as RawS68k, SemanticError as RawSemanticError, Interpreter as RawInterpreter, PreInterpreter as RawPreInterpreter, InterruptResult, Step, Condition, Cpu as RawCpu, Interrupt, InstructionLine, InterpreterStatus, RegisterOperand, Size, Register as RawRegister } from './pkg';

type CompilationResult =
    | {
        errors: SemanticError[];
    }
    | {
        interpreter: Interpreter;
    };

export { RawS68k, RawInterpreter, RawSemanticError, RawPreInterpreter, RawCpu, RawRegister };

export enum RegisterType {
    Data,
    Address,
}
class Register {
    private register: RawRegister;
    constructor(register: RawRegister) {
        this.register = register;
    }
    getLong() {
        return this.register.wasm_get_long();
    }
    getWord() {
        return this.register.wasm_get_word();
    }
    getByte() {
        return this.register.wasm_get_byte();
    }
}

class Cpu {
    cpu: RawCpu;
    constructor(cpu: RawCpu) {
        this.cpu = cpu;
    }
    getRegistersValues(): number[] {
        const aReg = this.cpu.wasm_get_a_regs_value()
        const dReg = this.cpu.wasm_get_d_regs_value()
        return [...dReg, ...aReg]
    }
    getRegister(register: number, type: RegisterType): Register {
        if (type == RegisterType.Data) {
            return new Register(this.cpu.wasm_get_d_reg(register))
        }
        else {
            return new Register(this.cpu.wasm_get_a_reg(register))
        }
    }
    getRegisterValue(register: number, type: RegisterType): number {
        return this.getRegister(register, type).getLong()
    }
}

type InterruptHandler = (interrupt: Interrupt) => Promise<InterruptResult> | void


class Interpreter {
    private interpreter: RawInterpreter
    constructor(interpreter: RawInterpreter) {
        this.interpreter = interpreter;
    }

    answerInterrupt(interruptResult: InterruptResult) {
        this.interpreter.wasm_answer_interrupt(interruptResult);
    }
    step(): Step {
        return this.interpreter.wasm_step()
    }
    async stepWithInterruptHandler(onInterrupt: InterruptHandler): Promise<Step> {
        const step = this.interpreter.wasm_step() as Step
        const [_, status] = step
        if (status == InterpreterStatus.Interrupt) {
            let result = await onInterrupt(this.getCurrentInterrupt()!)
            if (result) this.answerInterrupt(result)
        }
        return step
    }
    getConditionValue(condition: Condition): boolean {
        return this.interpreter.wasm_get_condition_value(condition);
    }
    getCpuSnapshot(): Cpu {
        return new Cpu(this.interpreter.wasm_get_cpu_snapshot());
    }
    getCurrentInterrupt(): Interrupt | null {
        return this.interpreter.wasm_get_current_interrupt();
    }
    getPc(): number {
        return this.interpreter.wasm_get_pc();
    }
    getSp(): number {
        return this.interpreter.wasm_get_sp();
    }
    readMemoryBytes(address: number, length: number): Uint8Array {
        return this.interpreter.wasm_read_memory_bytes(address, length);
    }
    getFlag(flag: Flags): boolean {
        return this.interpreter.wasm_get_flag(flag);
    }
    getCurrentLineIndex(): number {
        return this.interpreter.wasm_get_current_line_index();
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
    hasTerminated(): boolean {
        return this.interpreter.wasm_has_terminated();
    }
    hasReachedBottom(): boolean {
        return this.interpreter.wasm_has_reached_bottom();
    }
    run(): InterpreterStatus {
        return this.interpreter.wasm_run()
    }
    async runWithInterruptHandler(onInterrupt: InterruptHandler): Promise<InterpreterStatus> {
        const status = this.interpreter.wasm_run() as InterpreterStatus
        if (status == InterpreterStatus.Interrupt) {
            let result = await onInterrupt(this.getCurrentInterrupt()!)
            if (result) this.answerInterrupt(result)
        }
        return status
    }
}
class SemanticError {
    error: RawSemanticError;
    constructor(error: RawSemanticError) {
        this.error = error;
    }
    getMessage() {
        return this.error.wasm_get_message();
    }
    getLineIndex() {
        const result = add(1, 2); //just to test the bundler
        return this.error.wasm_get_line();
    }
}

export class S68k {
    private _s68k: RawS68k;

    constructor(code: string) {
        this._s68k = new RawS68k(code);
    }

    static compile(code: string, memorySize: number): CompilationResult {
        const s68k = new S68k(code);
        const errors = s68k.semanticCheck()
        if (errors.length > 0) return { errors }
        const interpreter = s68k.createInterpreter(memorySize)
        return { interpreter }
    }

    static semanticCheck(code: string): SemanticError[] {
        let s68k = new S68k(code)
        return s68k.semanticCheck()
    }

    semanticCheck(): SemanticError[] {
        const errorWrapper = this._s68k.wasm_semantic_check()
        const errors: SemanticError[] = []
        for (let i = 0; i < errorWrapper.get_length(); i++) {
            errors.push(new SemanticError(errorWrapper.get_error_at_index(i)))
        }
        return errors;
    }
    preCompile(): RawPreInterpreter {
        return this._s68k.wasm_pre_process()
    }
    createInterpreter(memorySize: number, preCompiled?: RawPreInterpreter): Interpreter {
        if (preCompiled) {
            return new Interpreter(this._s68k.wasm_create_interpreter(preCompiled, memorySize))
        }
        return new Interpreter(this._s68k.wasm_create_interpreter(this.preCompile(), memorySize))
    }
}

