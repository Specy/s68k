import { S68k } from "../pkg/s68k" 


const run = document.getElementById("run-button")
const code = document.getElementById("code-text")
const errorWrapper = document.getElementById("error-wrapper")
code.value = localStorage.getItem("s68k_code") || ""
run.addEventListener("click", () => {
    const text = code.value
    const s68k = new S68k(text) 
    console.log(s68k)
    const errors = s68k.wasm_semantic_check()
    const lines = s68k.wasm_get_lexed_lines()
    console.log(lines)
    console.log(errors) 
    localStorage.setItem("s68k_code", text)
    errorWrapper.innerHTML = ""
    errors.forEach(e => {
        const error = document.createElement("div")
        error.className = "error"
        error.innerText = e.wasm_get_message()
        errorWrapper.appendChild(error)
    })
})