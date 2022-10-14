import { S68k, Interpreter, SemanticError, Interrupt, InterruptResult } from "../pkg/s68k"
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
const memoryNumbers = document.getElementById("memory-numbers") as HTMLDivElement
const memoryOffsets = document.getElementById("memory-offsets") as HTMLDivElement
const memoryAddresses = document.getElementById("memory-addresses") as HTMLDivElement
const sr = document.getElementById("sr") as HTMLDivElement
code.value = localStorage.getItem("s68k_code") ??
    `ORG $1000

START:
    `
let currentProgram: S68k | null = null
let currentInterpreter: Interpreter | null = null
const MEMORY_SIZE = 0XFFFFFF
const PAGE_SIZE = 16 * 16
compile.addEventListener("click", () => {
    const text = code.value
    try {
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
        updateRegisters(Array(regs.length).fill(0))
        updateMemoryTable()
        stdOut.innerText = ""
        currentInstruction.innerText = ""
        disableButtons(true)
        if (!er.length) {
            try {
                const compiledProgram = currentProgram.wasm_compile()
                currentInterpreter = currentProgram.wasm_create_interpreter(compiledProgram, MEMORY_SIZE)
                disableButtons(false)
            } catch (e) {
                console.log(e)
                const errorEl = document.createElement("div")
                errorEl.className = "error"
                errorEl.innerText = e
                errorWrapper.append(errorEl)
            }

        }
    } catch (e) {
        console.error(e)
        alert("There was an error compiling the program, check the console for more info")
    }
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
code.addEventListener("keydown", (e) => {
    if (e.code === "Tab") {
        e.preventDefault()
        const start = code.selectionStart
        code.value = code.value.substring(0, start) + "\t" + code.value.substring(start, code.value.length)
        code.selectionStart = code.selectionEnd = start + 1
    }
})
clear.addEventListener("click", () => {
    currentProgram = null
    currentInterpreter = null
    updateRegisters(Array(regs.length).fill(0))
    disableButtons(true)
    currentInstruction.innerText = ""
    stdOut.innerText = ""
    updateMemoryTable()
    updateCr()
})


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
            const result: InterruptResult = { type: "ReadKeyboardString", value: input ?? "" }
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
            const result: InterruptResult = { type: "ReadNumber", value: parseInt(input ?? "0") }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "DisplayChar": {
            console.log(interrupt.value)
            stdOut.innerText += interrupt.value
            currentInterpreter?.wasm_answer_interrupt({ type: "DisplayChar" })
            break
        }
        case "ReadChar": {
            const input = prompt("Enter a char")
            const result: InterruptResult = { type: "ReadChar", value: input ?? "" }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "GetTime": {
            const result: InterruptResult = { type: "GetTime", value: Date.now() }
            currentInterpreter?.wasm_answer_interrupt(result)
            break
        }
        case "Terminate": {
            currentInterpreter?.wasm_answer_interrupt({ type: "Terminate" })
            break
        }
        default: {
            console.error("Unknown interrupt")
        }
    }
}


run.addEventListener('click', async () => {
    try {
        while (!currentInterpreter.wasm_has_terminated()) {
            let status = currentInterpreter.wasm_run()
            if (status == 1) {
                let interrupt = currentInterpreter.wasm_get_current_interrupt()
                handleInterrupt(interrupt)
            }
            const cpu = currentInterpreter.wasm_get_cpu_snapshot()
            updateCr()
            updateMemoryTable()
            updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
        }

        disableExecution(true)
    } catch (e) {
        console.error(e)
        alert("Error during execution, check console for more info")
    }
})

step.addEventListener("click", () => {
    try {
        if (currentInterpreter) {
            let a = currentInterpreter.wasm_step()
            let status = currentInterpreter.wasm_get_status()
            if (currentInterpreter.wasm_has_terminated()) {
                disableExecution(true)
            }
            const instruction = a[0]

            if (instruction) {
                showCurrent(instruction.parsed_line)
            }
            const cpu = currentInterpreter.wasm_get_cpu_snapshot()
            updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
            updateMemoryTable()
            updateCr()
            if (status == 1) {
                let interrupt = currentInterpreter.wasm_get_current_interrupt()
                handleInterrupt(interrupt)
                const cpu = currentInterpreter.wasm_get_cpu_snapshot()
                updateRegisters([...cpu.wasm_get_d_regs_value(), ...cpu.wasm_get_a_regs_value()])
            }
        }
    } catch (e) {
        console.error(e)
        alert("Error during execution, check console for more info")
    }

})


function showCurrent(ins: any) {
    currentInstruction.innerText = ins.line
}

function updateCr() {
    const bits = currentInterpreter?.wasm_get_flags_as_array() ?? new Array(5).fill(0)
    bits.forEach((b,i) => {
        sr.children[5 + i].innerHTML = b.toString()
    })

}
function updateMemoryTable() {
    const squareSize = Math.sqrt(PAGE_SIZE)
    const currentAddress = Number(memAddress.value)
    const clampedSize = currentAddress - (currentAddress % PAGE_SIZE)
    new Array(squareSize).fill(0).map((_, i) => {
        const address = clampedSize + (i) * squareSize
        const el = memoryAddresses.children[i + 1] as HTMLSpanElement
        el.innerText = address.toString(16).toUpperCase().padStart(4, "0")
    })
    memAddress.value = clampedSize.toString()
    if (!currentInterpreter) {
        return new Array(...memoryNumbers.children).forEach(c => {
            c.innerHTML = "FF"
        })
    }
    const data = currentInterpreter.wasm_read_memory_bytes(clampedSize, 16 * 16)
    const sp = currentInterpreter.wasm_get_sp() - clampedSize
    data.forEach((byte, i) => {
        const cell = memoryNumbers.children[i] as HTMLSpanElement
        const value = byte.toString(16).toUpperCase().padStart(2, "0")
        if (cell.innerText.toUpperCase() !== value) {
            cell.innerText = value
        }
    })
    for (const child of memoryNumbers.children) {
        child.classList.remove("sp")
    }

    memoryNumbers.children[sp]?.classList.add("sp")
}

memAddress.addEventListener("change", () => {
    updateMemoryTable()
})
memBefore.addEventListener("click", () => {
    memAddress.value = (Number(memAddress.value) - 16 * 16).toString()
    updateMemoryTable()
})
memAfter.addEventListener("click", () => {
    memAddress.value = (Number(memAddress.value) + 16 * 16).toString()
    updateMemoryTable()
})

function createMemoryTable(pageSize: number) {
    const squareSize = Math.sqrt(pageSize)
    const elements = new Array(pageSize).fill(0).map((_, i) => {
        const el = document.createElement('span')
        el.innerText = "FF"
        return el
    })
    memoryOffsets.innerHTML = ""
    const offsets = new Array(squareSize).fill(0).map((_, i) => {
        const el = document.createElement('span')
        el.innerText = i.toString(16).toUpperCase().padStart(2, "0")
        return el
    })
    memoryAddresses.innerHTML = "<span style='opacity:0;'>.</span>"

    const addresses = new Array(squareSize).fill(0).map((_, i) => {
        const el = document.createElement('span')
        const currentAddress = Number(memAddress.value)
        const address = currentAddress + (i) * squareSize
        el.innerText = address.toString(16).toUpperCase().padStart(2, "0")
        return el
    })
    memoryAddresses.append(...addresses)
    memoryOffsets.append(...offsets)
    memoryNumbers.innerHTML = ""
    memoryNumbers.append(...elements)
}

function updateRegisters(values: number[]) {
    const spans = registersWrapper.querySelectorAll("span")
    spans.forEach((s, i) => {
        s.innerHTML = `
        <div class='column'>
            <div>
                ${values[i].toString(16).toUpperCase().padStart(8, "0")}
            </div> 
            <div>${values[i]}</div>
        </div>
`
    })
}
const regs = [...new Array(8).fill(0).map((_, i) => `D${i}`), ...new Array(8).fill(0).map((_, i) => `A${i}`)]
function createRegisters() {
    registersWrapper.innerHTML = ""
    registersWrapper.append(...regs.map((r) => {
        const el = document.createElement("div")
        el.className = "register"
        el.innerText = `${r} `
        const value = document.createElement("span")
        value.innerText = "0"
        el.append(value)
        return el
    }))
}
createRegisters()
disableButtons(true)
createMemoryTable(PAGE_SIZE)