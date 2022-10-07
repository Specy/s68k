import { S68k } from "../pkg/s68k" 

const run = document.getElementById("run-button")
const code = document.getElementById("code-text")
const errorWrapper = document.getElementById("error-wrapper")
code.value = localStorage.getItem("s68k_code") || ""
run.addEventListener("click", () => {
    const text = code.value
    const s68k = new S68k(text) 
    const errors = s68k.wasm_semantic_check()
    const lines = s68k.wasm_get_lexed_lines()
    console.log(lines)
    //console.log(errors) 
    localStorage.setItem("s68k_code", text)
    errorWrapper.innerHTML = ""
    for (let i = 0; i < errors.get_length(); i++) {
        const errorEl = document.createElement("div")
        const error = errors.get_error_at_index(i)
        errorEl.className = "error"
        errorEl.innerText = error.wasm_get_message()
        console.log(error.wasm_get_line())
        errorWrapper.appendChild(errorEl)
    }
    const preProcess = s68k.wasm_pre_process()
    //console.log(preProcess)
    const interpreter = s68k.wasm_create_interpreter(preProcess, Math.pow(2, 16))
    interpreter.wasm_run()
    const cpu = interpreter.wasm_get_cpu_snapshot()
    console.group("D registers")
    console.table(cpu.wasm_get_d_regs_value())
    console.groupEnd()
    console.group("A registers")
    console.table(cpu.wasm_get_a_regs_value())
    console.groupEnd()
})