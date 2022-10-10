import { S68k, Interpreter, SemanticError } from "../pkg/s68k"
const compile = document.getElementById("compile-button")
const run = document.getElementById("run-button") as HTMLButtonElement
const step = document.getElementById("step-button") as HTMLButtonElement
const clear = document.getElementById("clear-button") as HTMLButtonElement
const code = document.getElementById("code-text") as HTMLInputElement
const errorWrapper = document.getElementById("error-wrapper")
const registersWrapper = document.getElementById("registers")
const currentInstruction = document.getElementById("current-instruction")
const stdOut = document.getElementById("std-out") as HTMLDivElement
const memBefore = document.getElementById("mem-before") as HTMLDivElement
const memAfter = document.getElementById("mem-after") as HTMLDivElement
const memAddress = document.getElementById("mem-address") as HTMLInputElement
const memory = document.getElementById("memory") as HTMLDivElement
code.value = localStorage.getItem("s68k_code") ?? ""
let currentProgram: S68k | null = null
let currentInterpreter: Interpreter | null = null

compile.addEventListener("click", () => {
    const text = code.value
    currentProgram = new S68k(text)
    const errors = currentProgram.wasm_semantic_check()
    localStorage.setItem("s68k_code", text)
    errorWrapper.innerHTML = ""
    const len = errors.get_length()
    const er = new Array(len).fill(0).map((_, i) => errors.get_error_at_index(i))
    er.forEach(error => {
        const errorEl = document.createElement("div")
        errorEl.className = "error"
        errorEl.innerText = error.wasm_get_message()
        console.log(error.wasm_get_line())
        errorWrapper.appendChild(errorEl)
    })
    const preProcess = currentProgram.wasm_pre_process()
    currentInterpreter = currentProgram.wasm_create_interpreter(preProcess, Math.pow(2, 16))
    updateRegisters(Array(regs.length).fill(0))
    disableButtons(false)
    updateMemoryTable()
    stdOut.innerText = ""
    currentInstruction.innerText = ""
})


function disableButtons(value: boolean) {
    run.disabled = value
    step.disabled = value
    clear.disabled = value
}

function disableExecution(value: boolean) {
    run.disabled = value
    step.disabled = value
}
clear.addEventListener("click", () => {
    currentProgram = null
    currentInterpreter = null
    updateRegisters(Array(regs.length).fill(0))
    disableButtons(true)
    currentInstruction.innerText = ""
    stdOut.innerText = ""
    updateMemoryTable()
})

type Interrupt = { type: "DisplayStringWithCRLF", value: string} |
    { type: "DisplayStringWithoutCRLF", value: string} |
    { type: "ReadKeyboardString" } |
    { type: "DisplayNumber", value: number } |
    { type: "ReadNumber" } |
    { type: "ReadChar" } |
    { type: "GetTime" } |
    { type: "Terminate" }

type InterruptResult = { type: "DisplayStringWithCRLF" } |
    { type: "DisplayStringWithoutCRLF" } |
    { type: "ReadKeyboardString", value: string } |
    { type: "DisplayNumber" } |
    { type: "ReadNumber", value: number } |
    { type: "ReadChar", value: string } |
    { type: "GetTime", value: number } |
    { type: "Terminate" }


function handleInterrupt(interrupt: Interrupt) {
    console.log(interrupt)
    switch (interrupt.type) {
        case "DisplayStringWithCRLF": {
            console.log(interrupt.value)
            stdOut.innerText += interrupt.value + "\n"
            currentInterpreter.wasm_answer_interrupt({ type: "DisplayStringWithCRLF" })
            break
        }
        case "DisplayStringWithoutCRLF": {
            console.log(interrupt.value)
            stdOut.innerText += interrupt.value
            currentInterpreter.wasm_answer_interrupt({ type: "DisplayStringWithoutCRLF" })
            break
        }
        case "ReadKeyboardString": {
            const input = prompt("Enter a string")
            const result:InterruptResult = { type: "ReadKeyboardString", value: input ?? "" }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "DisplayNumber": {
            console.log(interrupt.value)
            stdOut.innerText += interrupt.value
            currentInterpreter?.wasm_answer_interrupt({ type: "DisplayNumber" })
            break
        }
        case "ReadNumber": {
            const input = prompt("Enter a number")
            const result:InterruptResult = { type: "ReadNumber", value: parseInt(input ?? "0") }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "ReadChar": {
            const input = prompt("Enter a char")
            const result:InterruptResult = { type: "ReadChar", value: input ?? "" }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "GetTime": {
            const result:InterruptResult = { type: "GetTime", value: Date.now() }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "Terminate": {
            currentInterpreter?.wasm_answer_interrupt({type: "Terminate"})
            break
        }
        default: {
            console.error("Unknown interrupt")
        }
    }
}


run.addEventListener('click', async () => {
    while (!currentInterpreter.wasm_has_terminated()){
        let status = currentInterpreter.wasm_run()
        if(status == 1){
            let interrupt = currentInterpreter.wasm_get_current_interrupt()
            handleInterrupt(interrupt)
        }
        const cpu = currentInterpreter.wasm_get_cpu_snapshot()
        updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
    }

    disableExecution(true)
})

step.addEventListener("click", () => {
    if (currentInterpreter) {
        let a = currentInterpreter.wasm_step()
        let status = currentInterpreter.wasm_get_status()
        if (currentInterpreter.wasm_has_terminated()){
            disableExecution(true)
        }
        const instruction = a[0]

        if(instruction){
            showCurrent(instruction.parsed_line)
        }
        const cpu = currentInterpreter.wasm_get_cpu_snapshot()
        updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
        updateMemoryTable()
        if(status == 1){
            let interrupt = currentInterpreter.wasm_get_current_interrupt()
            handleInterrupt(interrupt)
            const cpu = currentInterpreter.wasm_get_cpu_snapshot()
            updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
        }
    }
})


function showCurrent(ins: any) {
    currentInstruction.innerText = ins.line
}


function updateMemoryTable(){
    if(!currentInterpreter) return
    const data = currentInterpreter.wasm_read_memory_bytes(Number(memAddress.value), 16*16)
    data.forEach((byte, i) => {
        const cell = memory.children[i] as HTMLSpanElement
        const value = byte.toString(16).toUpperCase().padStart(2, "0")
        if(cell.innerText.toUpperCase() !== value){
            cell.innerText = value
        }
    })
}

memAddress.addEventListener("change", () => {
    updateMemoryTable()
})
memBefore.addEventListener("click", () => {
    memAddress.value = (Number(memAddress.value) - 16*16).toString()
    updateMemoryTable()
})
memAfter.addEventListener("click", () => {
    memAddress.value = (Number(memAddress.value) + 16*16).toString()
    updateMemoryTable()
})

function createMemoryTable(pageSize: number){
    const elements = new Array(pageSize).fill(0).map((_, i) => {
        const el = document.createElement('span')
        el.innerText = "FF"
        return el
    })
    memory.innerHTML = ""
    memory.append(...elements)
}

function updateRegisters(values: number[]) {
    const spans = registersWrapper.querySelectorAll("span")
    spans.forEach((s, i) => {
        s.innerText = `${values[i]}`
    })
}
const regs = [...new Array(8).fill(0).map((_, i) => `D${i}`), ...new Array(8).fill(0).map((_, i) => `A${i}`)]
function createRegisters() {
    registersWrapper.innerHTML = ""
    registersWrapper.append(...regs.map((r) => {
        const el = document.createElement("div")
        el.className = "register"
        el.innerText = `${r}: `
        const value = document.createElement("span")
        value.innerText = "0"
        el.append(value)
        return el
    }))
}
createRegisters()
disableButtons(true)
createMemoryTable(16*16)