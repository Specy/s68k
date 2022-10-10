/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export function __wbg_memory_free(a: number): void;
export function __wbg_get_memory_sp(a: number): number;
export function __wbg_set_memory_sp(a: number, b: number): void;
export function __wbg_flags_free(a: number): void;
export function flags_new(): number;
export function flags_clear(a: number): void;
export function flags_get_status(a: number, b: number): void;
export function memory_wasm_read_bytes(a: number, b: number, c: number, d: number): void;
export function __wbg_register_free(a: number): void;
export function register_wasm_get_long(a: number): number;
export function register_wasm_get_word(a: number): number;
export function register_wasm_get_byte(a: number): number;
export function __wbg_cpu_free(a: number): void;
export function cpu_wasm_get_d_reg(a: number, b: number): number;
export function cpu_wasm_get_d_regs_value(a: number, b: number): void;
export function cpu_wasm_get_a_regs_value(a: number, b: number): void;
export function cpu_wasm_get_a_reg(a: number, b: number): number;
export function cpu_wasm_get_ccr(a: number): number;
export function __wbg_interpreter_free(a: number): void;
export function interpreter_wasm_read_memory_bytes(a: number, b: number, c: number, d: number): void;
export function interpreter_wasm_get_cpu_snapshot(a: number): number;
export function interpreter_wasm_get_pc(a: number): number;
export function interpreter_wasm_get_instruction_at(a: number, b: number): number;
export function interpreter_wasm_step(a: number): number;
export function interpreter_wasm_run(a: number): number;
export function interpreter_wasm_get_status(a: number): number;
export function interpreter_wasm_get_flag(a: number, b: number): number;
export function interpreter_wasm_get_condition_value(a: number, b: number): number;
export function interpreter_wasm_get_register_value(a: number, b: number, c: number): number;
export function interpreter_wasm_set_register_value(a: number, b: number, c: number, d: number): void;
export function interpreter_wasm_has_reached_bottom(a: number): number;
export function interpreter_wasm_has_terminated(a: number): number;
export function interpreter_wasm_get_current_interrupt(a: number): number;
export function interpreter_wasm_answer_interrupt(a: number, b: number): void;
export function __wbg_s68k_free(a: number): void;
export function s68k_wasm_new(a: number, b: number): number;
export function s68k_wasm_get_lexed_lines(a: number, b: number): void;
export function s68k_wasm_pre_process(a: number): number;
export function s68k_wasm_semantic_check(a: number): number;
export function s68k_wasm_create_interpreter(a: number, b: number, c: number): number;
export function __wbg_wasmsemanticerrors_free(a: number): void;
export function wasmsemanticerrors_get_length(a: number): number;
export function wasmsemanticerrors_get_errors(a: number, b: number): void;
export function wasmsemanticerrors_get_error_at_index(a: number, b: number): number;
export function __wbg_semanticerror_free(a: number): void;
export function semanticerror_wasm_get_message(a: number, b: number): void;
export function semanticerror_wasm_get_line(a: number): number;
export function __wbg_preinterpreter_free(a: number): void;
export function __wbindgen_malloc(a: number): number;
export function __wbindgen_realloc(a: number, b: number, c: number): number;
export function __wbindgen_add_to_stack_pointer(a: number): number;
export function __wbindgen_free(a: number, b: number): void;
export function __wbindgen_exn_store(a: number): void;
