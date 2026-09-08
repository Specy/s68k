// Smoke test for the published artifact: assembles, diagnoses and runs small
// M68K programs through dist/, so it covers the whole Rust -> WebAssembly ->
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

const { S68k, Interpreter, RegisterType, InterpreterStatus } = await import(dist)

// ---------------------------------------------------------------------------
// Assembling and running
// ---------------------------------------------------------------------------

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

const assembly = S68k.assemble(SOURCE)
assert.deepEqual(assembly.diagnostics, [], 'the program should assemble with nothing to report')
assert.notEqual(assembly.program, undefined, 'a program with no errors comes with a program')
assert.equal(assembly.program.getEntryPoint(), 0x1000, 'START is the entry point')
assert.equal(assembly.program.getInstructionCount(), 6)
assert.equal(assembly.program.getSymbols()['START'].value, 0x1000)
assert.equal(assembly.program.getSymbols()['START'].kind, 'label')
assert.equal(assembly.program.getSymbols()['START'].location.line, 3, 'START is on the fourth line')

const interpreter = new Interpreter(assembly.program)
let steps = 0
let status = InterpreterStatus.Running
while (status !== InterpreterStatus.Terminated && steps < 1000) {
    status = interpreter.step()
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

// ---------------------------------------------------------------------------
// The status register
// ---------------------------------------------------------------------------

// $2700 to start with, as in EASy68K: supervisor, interrupt mask 7, no
// condition code set. The high byte is stored and has no effect.
const statusRegister = S68k.assemble(`
    ORG $1000
START:
    MOVE.W  #$2705, SR
    MOVE.W  SR, D0
    ANDI.B  #$00, CCR
`)
assert.deepEqual(statusRegister.diagnostics, [], 'the status register instructions assemble')
const withStatus = new Interpreter(statusRegister.program)
assert.equal(withStatus.getSr(), 0x2700, 'a program starts at $2700')
withStatus.step()
assert.equal(withStatus.getSr(), 0x2705, 'MOVE to SR writes the whole register')
withStatus.step()
assert.equal(
    withStatus.getCpuSnapshot().getRegisterValue(0, RegisterType.Data) & 0xffff,
    0x2705,
    'MOVE from SR reads it back'
)
withStatus.step()
assert.equal(withStatus.getSr(), 0x2700, 'ANDI to CCR clears the condition codes only')
const undone = withStatus.undo()
assert.equal(typeof undone.old_sr, 'number', 'an undone step carries the status register')
assert.equal(withStatus.getSr(), 0x2705, 'and undo puts it back')
withStatus.dispose()
statusRegister.program.dispose()

// ---------------------------------------------------------------------------
// Locations: a breakpoint, and where the program counter is
// ---------------------------------------------------------------------------

const located = S68k.assemble({files: {'lecture/one.x68': SOURCE}, entry: 'lecture/one.x68'})
assert.deepEqual(located.diagnostics, [], 'the same source assembles under any file name')
const located_run = new Interpreter(located.program)
// The MOVE.W on line 6, counting the leading newline as line 0.
assert.equal(
    located_run.runWithBreakpoints([{file: 'lecture/one.x68', line: 6}]),
    InterpreterStatus.Running,
    'the run should stop on the breakpoint, not terminate'
)
const at = located_run.getCurrentLocation()
assert.equal(at.file, 'lecture/one.x68', 'a location names the file it came from')
assert.equal(at.line, 6)
assert.equal(typeof at.endColumn, 'number', 'a location carries a column range')
const next = located_run.getNextInstruction()
assert.equal(next.location.line, 6, 'the instruction about to run is the one stopped on')
assert.equal(next.source.trim(), 'MOVE.W  #-2, D2')
assert.equal(typeof next.address, 'number')
assert.equal(next.size, 4)
located_run.dispose()
located.program.dispose()

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

const broken = S68k.assemble('    ORG $1000\n    MOVE.W  D0,#1\n')
assert.equal(broken.program, undefined, 'a program with an error builds no program')
assert.equal(broken.diagnostics.length, 1, `expected one diagnostic, got ${JSON.stringify(broken.diagnostics)}`)
const [diagnostic] = broken.diagnostics
assert.equal(diagnostic.severity, 'error')
assert.equal(diagnostic.code, 'invalid_addressing_mode')
assert.equal(typeof diagnostic.message, 'string')
assert.ok(diagnostic.message.length > 0, 'a diagnostic says what is wrong')
assert.equal(diagnostic.location.file, 'main.m68k', 'a bare string is filed under main.m68k')
assert.equal(diagnostic.location.line, 1, 'the second line, counting from 0')
assert.equal(diagnostic.location.column, 15, 'the immediate operand starts in column 15')
assert.equal(diagnostic.location.endColumn, 17)
assert.deepEqual(diagnostic.related, [])

// A suggestion still builds a program: EASy68K's markerless comment field is
// read as a comment and said so once.
const bare = S68k.assemble('    ORG $1000\n    MOVE.W  D0,D1 copy it\n')
assert.notEqual(bare.program, undefined, 'a suggestion does not stop the program from being built')
assert.equal(bare.diagnostics.length, 1)
assert.equal(bare.diagnostics[0].severity, 'suggestion')
assert.equal(bare.diagnostics[0].code, 'bare_comment')
assert.equal(typeof bare.diagnostics[0].hint, 'string', 'this one says what to do about it')
bare.program.dispose()

// ---------------------------------------------------------------------------
// parseLine
// ---------------------------------------------------------------------------

const line = S68k.parseLine('start:  move.w #$10,(a0)+  ; go')
assert.equal(line.kind, 'instruction')
assert.equal(line.label.name, 'start')
assert.equal(line.label.colon, true)
assert.equal(line.operation.name, 'move')
assert.equal(line.operation.size, 'word')
assert.equal(line.operation.operands.length, 2)
assert.equal(line.operation.operands[0].mode, 'immediate')
assert.equal(line.operation.operands[0].text, '#$10')
assert.equal(line.operation.operands[1].mode, 'postincrement')
assert.equal(line.operation.operands[1].description, 'a postincrement operand')
assert.deepEqual(line.operation.operands[1].span, {start: 20, end: 25})
assert.equal(line.comment.kind, 'explicit')
assert.equal(line.comment.text, '; go')

assert.equal(S68k.parseLine('    org $1000').kind, 'directive')
assert.equal(S68k.parseLine('').kind, 'blank')
assert.equal(S68k.parseLine('* a comment').kind, 'comment')
assert.equal(S68k.parseLine('    mvoe.w d0,d1').kind, 'unknown', 'parseLine never throws')

// ---------------------------------------------------------------------------
// Undo, and the identity of a step
// ---------------------------------------------------------------------------

// Identical loop iterations still have distinct IDs after the bounded history fills,
// and executing again after undo must not reuse the abandoned instruction's ID.
const looping = S68k.assemble('    ORG $1000\nloop: nop\n    bra loop')
const loop = new Interpreter(looping.program, {keep_history: true, history_size: 2})
for (let i = 0; i < 10; i++) loop.step()
const history = loop.getUndoHistory(2)
assert.equal(history.length, 2)
assert.ok(history[0].id > history[1].id)
assert.equal(history[0].location.line, 2, 'an undo step carries the location of the line that ran')
assert.equal(loop.getLastStepId(), history[0].id)
assert.equal(loop.undo().id, history[0].id)
assert.equal(loop.getLastStepId(), history[1].id)
loop.step()
assert.ok(loop.getLastStepId() > history[0].id)
loop.dispose()
looping.program.dispose()

interpreter.dispose()
assembly.program.dispose()

console.log(`ok - ran ${steps} instructions, D1 = ${cpu.getRegisterValue(1, RegisterType.Data)}`)
