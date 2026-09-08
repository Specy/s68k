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
// A project of several files: include, incbin and the include chain
// ---------------------------------------------------------------------------

// The entry file holds the program; the numbers it adds up are in a data file
// and the routine that adds them is in another, both pasted in by `include`.
// The bytes of `sprite.bin` are a binary file, which is what `incbin` reads and
// what the program reads back out of memory.
const MAIN = [
    '    ORG $1000',
    'START:',
    '    LEA     VALUES,A0',
    '    MOVE.W  COUNT,D1',
    '    BSR     SUM             ; defined in lib/sum.x68',
    '    MOVE.L  D0,D2           ; the sum, out of the way of the terminate task',
    '    MOVE.B  SPRITE+2,D3     ; the third byte of the binary file',
    '    MOVE.B  #9,D0',
    '    TRAP    #15',
    "    INCLUDE 'data/values.x68'",
    "    INCLUDE 'lib/sum.x68'",
    'SPRITE:',
    "    INCBIN  'data/sprite.bin'",
    '    END     START'
].join('\n')

const VALUES = ['COUNT:  DC.W    4', 'VALUES: DC.W    1,2,3,4'].join('\n')

// A local label inside an included file: its scope is the global label above
// it, which is in the same file, and the whole of it is assembled where the
// `include` line is.
const SUM = [
    'SUM:',
    '    CLR.L   D0',
    '.loop:',
    '    ADD.W   (A0)+,D0',
    '    SUBQ.W  #1,D1',
    '    BNE     .loop',
    '    RTS'
].join('\n')

const project = S68k.assemble({
    files: {
        'main.x68': MAIN,
        'data/values.x68': VALUES,
        'lib/sum.x68': SUM,
        'data/sprite.bin': new Uint8Array([1, 2, 3, 4])
    },
    entry: 'main.x68'
})
assert.deepEqual(project.diagnostics, [], 'a project of four files assembles')

const symbols = project.program.getInfo().symbols
assert.equal(symbols['COUNT'].location.file, 'data/values.x68', 'one namespace, every file in it')
assert.equal(symbols['SUM'].location.file, 'lib/sum.x68')
assert.equal(symbols['SUM:loop'].location.file, 'lib/sum.x68', 'a local label keeps its scope')
assert.equal(symbols['SPRITE'].kind, 'label', 'the name on an incbin line is a label like any other')

const projectRun = new Interpreter(project.program)
projectRun.run()
assert.ok(projectRun.hasTerminated(), 'the program ran to the end')
const registers = projectRun.getCpuSnapshot()
assert.equal(registers.getRegisterValue(2, RegisterType.Data), 10, 'the included routine added the included data')
assert.equal(registers.getRegisterValue(3, RegisterType.Data), 3, 'the program read a byte of the binary file')
assert.equal(
    Array.from(projectRun.readMemoryBytes(symbols['SPRITE'].value, 4)).join(),
    '1,2,3,4',
    'incbin put the whole file in memory, untouched, with the label on its first byte'
)

// An instruction of an included file carries the `include` line it was reached
// through: its location says where it was written, the chain how it got there.
const summing = projectRun.getInstructionAt(symbols['SUM'].value)
assert.equal(summing.source.trim(), 'CLR.L   D0')
assert.equal(summing.location.file, 'lib/sum.x68')
assert.equal(summing.includeChain.length, 1, 'reached through one include line')
assert.equal(summing.includeChain[0].file, 'main.x68')
assert.equal(summing.includeChain[0].line, 10, 'the INCLUDE line of main.x68')
assert.deepEqual(
    projectRun.getInstructionAt(0x1000).includeChain,
    [],
    'an instruction of the entry file was reached through none'
)

// A breakpoint is a line of a file, and now of any file of the project.
const stopping = new Interpreter(project.program)
assert.equal(
    stopping.runWithBreakpoints([{file: 'lib/sum.x68', line: 3}]),
    InterpreterStatus.Running,
    'the run stops inside the included file'
)
assert.equal(stopping.getCurrentLocation().file, 'lib/sum.x68')
stopping.dispose()
projectRun.dispose()
project.program.dispose()

// A mistake in an included file is reported there, with the include line beside
// it: the location names the file the mistake is in, `related` how it got read.
const brokenProject = S68k.assemble({
    files: {
        'main.x68': ['    ORG $1000', "    INCLUDE 'lib/io.x68'"].join('\n'),
        'lib/io.x68': ['* the library', '    MOVE.W  D0,#1'].join('\n')
    },
    entry: 'main.x68'
})
assert.equal(brokenProject.program, undefined, 'an error anywhere in the project builds no program')
assert.equal(brokenProject.diagnostics.length, 1)
const [inIncluded] = brokenProject.diagnostics
assert.equal(inIncluded.severity, 'error')
assert.equal(inIncluded.code, 'invalid_addressing_mode')
assert.equal(inIncluded.location.file, 'lib/io.x68', 'reported where it is written')
assert.equal(inIncluded.location.line, 1)
assert.equal(inIncluded.related.length, 1)
assert.equal(inIncluded.related[0].location.file, 'main.x68', 'and how the file was reached')
assert.equal(inIncluded.related[0].location.line, 1)
assert.equal(typeof inIncluded.related[0].location.endColumn, 'number', 'a related location is a location')
assert.equal(inIncluded.related[0].message, 'included from `main.x68`')

// A file the project has not got names the closest one it has.
const missing = S68k.assemble({
    files: {'main.x68': "    INCLUDE 'io.x68'\n", 'lib/io.x68': '    NOP\n'},
    entry: 'main.x68'
})
assert.equal(missing.program, undefined)
assert.equal(missing.diagnostics.length, 1)
const [notFound] = missing.diagnostics
assert.equal(notFound.severity, 'error')
assert.equal(notFound.code, 'unreadable_file')
assert.equal(notFound.message, 'there is no file named `io.x68` in this project')
assert.equal(notFound.hint, 'did you mean `lib/io.x68`?')
assert.equal(notFound.location.file, 'main.x68')
assert.equal(notFound.location.line, 0)
assert.equal(notFound.location.column, 12, 'it points at the file name')
assert.equal(notFound.location.endColumn, 20)

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
