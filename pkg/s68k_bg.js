import * as wasm from './s68k_bg.wasm';

const heap = new Array(32).fill(undefined);

heap.push(undefined, null, true, false);

function getObject(idx) { return heap[idx]; }

let heap_next = heap.length;

function dropObject(idx) {
    if (idx < 36) return;
    heap[idx] = heap_next;
    heap_next = idx;
}

function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
}

function addHeapObject(obj) {
    if (heap_next === heap.length) heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];

    heap[idx] = obj;
    return idx;
}

const lTextDecoder = typeof TextDecoder === 'undefined' ? (0, module.require)('util').TextDecoder : TextDecoder;

let cachedTextDecoder = new lTextDecoder('utf-8', { ignoreBOM: true, fatal: true });

cachedTextDecoder.decode();

let cachedUint8Memory0 = new Uint8Array();

function getUint8Memory0() {
    if (cachedUint8Memory0.byteLength === 0) {
        cachedUint8Memory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8Memory0;
}

function getStringFromWasm0(ptr, len) {
    return cachedTextDecoder.decode(getUint8Memory0().subarray(ptr, ptr + len));
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

let cachedFloat64Memory0 = new Float64Array();

function getFloat64Memory0() {
    if (cachedFloat64Memory0.byteLength === 0) {
        cachedFloat64Memory0 = new Float64Array(wasm.memory.buffer);
    }
    return cachedFloat64Memory0;
}

let cachedInt32Memory0 = new Int32Array();

function getInt32Memory0() {
    if (cachedInt32Memory0.byteLength === 0) {
        cachedInt32Memory0 = new Int32Array(wasm.memory.buffer);
    }
    return cachedInt32Memory0;
}

let WASM_VECTOR_LEN = 0;

const lTextEncoder = typeof TextEncoder === 'undefined' ? (0, module.require)('util').TextEncoder : TextEncoder;

let cachedTextEncoder = new lTextEncoder('utf-8');

const encodeString = (typeof cachedTextEncoder.encodeInto === 'function'
    ? function (arg, view) {
    return cachedTextEncoder.encodeInto(arg, view);
}
    : function (arg, view) {
    const buf = cachedTextEncoder.encode(arg);
    view.set(buf);
    return {
        read: arg.length,
        written: buf.length
    };
});

function passStringToWasm0(arg, malloc, realloc) {

    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length);
        getUint8Memory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len);

    const mem = getUint8Memory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }

    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3);
        const view = getUint8Memory0().subarray(ptr + offset, ptr + len);
        const ret = encodeString(arg, view);

        offset += ret.written;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

function getArrayU8FromWasm0(ptr, len) {
    return getUint8Memory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedUint32Memory0 = new Uint32Array();

function getUint32Memory0() {
    if (cachedUint32Memory0.byteLength === 0) {
        cachedUint32Memory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32Memory0;
}

function getArrayU32FromWasm0(ptr, len) {
    return getUint32Memory0().subarray(ptr / 4, ptr / 4 + len);
}

function _assertClass(instance, klass) {
    if (!(instance instanceof klass)) {
        throw new Error(`expected instance of ${klass.name}`);
    }
    return instance.ptr;
}

function getArrayJsValueFromWasm0(ptr, len) {
    const mem = getUint32Memory0();
    const slice = mem.subarray(ptr / 4, ptr / 4 + len);
    const result = [];
    for (let i = 0; i < slice.length; i++) {
        result.push(takeObject(slice[i]));
    }
    return result;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        wasm.__wbindgen_exn_store(addHeapObject(e));
    }
}
/**
*/
export const InterpreterStatus = Object.freeze({ Running:0,"0":"Running",Interrupt:1,"1":"Interrupt",Terminated:2,"2":"Terminated",TerminatedWithException:3,"3":"TerminatedWithException", });
/**
*/
export const Size = Object.freeze({ Byte:0,"0":"Byte",Word:1,"1":"Word",Long:2,"2":"Long", });
/**
*/
export const Condition = Object.freeze({ True:0,"0":"True",False:1,"1":"False",High:2,"2":"High",LowOrSame:3,"3":"LowOrSame",CarryClear:4,"4":"CarryClear",CarrySet:5,"5":"CarrySet",NotEqual:6,"6":"NotEqual",Equal:7,"7":"Equal",OverflowClear:8,"8":"OverflowClear",OverflowSet:9,"9":"OverflowSet",Plus:10,"10":"Plus",Minus:11,"11":"Minus",GreaterThanOrEqual:12,"12":"GreaterThanOrEqual",LessThan:13,"13":"LessThan",GreaterThan:14,"14":"GreaterThan",LessThanOrEqual:15,"15":"LessThanOrEqual", });
/**
*/
export class Compiler {

    static __wrap(ptr) {
        const obj = Object.create(Compiler.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_compiler_free(ptr);
    }
}
/**
*/
export class Cpu {

    static __wrap(ptr) {
        const obj = Object.create(Cpu.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_cpu_free(ptr);
    }
    /**
    * @param {number} index
    * @returns {Register}
    */
    wasm_get_d_reg(index) {
        const ret = wasm.cpu_wasm_get_d_reg(this.ptr, index);
        return Register.__wrap(ret);
    }
    /**
    * @returns {Uint32Array}
    */
    wasm_get_d_regs_value() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.cpu_wasm_get_d_regs_value(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 4);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {Uint32Array}
    */
    wasm_get_a_regs_value() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.cpu_wasm_get_a_regs_value(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 4);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @param {number} index
    * @returns {Register}
    */
    wasm_get_a_reg(index) {
        const ret = wasm.cpu_wasm_get_a_reg(this.ptr, index);
        return Register.__wrap(ret);
    }
    /**
    * @returns {Flags}
    */
    wasm_get_ccr() {
        const ret = wasm.cpu_wasm_get_ccr(this.ptr);
        return Flags.__wrap(ret);
    }
}
/**
*/
export class Flags {

    static __wrap(ptr) {
        const obj = Object.create(Flags.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_flags_free(ptr);
    }
    /**
    * @returns {Flags}
    */
    static new() {
        const ret = wasm.flags_new();
        return Flags.__wrap(ret);
    }
    /**
    */
    clear() {
        wasm.flags_clear(this.ptr);
    }
    /**
    * @returns {string}
    */
    get_status() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.flags_get_status(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_free(r0, r1);
        }
    }
}
/**
*/
export class Interpreter {

    static __wrap(ptr) {
        const obj = Object.create(Interpreter.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_interpreter_free(ptr);
    }
    /**
    * @param {number} address
    * @param {number} size
    * @returns {Uint8Array}
    */
    wasm_read_memory_bytes(address, size) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_read_memory_bytes(retptr, this.ptr, address, size);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 1);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {Cpu}
    */
    wasm_get_cpu_snapshot() {
        const ret = wasm.interpreter_wasm_get_cpu_snapshot(this.ptr);
        return Cpu.__wrap(ret);
    }
    /**
    * @returns {number}
    */
    wasm_get_pc() {
        const ret = wasm.interpreter_wasm_get_pc(this.ptr);
        return ret >>> 0;
    }
    /**
    * @returns {number}
    */
    wasm_get_sp() {
        const ret = wasm.interpreter_wasm_get_sp(this.ptr);
        return ret >>> 0;
    }
    /**
    * @param {number} address
    * @returns {any}
    */
    wasm_get_instruction_at(address) {
        const ret = wasm.interpreter_wasm_get_instruction_at(this.ptr, address);
        return takeObject(ret);
    }
    /**
    * @returns {any}
    */
    wasm_step() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_step(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var r2 = getInt32Memory0()[retptr / 4 + 2];
            if (r2) {
                throw takeObject(r1);
            }
            return takeObject(r0);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {number}
    */
    wasm_run() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_run(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var r2 = getInt32Memory0()[retptr / 4 + 2];
            if (r2) {
                throw takeObject(r1);
            }
            return r0 >>> 0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {number}
    */
    wasm_get_status() {
        const ret = wasm.interpreter_wasm_get_status(this.ptr);
        return ret >>> 0;
    }
    /**
    * @param {Flags} flag
    * @returns {boolean}
    */
    wasm_get_flag(flag) {
        _assertClass(flag, Flags);
        var ptr0 = flag.ptr;
        flag.ptr = 0;
        const ret = wasm.interpreter_wasm_get_flag(this.ptr, ptr0);
        return ret !== 0;
    }
    /**
    * @returns {number}
    */
    wasm_get_flags_as_number() {
        const ret = wasm.interpreter_wasm_get_flags_as_number(this.ptr);
        return ret;
    }
    /**
    * @returns {Uint8Array}
    */
    wasm_get_flags_as_array() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_get_flags_as_array(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 1);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @param {number} cond
    * @returns {boolean}
    */
    wasm_get_condition_value(cond) {
        const ret = wasm.interpreter_wasm_get_condition_value(this.ptr, cond);
        return ret !== 0;
    }
    /**
    * @param {any} reg
    * @param {number} size
    * @returns {number}
    */
    wasm_get_register_value(reg, size) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_get_register_value(retptr, this.ptr, addHeapObject(reg), size);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var r2 = getInt32Memory0()[retptr / 4 + 2];
            if (r2) {
                throw takeObject(r1);
            }
            return r0 >>> 0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @param {any} reg
    * @param {number} value
    * @param {number} size
    */
    wasm_set_register_value(reg, value, size) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_set_register_value(retptr, this.ptr, addHeapObject(reg), value, size);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            if (r1) {
                throw takeObject(r0);
            }
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {boolean}
    */
    wasm_has_reached_bottom() {
        const ret = wasm.interpreter_wasm_has_reached_bottom(this.ptr);
        return ret !== 0;
    }
    /**
    * @returns {boolean}
    */
    wasm_has_terminated() {
        const ret = wasm.interpreter_wasm_has_terminated(this.ptr);
        return ret !== 0;
    }
    /**
    * @returns {any}
    */
    wasm_get_current_interrupt() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_get_current_interrupt(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var r2 = getInt32Memory0()[retptr / 4 + 2];
            if (r2) {
                throw takeObject(r1);
            }
            return takeObject(r0);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @param {any} value
    */
    wasm_answer_interrupt(value) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.interpreter_wasm_answer_interrupt(retptr, this.ptr, addHeapObject(value));
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            if (r1) {
                throw takeObject(r0);
            }
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {number}
    */
    wasm_get_current_line_index() {
        const ret = wasm.interpreter_wasm_get_current_line_index(this.ptr);
        return ret >>> 0;
    }
}
/**
*/
export class Memory {

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_memory_free(ptr);
    }
    /**
    * @param {number} address
    * @param {number} size
    * @returns {Uint8Array}
    */
    wasm_read_bytes(address, size) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.memory_wasm_read_bytes(retptr, this.ptr, address, size);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 1);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
}
/**
*/
export class Register {

    static __wrap(ptr) {
        const obj = Object.create(Register.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_register_free(ptr);
    }
    /**
    * @returns {number}
    */
    wasm_get_long() {
        const ret = wasm.register_wasm_get_long(this.ptr);
        return ret >>> 0;
    }
    /**
    * @returns {number}
    */
    wasm_get_word() {
        const ret = wasm.register_wasm_get_word(this.ptr);
        return ret;
    }
    /**
    * @returns {number}
    */
    wasm_get_byte() {
        const ret = wasm.register_wasm_get_byte(this.ptr);
        return ret;
    }
}
/**
*/
export class S68k {

    static __wrap(ptr) {
        const obj = Object.create(S68k.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_s68k_free(ptr);
    }
    /**
    * @param {string} code
    */
    constructor(code) {
        const ptr0 = passStringToWasm0(code, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.s68k_wasm_new(ptr0, len0);
        return S68k.__wrap(ret);
    }
    /**
    * @returns {any}
    */
    wasm_get_lexed_lines() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.s68k_wasm_get_lexed_lines(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var r2 = getInt32Memory0()[retptr / 4 + 2];
            if (r2) {
                throw takeObject(r1);
            }
            return takeObject(r0);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {Compiler}
    */
    wasm_compile() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.s68k_wasm_compile(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var r2 = getInt32Memory0()[retptr / 4 + 2];
            if (r2) {
                throw takeObject(r1);
            }
            return Compiler.__wrap(r0);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @returns {WasmSemanticErrors}
    */
    wasm_semantic_check() {
        const ret = wasm.s68k_wasm_semantic_check(this.ptr);
        return WasmSemanticErrors.__wrap(ret);
    }
    /**
    * @param {Compiler} pre_processed_program
    * @param {number} memory_size
    * @returns {Interpreter}
    */
    wasm_create_interpreter(pre_processed_program, memory_size) {
        _assertClass(pre_processed_program, Compiler);
        var ptr0 = pre_processed_program.ptr;
        pre_processed_program.ptr = 0;
        const ret = wasm.s68k_wasm_create_interpreter(this.ptr, ptr0, memory_size);
        return Interpreter.__wrap(ret);
    }
}
/**
*/
export class SemanticError {

    static __wrap(ptr) {
        const obj = Object.create(SemanticError.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_semanticerror_free(ptr);
    }
    /**
    * @returns {string}
    */
    wasm_get_message() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.semanticerror_wasm_get_message(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_free(r0, r1);
        }
    }
    /**
    * @returns {any}
    */
    wasm_get_line() {
        const ret = wasm.semanticerror_wasm_get_line(this.ptr);
        return takeObject(ret);
    }
}
/**
*/
export class WasmSemanticErrors {

    static __wrap(ptr) {
        const obj = Object.create(WasmSemanticErrors.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmsemanticerrors_free(ptr);
    }
    /**
    * @returns {number}
    */
    get_length() {
        const ret = wasm.wasmsemanticerrors_get_length(this.ptr);
        return ret >>> 0;
    }
    /**
    * @returns {any[]}
    */
    get_errors() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsemanticerrors_get_errors(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayJsValueFromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 4);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
    * @param {number} index
    * @returns {SemanticError}
    */
    get_error_at_index(index) {
        const ret = wasm.wasmsemanticerrors_get_error_at_index(this.ptr, index);
        return SemanticError.__wrap(ret);
    }
}

export function __wbindgen_object_drop_ref(arg0) {
    takeObject(arg0);
};

export function __wbindgen_object_clone_ref(arg0) {
    const ret = getObject(arg0);
    return addHeapObject(ret);
};

export function __wbindgen_is_bigint(arg0) {
    const ret = typeof(getObject(arg0)) === 'bigint';
    return ret;
};

export function __wbindgen_string_new(arg0, arg1) {
    const ret = getStringFromWasm0(arg0, arg1);
    return addHeapObject(ret);
};

export function __wbindgen_boolean_get(arg0) {
    const v = getObject(arg0);
    const ret = typeof(v) === 'boolean' ? (v ? 1 : 0) : 2;
    return ret;
};

export function __wbindgen_number_get(arg0, arg1) {
    const obj = getObject(arg1);
    const ret = typeof(obj) === 'number' ? obj : undefined;
    getFloat64Memory0()[arg0 / 8 + 1] = isLikeNone(ret) ? 0 : ret;
    getInt32Memory0()[arg0 / 4 + 0] = !isLikeNone(ret);
};

export function __wbindgen_string_get(arg0, arg1) {
    const obj = getObject(arg1);
    const ret = typeof(obj) === 'string' ? obj : undefined;
    var ptr0 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len0 = WASM_VECTOR_LEN;
    getInt32Memory0()[arg0 / 4 + 1] = len0;
    getInt32Memory0()[arg0 / 4 + 0] = ptr0;
};

export function __wbindgen_is_object(arg0) {
    const val = getObject(arg0);
    const ret = typeof(val) === 'object' && val !== null;
    return ret;
};

export function __wbg_new_abda76e883ba8a5f() {
    const ret = new Error();
    return addHeapObject(ret);
};

export function __wbg_stack_658279fe44541cf6(arg0, arg1) {
    const ret = getObject(arg1).stack;
    const ptr0 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    getInt32Memory0()[arg0 / 4 + 1] = len0;
    getInt32Memory0()[arg0 / 4 + 0] = ptr0;
};

export function __wbg_error_f851667af71bcfc6(arg0, arg1) {
    try {
        console.error(getStringFromWasm0(arg0, arg1));
    } finally {
        wasm.__wbindgen_free(arg0, arg1);
    }
};

export function __wbg_BigInt_d0c7d465bfa30d3b(arg0) {
    const ret = BigInt(arg0);
    return addHeapObject(ret);
};

export function __wbindgen_number_new(arg0) {
    const ret = arg0;
    return addHeapObject(ret);
};

export function __wbg_BigInt_1fab4952b6c4a499(arg0) {
    const ret = BigInt(BigInt.asUintN(64, arg0));
    return addHeapObject(ret);
};

export function __wbg_BigInt_06819bca5a5bedef(arg0) {
    const ret = BigInt(getObject(arg0));
    return ret;
};

export function __wbg_BigInt_67359e71cae1c6c9(arg0) {
    const ret = BigInt(getObject(arg0));
    return ret;
};

export function __wbindgen_is_null(arg0) {
    const ret = getObject(arg0) === null;
    return ret;
};

export function __wbindgen_is_undefined(arg0) {
    const ret = getObject(arg0) === undefined;
    return ret;
};

export function __wbg_String_c9c0f9be374874ba(arg0, arg1) {
    const ret = String(getObject(arg1));
    const ptr0 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    getInt32Memory0()[arg0 / 4 + 1] = len0;
    getInt32Memory0()[arg0 / 4 + 0] = ptr0;
};

export function __wbg_get_2268d91a19a98b92(arg0, arg1) {
    const ret = getObject(arg0)[takeObject(arg1)];
    return addHeapObject(ret);
};

export function __wbg_set_c943d600fa71e4dd(arg0, arg1, arg2) {
    getObject(arg0)[takeObject(arg1)] = takeObject(arg2);
};

export function __wbg_get_57245cc7d7c7619d(arg0, arg1) {
    const ret = getObject(arg0)[arg1 >>> 0];
    return addHeapObject(ret);
};

export function __wbg_length_6e3bbe7c8bd4dbd8(arg0) {
    const ret = getObject(arg0).length;
    return ret;
};

export function __wbg_new_1d9a920c6bfc44a8() {
    const ret = new Array();
    return addHeapObject(ret);
};

export function __wbindgen_is_function(arg0) {
    const ret = typeof(getObject(arg0)) === 'function';
    return ret;
};

export function __wbg_next_579e583d33566a86(arg0) {
    const ret = getObject(arg0).next;
    return addHeapObject(ret);
};

export function __wbg_next_aaef7c8aa5e212ac() { return handleError(function (arg0) {
    const ret = getObject(arg0).next();
    return addHeapObject(ret);
}, arguments) };

export function __wbg_done_1b73b0672e15f234(arg0) {
    const ret = getObject(arg0).done;
    return ret;
};

export function __wbg_value_1ccc36bc03462d71(arg0) {
    const ret = getObject(arg0).value;
    return addHeapObject(ret);
};

export function __wbg_iterator_6f9d4f28845f426c() {
    const ret = Symbol.iterator;
    return addHeapObject(ret);
};

export function __wbg_get_765201544a2b6869() { return handleError(function (arg0, arg1) {
    const ret = Reflect.get(getObject(arg0), getObject(arg1));
    return addHeapObject(ret);
}, arguments) };

export function __wbg_call_97ae9d8645dc388b() { return handleError(function (arg0, arg1) {
    const ret = getObject(arg0).call(getObject(arg1));
    return addHeapObject(ret);
}, arguments) };

export function __wbg_new_0b9bfdd97583284e() {
    const ret = new Object();
    return addHeapObject(ret);
};

export function __wbindgen_is_string(arg0) {
    const ret = typeof(getObject(arg0)) === 'string';
    return ret;
};

export function __wbg_length_f2ab5db52e68a619(arg0) {
    const ret = getObject(arg0).length;
    return ret;
};

export function __wbg_codePointAt_4e75511c39fe8398(arg0, arg1) {
    const ret = getObject(arg0).codePointAt(arg1 >>> 0);
    return addHeapObject(ret);
};

export function __wbg_set_a68214f35c417fa9(arg0, arg1, arg2) {
    getObject(arg0)[arg1 >>> 0] = takeObject(arg2);
};

export function __wbg_isArray_27c46c67f498e15d(arg0) {
    const ret = Array.isArray(getObject(arg0));
    return ret;
};

export function __wbg_instanceof_ArrayBuffer_e5e48f4762c5610b(arg0) {
    let result;
    try {
        result = getObject(arg0) instanceof ArrayBuffer;
    } catch {
        result = false;
    }
    const ret = result;
    return ret;
};

export function __wbg_new_8d2af00bc1e329ee(arg0, arg1) {
    const ret = new Error(getStringFromWasm0(arg0, arg1));
    return addHeapObject(ret);
};

export function __wbg_isSafeInteger_dfa0593e8d7ac35a(arg0) {
    const ret = Number.isSafeInteger(getObject(arg0));
    return ret;
};

export function __wbg_entries_65a76a413fc91037(arg0) {
    const ret = Object.entries(getObject(arg0));
    return addHeapObject(ret);
};

export function __wbg_is_40a66842732708e7(arg0, arg1) {
    const ret = Object.is(getObject(arg0), getObject(arg1));
    return ret;
};

export function __wbg_fromCodePoint_3a5b15ba4d213634() { return handleError(function (arg0) {
    const ret = String.fromCodePoint(arg0 >>> 0);
    return addHeapObject(ret);
}, arguments) };

export function __wbg_buffer_3f3d764d4747d564(arg0) {
    const ret = getObject(arg0).buffer;
    return addHeapObject(ret);
};

export function __wbg_new_8c3f0052272a457a(arg0) {
    const ret = new Uint8Array(getObject(arg0));
    return addHeapObject(ret);
};

export function __wbg_set_83db9690f9353e79(arg0, arg1, arg2) {
    getObject(arg0).set(getObject(arg1), arg2 >>> 0);
};

export function __wbg_length_9e1ae1900cb0fbd5(arg0) {
    const ret = getObject(arg0).length;
    return ret;
};

export function __wbg_instanceof_Uint8Array_971eeda69eb75003(arg0) {
    let result;
    try {
        result = getObject(arg0) instanceof Uint8Array;
    } catch {
        result = false;
    }
    const ret = result;
    return ret;
};

export function __wbg_has_8359f114ce042f5a() { return handleError(function (arg0, arg1) {
    const ret = Reflect.has(getObject(arg0), getObject(arg1));
    return ret;
}, arguments) };

export function __wbindgen_debug_string(arg0, arg1) {
    const ret = debugString(getObject(arg1));
    const ptr0 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    getInt32Memory0()[arg0 / 4 + 1] = len0;
    getInt32Memory0()[arg0 / 4 + 0] = ptr0;
};

export function __wbindgen_throw(arg0, arg1) {
    throw new Error(getStringFromWasm0(arg0, arg1));
};

export function __wbindgen_memory() {
    const ret = wasm.memory;
    return addHeapObject(ret);
};

