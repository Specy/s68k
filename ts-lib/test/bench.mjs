// Throughput of the interpreter through dist/, in millions of instructions a
// second: the gate of the decode cache (the asm-editor's
// docs/design/m68k-machine-plan.md, planning decision 2), which must not be
// slower than the instruction list it replaced.
//
// Two endless loops, one of 8 instructions and one of about 20,000, each run
// with no history and with a history of 100, through runWithLimit and through
// runWithBreakpoints with no breakpoint (the editor's run). Not part of
// `npm test`: run `npm run build-lib` and then `node test/bench.mjs`.
//
// Each case runs in slices of SLICE instructions for about SECONDS seconds, a
// few times, and the best round is printed, which is the least disturbed by
// whatever else the machine is doing.
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const dist = new URL('../dist/index.js', import.meta.url)
if (!existsSync(fileURLToPath(dist))) {
    console.error('dist/index.js is missing - run `npm run build-all` first.')
    process.exit(1)
}
const { S68k, Interpreter } = await import(dist)

const SLICE = 100_000
const SECONDS = 0.6
const ROUNDS = 5

// Eight instructions of the kinds a student's loop is made of, which assemble
// to five sizes: a moveq-sized addq, a word immediate, a register move, a
// memory store and load, a shift, a compare and a branch.
const SMALL = `
    org $1000
start:
    lea data,a0
loop:
    addq.l #1,d0
    add.w #3,d1
    move.l d0,d2
    move.w d1,(a0)
    move.w (a0),d3
    lsl.l #1,d2
    cmp.l d2,d0
    bra loop
data:
    ds.w 1
`

// About 20,000 instructions in a row and a jump back, cycling through the same
// eight kinds, so that a lookup by address has a long list to search.
function large(count) {
    const body = [
        '    addq.l #1,d0',
        '    add.w #3,d1',
        '    move.l d0,d2',
        '    move.w d1,(a0)',
        '    move.w (a0),d3',
        '    lsl.l #1,d2',
        '    cmp.l d2,d0',
        '    eor.l d0,d4',
    ]
    const lines = ['    org $1000', 'start:', '    lea data,a0', 'loop:']
    for (let i = 0; i < count - 1; i++) lines.push(body[i % body.length])
    lines.push('    jmp loop', 'data:', '    ds.w 1')
    return lines.join('\n')
}

function program(source) {
    const { diagnostics, program } = S68k.assemble(source)
    if (!program) throw new Error(JSON.stringify(diagnostics))
    return program
}

const RUNS = {
    runWithLimit: (interpreter) => interpreter.runWithLimit(SLICE),
    runWithBreakpoints: (interpreter) => interpreter.runWithBreakpoints([], SLICE, { skipBreakpointAtPc: false }),
}

function measure(compiled, options, run) {
    const interpreter = new Interpreter(compiled, options)
    let best = 0
    for (let round = 0; round < ROUNDS; round++) {
        const before = interpreter.getExecutedInstructions()
        const start = performance.now()
        let elapsed = 0
        while (elapsed < SECONDS * 1000) {
            try {
                run(interpreter)
            } catch (error) {
                if (error?.type !== 'ExecutionLimit') throw error
            }
            elapsed = performance.now() - start
        }
        const ran = interpreter.getExecutedInstructions() - before
        best = Math.max(best, ran / elapsed / 1000)
    }
    interpreter.dispose()
    return best
}

const cases = [
    ['8 instructions', program(SMALL)],
    ['20,000 instructions', program(large(20_000))],
]
const histories = [
    ['no history', { keep_history: false, history_size: 0 }],
    ['history 100', { keep_history: true, history_size: 100 }],
]

console.log(`M instructions/s, best of ${ROUNDS} rounds of ${SECONDS} s, slices of ${SLICE}`)
for (const [name, run] of Object.entries(RUNS)) {
    for (const [label, compiled] of cases) {
        for (const [history, options] of histories) {
            const rate = measure(compiled, options, run)
            console.log(`${name.padEnd(20)} ${label.padEnd(22)} ${history.padEnd(12)} ${rate.toFixed(1)}`)
        }
    }
}
for (const [, compiled] of cases) compiled.dispose()
