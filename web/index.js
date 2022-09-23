import { WASM_S68k } from "s68k" 


const run = document.getElementById("run-button")
const code = document.getElementById("code-text")

run.addEventListener("click", () => {
    const text = code.value
    const s68k = new WASM_S68k(text)
    const errors = s68k.wasm_semantic_check()
    const lines = s68k.wasm_get_lexed_lines()
    console.log(lines)
    console.log(errors) 
})