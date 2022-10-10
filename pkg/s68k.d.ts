/* tslint:disable */
/* eslint-disable */
/**
*/
export enum InterpreterStatus {
  Running,
  Interrupt,
  Terminated,
  TerminatedWithException,
}
/**
*/
export enum Size {
  Byte,
  Word,
  Long,
}
/**
*/
export enum Condition {
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
/**
*/
export class Cpu {
  free(): void;
/**
* @param {number} index
* @returns {Register}
*/
  wasm_get_d_reg(index: number): Register;
/**
* @returns {Uint32Array}
*/
  wasm_get_d_regs_value(): Uint32Array;
/**
* @returns {Uint32Array}
*/
  wasm_get_a_regs_value(): Uint32Array;
/**
* @param {number} index
* @returns {Register}
*/
  wasm_get_a_reg(index: number): Register;
/**
* @returns {Flags}
*/
  wasm_get_ccr(): Flags;
}
/**
*/
export class Flags {
  free(): void;
/**
* @returns {Flags}
*/
  static new(): Flags;
/**
*/
  clear(): void;
/**
* @returns {string}
*/
  get_status(): string;
}
/**
*/
export class Interpreter {
  free(): void;
/**
* @param {number} address
* @param {number} size
* @returns {Uint8Array}
*/
  wasm_read_memory_bytes(address: number, size: number): Uint8Array;
/**
* @returns {Cpu}
*/
  wasm_get_cpu_snapshot(): Cpu;
/**
* @returns {number}
*/
  wasm_get_pc(): number;
/**
* @param {number} address
* @returns {any}
*/
  wasm_get_instruction_at(address: number): any;
/**
* @returns {any}
*/
  wasm_step(): any;
/**
* @returns {number}
*/
  wasm_run(): number;
/**
* @returns {number}
*/
  wasm_get_status(): number;
/**
* @param {Flags} flag
* @returns {boolean}
*/
  wasm_get_flag(flag: Flags): boolean;
/**
* @param {number} cond
* @returns {boolean}
*/
  wasm_get_condition_value(cond: number): boolean;
/**
* @param {any} reg
* @param {number} size
* @returns {number}
*/
  wasm_get_register_value(reg: any, size: number): number;
/**
* @param {any} reg
* @param {number} value
* @param {number} size
*/
  wasm_set_register_value(reg: any, value: number, size: number): void;
/**
* @returns {boolean}
*/
  wasm_has_reached_bottom(): boolean;
/**
* @returns {boolean}
*/
  wasm_has_terminated(): boolean;
/**
* @returns {any}
*/
  wasm_get_current_interrupt(): any;
/**
* @param {any} value
*/
  wasm_answer_interrupt(value: any): void;
}
/**
*/
export class Memory {
  free(): void;
/**
* @param {number} address
* @param {number} size
* @returns {Uint8Array}
*/
  wasm_read_bytes(address: number, size: number): Uint8Array;
/**
*/
  sp: number;
}
/**
*/
export class PreInterpreter {
  free(): void;
}
/**
*/
export class Register {
  free(): void;
/**
* @returns {number}
*/
  wasm_get_long(): number;
/**
* @returns {number}
*/
  wasm_get_word(): number;
/**
* @returns {number}
*/
  wasm_get_byte(): number;
}
/**
*/
export class S68k {
  free(): void;
/**
* @param {string} code
*/
  constructor(code: string);
/**
* @returns {any}
*/
  wasm_get_lexed_lines(): any;
/**
* @returns {PreInterpreter}
*/
  wasm_pre_process(): PreInterpreter;
/**
* @returns {WasmSemanticErrors}
*/
  wasm_semantic_check(): WasmSemanticErrors;
/**
* @param {PreInterpreter} pre_processed_program
* @param {number} memory_size
* @returns {Interpreter}
*/
  wasm_create_interpreter(pre_processed_program: PreInterpreter, memory_size: number): Interpreter;
}
/**
*/
export class SemanticError {
  free(): void;
/**
* @returns {string}
*/
  wasm_get_message(): string;
/**
* @returns {any}
*/
  wasm_get_line(): any;
}
/**
*/
export class WasmSemanticErrors {
  free(): void;
/**
* @returns {number}
*/
  get_length(): number;
/**
* @returns {any[]}
*/
  get_errors(): any[];
/**
* @param {number} index
* @returns {SemanticError}
*/
  get_error_at_index(index: number): SemanticError;
}
