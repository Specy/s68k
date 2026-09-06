// Smoke test for the published artifact: assembles and runs a small M68K
// program through dist/, so it covers the whole Rust -> WebAssembly ->
// TypeScript chain rather than just type-checking it. Run `npm run build-all`
// first (or `npm run build-lib`, if the wasm package is already built).
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const dist = new URL('../dist/index.js', import.meta.url)
if (!existsSync(fileURLToPath(dist))) {
    console.error('dist/index.js is missing - run `npm run build-all` first.')
    process.exit(1)
}

const { S68k, RegisterType, InterpreterStatus } = await import(dist)

// TRAP #15 with 9 in D0 is EASy68K's "terminate", so the program ends by
// asking the host to stop rather than by running off the end of its code.
const SOURCE = `
    ORG $1000

START:
    MOVE.L  #5, D1
    ADD.L   #3, D1
    MOVE.W  #-2, D2
    EXT.L   D2
    MOVE.B  #9, D0
    TRAP    #15
`

const compiled = S68k.compile(SOURCE)
assert.equal(compiled.ok, true, `compilation failed: ${JSON.stringify(compiled.errors ?? [])}`)

const interpreter = compiled.interpreter
let steps = 0
let status = InterpreterStatus.Running
while (status !== InterpreterStatus.Terminated && steps < 1000) {
    status = interpreter.stepGetStatus()
    steps++
    assert.notEqual(
        status,
        InterpreterStatus.TerminatedWithException,
        'the program terminated with an exception'
    )
}

assert.equal(status, InterpreterStatus.Terminated, `program did not terminate in ${steps} steps`)

const cpu = interpreter.getCpuSnapshot()
assert.equal(cpu.getRegisterValue(1, RegisterType.Data), 8, 'D1 should hold 5 + 3')
// EXT.L sign-extends the word -2 across the whole register.
assert.equal(cpu.getRegisterValue(2, RegisterType.Data) | 0, -2, 'D2 should be sign-extended')

// Undo is part of the published surface and touches the history buffer that
// the interpreter options allocate, so it is worth one assertion here.
assert.equal(typeof interpreter.undo, 'function', 'undo should be exposed')

console.log(`ok - ran ${steps} instructions, D1 = ${cpu.getRegisterValue(1, RegisterType.Data)}`)
