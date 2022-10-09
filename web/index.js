import { S68k } from "../pkg/s68k"

const compile = document.getElementById("compile-button")
const run = document.getElementById("run-button")
const step = document.getElementById("step-button")
const clear = document.getElementById("clear-button")
const code = document.getElementById("code-text")
const errorWrapper = document.getElementById("error-wrapper")
const registersWrapper = document.getElementById("registers")
const currentInstruction = document.getElementById("current-instruction")
code.value = localStorage.getItem("s68k_code") || ""
let currentProgram = null
let currentInterpreter = null
compile.addEventListener("click", () => {
    const text = code.value
    currentProgram = new S68k(text)
    const errors = currentProgram.wasm_semantic_check()
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
    const preProcess = currentProgram.wasm_pre_process()
    currentInterpreter = currentProgram.wasm_create_interpreter(preProcess, Math.pow(2, 16))
    updateRegisters(Array(regs.length).fill(0))
    disableButtons(false)
    currentInstruction.innerText = ""
})


function disableButtons(value) {
    run.disabled = value
    step.disabled = value
    clear.disabled = value
}

function disableExecution(value) {
    run.disabled = value
    step.disabled = value
}
clear.addEventListener("click", () => {
    currentProgram = null
    currentInterpreter = null
    updateRegisters(Array(regs.length).fill(0))
    disableButtons(true)
    currentInstruction.innerText = ""
})

function handleInterrupt() {

}


run.addEventListener('click', () => {
    currentInterpreter.wasm_run()
    const cpu = currentInterpreter.wasm_get_cpu_snapshot()
    updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])

    disableExecution(true)
})

step.addEventListener("click", () => {
    if (currentInterpreter) {
        let instruction = currentInterpreter.wasm_step()
        instruction.parsed_line ? showCurrent(instruction.parsed_line) : disableExecution(true)
        const cpu = currentInterpreter.wasm_get_cpu_snapshot()
        updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
    }
})


function showCurrent(ins) {
    currentInstruction.innerText = ins.line
}


function updateRegisters(values) {
    const spans = registersWrapper.querySelectorAll("span")
    spans.forEach((s, i) => {
        s.innerText = values[i]
    })
}
const regs = [...new Array(8).fill().map((_, i) => `D${i}`), ...new Array(8).fill().map((_, i) => `A${i}`)]
function createRegisters() {
    registersWrapper.innerHTML = ""
    registersWrapper.append(...regs.map((r) => {
        const el = document.createElement("div")
        el.className = "register"
        el.innerText = `${r}: `
        const value = document.createElement("span")
        value.innerText = 0
        el.append(value)
        return el
    }))
}
createRegisters()
disableButtons(true)