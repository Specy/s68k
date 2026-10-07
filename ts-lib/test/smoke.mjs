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

const { S68k, Interpreter, RegisterType, InterpreterStatus, Size } = await import(dist)

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
assert.deepEqual(
    assembly.program.getInstructionAddresses(),
    [0x1000, 0x1004, 0x1008, 0x100c, 0x1010, 0x1014],
    'instruction addresses are exposed without copying full instructions'
)
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

// SIMHALT yields to the host without terminating the interpreter. Calling a
// run or step method again resumes at the following instruction.
const pausingAssembly = S68k.assemble(`
    ORG $1000
    MOVE.L #7,D0
    SIMHALT
    MOVE.L #9,D0
`)
assert.deepEqual(pausingAssembly.diagnostics, [])
const pausing = new Interpreter(pausingAssembly.program)
assert.equal(pausing.run(), InterpreterStatus.Paused, 'SIMHALT pauses the run')
assert.equal(pausing.hasTerminated(), false, 'a paused program has not terminated')
assert.equal(pausing.getPc(), 0x1008, 'the PC points after SIMHALT')
assert.equal(
    pausing.getCpuSnapshot().getRegisterValue(0, RegisterType.Data),
    7,
    'the instruction after SIMHALT has not run'
)
assert.equal(pausing.step(), InterpreterStatus.Terminated, 'step resumes after SIMHALT')
assert.equal(pausing.getCpuSnapshot().getRegisterValue(0, RegisterType.Data), 9)
pausing.dispose()
pausingAssembly.program.dispose()

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

// The breakpoint the program counter is on is the caller's to skip: the default
// continues past it, `skipBreakpointAtPc: false` stops on it having run nothing,
// which is what the host does when it resumes after answering a trap.
assert.equal(
    located_run.runWithBreakpoints(
        [{file: 'lecture/one.x68', line: 6}],
        undefined,
        {skipBreakpointAtPc: false}
    ),
    InterpreterStatus.Running,
    'a run told not to skip stops on the breakpoint it starts on'
)
assert.equal(
    located_run.getNextInstruction().location.line,
    6,
    'and it has run nothing: the same instruction is still the next one'
)
assert.equal(
    located_run.runWithBreakpoints([{file: 'lecture/one.x68', line: 6}]),
    InterpreterStatus.Terminated,
    'the default continues past the breakpoint it is parked on'
)
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
assert.equal(notFound.message, 'There is no file named `io.x68` in this project.')
assert.equal(notFound.hint, 'Did you mean `lib/io.x68`?')
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


const D0_WRITER = {type: 'Data', value: 0}

// ---------------------------------------------------------------------------
// What a write reports: the value it replaced and the value it wrote
// ---------------------------------------------------------------------------

// Every write of every step carries both sides, and they cross as plain
// unsigned numbers: a 32 bit register of all ones is 4294967295, never -1 and
// never a string.
const writing = S68k.assemble(
    '    ORG $1000\n    MOVE.L #$FFFFFFFF,D0\n    MOVE.B #$01,D0\n    MOVE.W D0,$2000\n'
)
assert.deepEqual(writing.diagnostics, [])
const writer = new Interpreter(writing.program, {keep_history: true, history_size: 100})

writer.step()
const [longMove] = writer.getUndoHistory(1)
assert.equal(longMove.mutations[0].type, 'WriteRegister')
assert.equal(longMove.mutations[0].value.old, 0)
assert.equal(longMove.mutations[0].value.new, 0xffffffff, 'unsigned, and not -1')
assert.equal(typeof longMove.mutations[0].value.new, 'number', 'a number, not a string')

// A sized store changes part of the register and reports the whole of it, both
// before and after, which is what the panels draw.
writer.step()
const [byteMove] = writer.getUndoHistory(1)
assert.equal(byteMove.mutations[0].value.size, 'Byte', 'the width the write was made at')
assert.equal(byteMove.mutations[0].value.old, 0xffffffff, 'the whole register before')
assert.equal(byteMove.mutations[0].value.new, 0xffffff01, 'and the whole register after')
assert.equal(writer.getRegisterValue(D0_WRITER), 0xffffff01)

// A memory write reports the bytes it replaced and the bytes it left, at the
// width it was made: memory starts as $ff, so the word it replaced is $ffff.
writer.step()
const [wordStore] = writer.getUndoHistory(1)
assert.equal(wordStore.mutations[0].type, 'WriteMemory')
assert.equal(wordStore.mutations[0].value.address, 0x2000)
assert.equal(wordStore.mutations[0].value.size, 'Word')
assert.equal(wordStore.mutations[0].value.old, 0xffff)
assert.equal(wordStore.mutations[0].value.new, 0xff01)
assert.deepEqual(
    Array.from(writer.readMemoryBytes(0x2000, 3)),
    [0xff, 0x01, 0xff],
    'and nothing past that width'
)

// The old fields are all still there, on every write of every step.
for (const step of writer.getUndoHistory(100)) {
    for (const mutation of step.mutations) {
        if (!mutation.type.startsWith('Write')) continue
        assert.notEqual(mutation.value.old, undefined, `${mutation.type} keeps its old value`)
        assert.notEqual(mutation.value.new, undefined, `${mutation.type} reports what it wrote`)
        if (mutation.type === 'WriteRegister') {
            assert.notEqual(mutation.value.register, undefined)
            assert.notEqual(mutation.value.size, undefined)
        } else {
            assert.notEqual(mutation.value.address, undefined)
        }
    }
}
writer.dispose()
writing.program.dispose()

// ---------------------------------------------------------------------------
// Pokes: what the host writes between two instructions
// ---------------------------------------------------------------------------

// A poke is one step of the same history the instructions use: the setters
// journal into the open transaction, endPoke records one entry, and undo
// reverts it like anything else.
const poking = S68k.assemble('    ORG $1000\n    MOVE.L #1,D0\n    MOVE.L #2,D0\n')
assert.deepEqual(poking.diagnostics, [])
const poked = new Interpreter(poking.program, {keep_history: true, history_size: 100})
const D0 = {type: 'Data', value: 0}
const D1 = {type: 'Data', value: 1}

poked.step()
assert.equal(poked.getRegisterValue(D0), 1, 'the first instruction wrote D0')

// Outside a transaction a setter is direct and records nothing: undoing the
// instruction before it leaves the host's write where it is.
poked.setRegisterValue(D1, 0xbeef, Size.Long)
poked.writeMemoryBytes(0x2000, new Uint8Array([1, 2, 3, 4]))
poked.undo()
assert.equal(poked.getRegisterValue(D1), 0xbeef, 'a host register write outside a poke is direct')
assert.deepEqual(
    Array.from(poked.readMemoryBytes(0x2000, 4)),
    [1, 2, 3, 4],
    'and so is a host memory write'
)
poked.step()

const beforePokeId = poked.getLastStepId()
assert.throws(() => poked.endPoke(), 'ending a poke that was never begun throws')
poked.beginPoke()
assert.throws(() => poked.beginPoke(), 'a poke inside a poke throws')
poked.setRegisterValue(D0, 0x99, Size.Long)
poked.writeMemoryBytes(0x2000, new Uint8Array([9, 9, 9, 9]))
assert.equal(poked.endPoke(), true, 'a poke that wrote something records a step')

const [pokeStep, instructionStep] = poked.getUndoHistory(2)
assert.equal(pokeStep.kind, 'poke', 'the newest step is the poke')
assert.equal(instructionStep.kind, 'instruction', 'and the one before it an instruction')
assert.ok(pokeStep.id > beforePokeId, 'a poke takes a step id of its own')
assert.equal(poked.getLastStepId(), pokeStep.id, 'which getLastStepId moves past')
assert.equal(pokeStep.writes.length, 2, 'one entry per value written')
assert.equal(pokeStep.writes[0].type, 'register')
assert.equal(pokeStep.writes[0].name, 'd0', 'named as the editor spells it')
assert.equal(pokeStep.writes[0].old, 1)
assert.equal(pokeStep.writes[0].new, 0x99)
assert.equal(pokeStep.writes[1].type, 'memory')
assert.equal(pokeStep.writes[1].address, 0x2000)
assert.deepEqual(pokeStep.writes[1].old, [1, 2, 3, 4])
assert.deepEqual(pokeStep.writes[1].new, [9, 9, 9, 9])
assert.deepEqual(instructionStep.writes, [], 'an instruction carries no poke writes')
// a poke journals the same three write shapes, so its mutations carry new as well
assert.equal(pokeStep.mutations[0].type, 'WriteRegister')
assert.equal(pokeStep.mutations[0].value.old, 1)
assert.equal(pokeStep.mutations[0].value.new, 0x99)
assert.equal(pokeStep.mutations[1].type, 'WriteMemoryBytes')
assert.deepEqual(pokeStep.mutations[1].value.old, [1, 2, 3, 4])
assert.deepEqual(pokeStep.mutations[1].value.new, [9, 9, 9, 9])

// A poke that changed nothing is no step at all.
poked.beginPoke()
poked.setRegisterValue(D0, 0x99, Size.Long)
assert.equal(poked.endPoke(), false, 'writing what is already there records nothing')
assert.equal(poked.getLastStepId(), pokeStep.id, 'and takes no id')

// Poke, instruction, undo, undo: the instruction goes first, then the poke,
// and the state is exactly what it was before the poke.
poked.step()
assert.equal(poked.getRegisterValue(D0), 2, 'the second instruction ran')
assert.equal(poked.canUndo(), true)
assert.equal(poked.undo().kind, 'instruction', 'the instruction is reverted first')
assert.equal(poked.getRegisterValue(D0), 0x99, 'back to what the poke left')
const undonePoke = poked.undo()
assert.equal(undonePoke.kind, 'poke', 'and then the poke')
assert.equal(undonePoke.writes.length, 2, 'the undone step says what it put back')
assert.equal(poked.getRegisterValue(D0), 1, 'the register is back')
assert.deepEqual(
    Array.from(poked.readMemoryBytes(0x2000, 4)),
    [1, 2, 3, 4],
    'and so are the bytes'
)
poked.dispose()
poking.program.dispose()

// ---------------------------------------------------------------------------
// The text tasks: text out, what was typed in
// ---------------------------------------------------------------------------

// A display task hands over the text to display, formatted and decoded the
// way EASy68K does it; a read task is answered with the line or the key that
// was typed, which the interpreter reads itself.
const texting = S68k.assemble(`
    ORG $1000
START:
    MOVE.L  #255,D1
    MOVE.B  #16,D2
    MOVE.B  #15,D0          ; 255 in base 16
    TRAP    #15
    MOVE.L  #-5,D1
    MOVE.B  #-6,D2
    MOVE.B  #20,D0          ; -5 in a field of -6 columns, left justified
    TRAP    #15
    LEA     PRICE,A1
    MOVE.L  #-12,D1
    MOVE.B  #17,D0          ; the string, then the number
    TRAP    #15
    MOVE.B  #$80,D1
    MOVE.B  #6,D0           ; the character $80
    TRAP    #15
    LEA     BUFFER,A1
    MOVE.L  #$FFFFFFFF,D1
    MOVE.B  #2,D0           ; a line
    TRAP    #15
    MOVE.B  #4,D0           ; a number
    TRAP    #15
    MOVE.L  D1,D3
    MOVE.B  #5,D0           ; a key
    TRAP    #15
    SIMHALT
PRICE:  DC.B    'Price in €: ',0
BUFFER: DS.B    100
`)
assert.deepEqual(texting.diagnostics, [], 'a program writing `€` in a string assembles')
const priceAddress = texting.program.getSymbols()['PRICE'].value
const bufferAddress = texting.program.getSymbols()['BUFFER'].value
const texter = new Interpreter(texting.program)
assert.equal(
    texter.readMemoryBytes(priceAddress + 9, 1)[0],
    0x80,
    '`€` is stored as its Windows-1252 byte'
)

const shown = []
const display = (expected) => {
    assert.equal(texter.run(), InterpreterStatus.Interrupt)
    const interrupt = texter.getCurrentInterrupt()
    assert.equal(interrupt.type, expected)
    shown.push(interrupt.value)
    texter.answerInterrupt({type: interrupt.type})
}
display('DisplayNumberInBase')
display('DisplaySignedNumberInField')
display('DisplayStringAndNumber')
display('DisplayChar')
assert.deepEqual(shown, ['FF', '-5    ', 'Price in €: -12', '€'], 'text ready to display')

assert.equal(texter.run(), InterpreterStatus.Interrupt)
assert.equal(texter.getCurrentInterrupt().type, 'ReadKeyboardString')
// A bad answer throws and leaves the interrupt waiting: the interpreter stays usable.
assert.throws(() => texter.answerInterrupt({type: 'ReadKeyboardString', value: 42}), 'a line is a string')
assert.throws(() => texter.answerInterrupt({type: 'ReadNumber', value: '7'}), 'an answer for another task')
assert.equal(texter.getStatus(), InterpreterStatus.Interrupt, 'the interrupt still waits')
texter.answerInterrupt({type: 'ReadKeyboardString', value: 'é€→' + 'x'.repeat(100)})
const stored = Array.from(texter.readMemoryBytes(bufferAddress, 81))
assert.deepEqual(stored.slice(0, 3), [0xe9, 0x80, 0x3f], 'Windows-1252, `?` for a character with no byte')
assert.equal(stored[79], 0, 'at most 79 characters, then the NUL')
assert.equal(texter.getRegisterValue({type: 'Data', value: 1}), 79, 'the count in the whole of D1.L')

assert.equal(texter.run(), InterpreterStatus.Interrupt)
assert.equal(texter.getCurrentInterrupt().type, 'ReadNumber')
texter.answerInterrupt({type: 'ReadNumber', value: ' 12abc'})
assert.equal(texter.run(), InterpreterStatus.Interrupt)
assert.equal(texter.getCurrentInterrupt().type, 'ReadChar')
texter.answerInterrupt({type: 'ReadChar', value: '\n'})
assert.equal(texter.run(), InterpreterStatus.Paused)
assert.equal(texter.getRegisterValue({type: 'Data', value: 3}), 12, '`atoi` reads 12 from `12abc`')
assert.equal(
    texter.getRegisterValue({type: 'Data', value: 1}) & 0xff,
    0x0d,
    'Enter is $0D, as EASy68K stores it'
)
texter.dispose()
texting.program.dispose()

// A character Windows-1252 has no byte for cannot be stored.
const arrow = S68k.assemble("    DC.B    '→',0\n")
assert.equal(arrow.program, undefined)
assert.deepEqual(arrow.diagnostics.map((d) => d.code), ['character_above_latin1'])

// ---------------------------------------------------------------------------
// Files: tasks 50 to 59 on the host's file system
// ---------------------------------------------------------------------------

// A host for the file tasks: files in a Map, EASy68K's eight file numbers
// handed out lowest first, a file opened for reading only when it cannot be
// written. It does what the host's file system does and nothing else: the
// interpreter turns each outcome into EASy68K's result in D0.W.
class FakeFiles {
    constructor(files = {}, readOnly = []) {
        this.files = new Map(Object.entries(files).map(([path, text]) => [path, new TextEncoder().encode(text)]))
        this.readOnly = new Set(readOnly)
        this.open = new Map()
        this.seen = []
    }
    free() {
        for (let handle = 0; handle < 8; handle++) if (!this.open.has(handle)) return handle
        return undefined
    }
    answer(interrupt) {
        const {type, value} = interrupt
        this.seen.push(interrupt)
        switch (type) {
            case 'CloseAllFiles':
                this.open.clear()
                return {type, value: true}
            case 'OpenFile': {
                const handle = this.free()
                if (handle === undefined || !this.files.has(value)) return {type, value: null}
                const readOnly = this.readOnly.has(value)
                this.open.set(handle, {path: value, position: 0, writable: !readOnly})
                return {type, value: {handle, read_only: readOnly}}
            }
            case 'NewFile': {
                const handle = this.free()
                if (handle === undefined || this.readOnly.has(value)) return {type, value: null}
                this.files.set(value, new Uint8Array())
                this.open.set(handle, {path: value, position: 0, writable: true})
                return {type, value: handle}
            }
            case 'ReadFile': {
                const file = this.open.get(value.handle)
                if (!file) return {type, value: null}
                const bytes = this.files.get(file.path).slice(file.position, file.position + value.count)
                file.position += bytes.length
                return {type, value: bytes}
            }
            case 'WriteFile': {
                const file = this.open.get(value.handle)
                if (!file || !file.writable) return {type, value: false}
                const old = this.files.get(file.path)
                const next = new Uint8Array(Math.max(old.length, file.position + value.bytes.length))
                next.set(old)
                next.set(value.bytes, file.position)
                this.files.set(file.path, next)
                file.position += value.bytes.length
                return {type, value: true}
            }
            case 'PositionFile': {
                const file = this.open.get(value.handle)
                if (file) file.position = value.offset
                return {type, value: file !== undefined}
            }
            case 'CloseFile':
                return {type, value: this.open.delete(value)}
            case 'DeleteFile':
                return {type, value: this.files.delete(value)}
            case 'FileExists':
                return {
                    type,
                    value: !this.files.has(value) ? 'Missing' : this.readOnly.has(value) ? 'ReadOnly' : 'Writable'
                }
            default:
                throw new Error(`not a file task: ${type}`)
        }
    }
}

// Runs to the next pause or the end, answering every interrupt from the host.
function runWith(interpreter, host) {
    for (;;) {
        const status = interpreter.run()
        if (status !== InterpreterStatus.Interrupt) return status
        interpreter.answerInterrupt(host.answer(interpreter.getCurrentInterrupt()))
    }
}

const FILES = `
    ORG $1000
START:
    lea     results,a2
    lea     name,a1
    move.b  #52,d0          ; a new file
    trap    #15
    move.w  d0,(a2)+
    move.l  d1,d6           ; its number
    lea     text,a1
    move.l  #11,d2
    move.b  #54,d0          ; write 'hello world'
    trap    #15
    move.w  d0,(a2)+
    move.l  d6,d1
    move.l  #6,d2
    move.b  #55,d0          ; to position 6
    trap    #15
    move.w  d0,(a2)+
    lea     buffer,a1
    move.l  #20,d2
    move.b  #53,d0          ; read 20: there are 5
    trap    #15
    move.w  d0,(a2)+
    move.l  d2,d7
    move.l  #20,d2
    move.b  #53,d0          ; read at the end of the file
    trap    #15
    move.w  d0,(a2)+
    move.l  d2,d5
    move.b  #56,d0          ; close
    trap    #15
    move.w  d0,(a2)+
    move.b  #56,d0          ; close it again
    trap    #15
    move.w  d0,(a2)+
    lea     name,a1
    move.b  #59,d0          ; it exists
    trap    #15
    move.w  d0,(a2)+
    lea     locked,a1
    move.b  #51,d0          ; a file that cannot be written
    trap    #15
    move.w  d0,(a2)+
    move.l  d1,d4
    lea     text,a1
    move.l  #1,d2
    move.b  #54,d0          ; a write to it
    trap    #15
    move.w  d0,(a2)+
    lea     name,a1
    move.b  #57,d0          ; delete the new file
    trap    #15
    move.w  d0,(a2)+
    move.b  #59,d0          ; it is gone
    trap    #15
    move.w  d0,(a2)+
    move.l  #9,d1
    move.b  #56,d0          ; there is no file number 9
    trap    #15
    move.w  d0,(a2)+
    move.b  #50,d0          ; close every file
    trap    #15
    move.w  d0,(a2)+
    SIMHALT
name:    dc.b    'out\\hello.txt',0
locked:  dc.b    'locked.txt',0
text:    dc.b    'hello world'
buffer:  dcb.b   32,0
results: ds.w    16
    END     START
`
const filing = S68k.assemble(FILES)
assert.deepEqual(filing.diagnostics, [], 'the file program assembles')
const fileSymbols = filing.program.getSymbols()
const filer = new Interpreter(filing.program)
const host = new FakeFiles({'locked.txt': 'no'}, ['locked.txt'])
assert.equal(runWith(filer, host), InterpreterStatus.Paused)
const results = filer.readMemoryBytes(fileSymbols['results'].value, 2 * 14)
const words = Array.from({length: 14}, (_, i) => (results[2 * i] << 8) | results[2 * i + 1])
assert.deepEqual(
    words,
    [0, 0, 0, 0, 1, 0, 2, 0, 3, 2, 0, 2, 2, 0],
    'created, written, moved, read, end of file, closed, no longer open, there, read only, ' +
    'a write it refuses, deleted, gone, no such number, all closed'
)
assert.equal(host.seen[0].value, 'out/hello.txt', 'a path with `\\` written as `/`')
assert.ok(host.seen[1].value.bytes instanceof Uint8Array, 'a write carries a Uint8Array')
assert.equal(new TextDecoder().decode(host.seen[1].value.bytes), 'hello world')
assert.deepEqual(host.seen[2].value, {handle: 0, offset: 6})
const readData = D => filer.getRegisterValue({type: 'Data', value: D})
assert.equal(readData(7), 5, 'a short read is a success with D2.L the count read')
assert.equal(readData(5), 20, 'the end of the file leaves D2.L as it was')
assert.equal(readData(4), 0, 'the read-only file is number 0, free again once closed')
assert.equal(
    new TextDecoder().decode(filer.readMemoryBytes(fileSymbols['buffer'].value, 6)),
    'world\0',
    'what was read went to (A1)'
)
assert.equal(
    host.seen.filter((interrupt) => interrupt.type === 'CloseFile').length,
    2,
    'the file number 9 never reached the host'
)
assert.equal(filer.getUndoHistory(1)[0].kind, 'instruction')
filer.dispose()
filing.program.dispose()

// Eight files at most: the ninth open is 2 in D0.W with -1 in D1.L.
const crowding = S68k.assemble(`
    ORG $1000
START:
    lea     numbers,a2
    moveq   #8,d7
loop:
    lea     name,a1
    move.b  #51,d0
    trap    #15
    move.l  d1,(a2)+
    move.w  d0,(a2)+
    dbra    d7,loop
    SIMHALT
name:    dc.b    'in.txt',0
numbers: ds.w    27
    END     START
`)
assert.deepEqual(crowding.diagnostics, [])
const crowded = new Interpreter(crowding.program)
assert.equal(runWith(crowded, new FakeFiles({'in.txt': 'x'})), InterpreterStatus.Paused)
const numbers = crowded.readMemoryBytes(crowding.program.getSymbols()['numbers'].value, 54)
const opened = Array.from({length: 9}, (_, i) => {
    const at = 6 * i
    const handle = ((numbers[at] << 24) | (numbers[at + 1] << 16) | (numbers[at + 2] << 8) | numbers[at + 3]) | 0
    return [handle, (numbers[at + 4] << 8) | numbers[at + 5]]
})
assert.deepEqual(opened, [[0, 0], [1, 0], [2, 0], [3, 0], [4, 0], [5, 0], [6, 0], [7, 0], [-1, 2]])
crowded.dispose()
crowding.program.dispose()

// Undo takes back what a file read wrote, with the registers it set.
const rereading = S68k.assemble(`
    ORG $1000
START:
    lea     buffer,a1
    move.l  #0,d1
    move.l  #4,d2
    move.b  #53,d0
    trap    #15
    SIMHALT
buffer:  dc.b    'zzzz'
    END     START
`)
const reread = new Interpreter(rereading.program)
assert.equal(reread.run(), InterpreterStatus.Interrupt)
const asked = reread.getCurrentInterrupt()
assert.deepEqual(asked, {type: 'ReadFile', value: {handle: 0, count: 4}})
// more bytes than were asked for is refused, as a typed error, and the read still waits
assert.throws(
    () => reread.answerInterrupt({type: 'ReadFile', value: new Uint8Array(5)}),
    (error) => error.type === 'InvalidAnswer' && error.value.interrupt === 'ReadFile'
)
// an array of numbers is taken as well as a Uint8Array
reread.answerInterrupt({type: 'ReadFile', value: [97, 98, 99]})
const bufferAt = rereading.program.getSymbols()['buffer'].value
assert.equal(new TextDecoder().decode(reread.readMemoryBytes(bufferAt, 4)), 'abcz')
const [readStep] = reread.getUndoHistory(1)
assert.ok(
    readStep.mutations.some((mutation) => mutation.type === 'WriteMemoryBytes'),
    'the bytes are journaled with the trap'
)
reread.undo()
assert.equal(new TextDecoder().decode(reread.readMemoryBytes(bufferAt, 4)), 'zzzz', 'undo puts them back')
assert.equal(reread.getStatus(), InterpreterStatus.Running)
assert.equal(reread.getCurrentInterrupt(), null, 'and the read is no longer waiting')
assert.equal(reread.run(), InterpreterStatus.Interrupt, 'the trap runs again')
reread.answerInterrupt({type: 'ReadFile', value: new Uint8Array()})
assert.equal(reread.getRegisterValue({type: 'Data', value: 0}) & 0xffff, 1, 'nothing read is the end of the file')
reread.dispose()
rereading.program.dispose()

// The file dialog writes the path chosen to (A3).
const choosing = S68k.assemble(`
    ORG $1000
START:
    move.l  #1,d1
    lea     title,a1
    lea     filter,a2
    lea     path,a3
    move.b  #58,d0
    trap    #15
    SIMHALT
title:  dc.b    'Save as',0
filter: dc.b    '*.txt',0
path:   dc.b    'old.txt',0
        dcb.b   300,$55
    END     START
`)
const chooser = new Interpreter(choosing.program)
assert.equal(chooser.run(), InterpreterStatus.Interrupt)
assert.deepEqual(chooser.getCurrentInterrupt(), {
    type: 'FileDialog',
    value: {mode: 'Save', title: 'Save as', filter: '*.txt', path: 'old.txt'}
})
chooser.answerInterrupt({type: 'FileDialog', value: 'scores/new.txt'})
const pathAt = choosing.program.getSymbols()['path'].value
const chosen = chooser.readMemoryBytes(pathAt, 257)
assert.equal(new TextDecoder().decode(chosen.slice(0, 14)), 'scores/new.txt')
assert.ok(chosen.slice(14, 256).every((byte) => byte === 0), 'NULs to 256 bytes')
assert.equal(chosen[256], 0x55)
assert.equal(chooser.getRegisterValue({type: 'Data', value: 1}), 1, 'D1.L is 1 for a file chosen')
chooser.dispose()
choosing.program.dispose()

// ---------------------------------------------------------------------------
// The input settings of tasks 12 and 16, and undo
// ---------------------------------------------------------------------------

const setting = S68k.assemble(`
    ORG $1000
START:
    move.b  #0,d1
    move.b  #12,d0          ; echo off
    trap    #15
    move.b  #2,d1
    move.b  #16,d0          ; no line feed after Enter
    trap    #15
    SIMHALT
    END     START
`)
const setter = new Interpreter(setting.program)
assert.deepEqual(setter.getInputSettings(), {echo: true, prompt: true, line_feed: true}, 'all on at the start')
assert.equal(setter.run(), InterpreterStatus.Paused, 'the settings raise no interrupt')
assert.deepEqual(setter.getInputSettings(), {echo: false, prompt: true, line_feed: false})
setter.undo() // the SIMHALT
const [settingStep] = setter.getUndoHistory(1)
assert.deepEqual(settingStep.mutations, [{
    type: 'SetInputSettings',
    value: {
        old: {echo: false, prompt: true, line_feed: true},
        new: {echo: false, prompt: true, line_feed: false}
    }
}])
setter.undo()
assert.deepEqual(setter.getInputSettings(), {echo: false, prompt: true, line_feed: true}, 'undo puts it back')
setter.undo()
setter.undo()
setter.undo()
assert.deepEqual(setter.getInputSettings(), {echo: true, prompt: true, line_feed: true})
setter.dispose()
setting.program.dispose()

// ---------------------------------------------------------------------------
// Errors are typed, a failing task ends the program, and nothing panics
// ---------------------------------------------------------------------------

const failing = S68k.assemble(`
    ORG $1000
START:
    move.b  #30,d0          ; the cycle counter
    trap    #15
    nop
    END     START
`)
const failer = new Interpreter(failing.program)
assert.throws(
    () => failer.run(),
    (error) => error.type === 'UnsupportedTrapTask' && error.value.task === 30
)
assert.equal(failer.getStatus(), InterpreterStatus.TerminatedWithException)
assert.ok(failer.hasTerminated())
assert.deepEqual(failer.getTermination(), {type: 'Exception', value: {type: 'UnsupportedTrapTask', value: {task: 30}}})
failer.undo()
assert.equal(failer.getStatus(), InterpreterStatus.Running, 'undo brings the failed trap back')
assert.equal(failer.getTermination(), null)
assert.throws(() => failer.answerInterrupt({type: 'Terminate'}), (error) => error.type === 'NoPendingInterrupt')
// a value that would index past the eight registers is refused, and the module stays usable
assert.throws(() => failer.getRegisterValue({type: 'Data', value: 9}), (error) => error.type === 'InvalidArgument')
assert.throws(() => failer.getCpuSnapshot().getRegister(8, RegisterType.Address), (error) => error.type === 'InvalidArgument')
assert.throws(() => failer.runWithBreakpoints([{line: 'x'}]), (error) => error.type === 'InvalidArgument')
assert.deepEqual(Array.from(failer.readMemoryBytes(0x10, 0xffffffff)), [], 'a length near 4 GB reads nothing')
assert.equal(failer.getRegisterValue({type: 'Data', value: 0}) & 0xff, 30, 'and it still answers')
failer.dispose()
failing.program.dispose()

const ending = S68k.assemble('    ORG $1000\nSTART:\n    MOVE.B #9,D0\n    TRAP #15\n    NOP\n    END START\n')
const ender = new Interpreter(ending.program)
assert.equal(ender.run(), InterpreterStatus.Terminated)
assert.deepEqual(ender.getTermination(), {type: 'TerminateTask'})
ender.dispose()
ending.program.dispose()

// Sound: the arguments, for a host with something to play them on.
const sounding = S68k.assemble(`
    ORG $1000
START:
    lea     wav,a1
    move.b  #4,d1
    move.b  #71,d0
    trap    #15
    SIMHALT
wav: dc.b 'sounds\\ding.wav',0
    END     START
`)
const sounder = new Interpreter(sounding.program)
assert.equal(sounder.run(), InterpreterStatus.Interrupt)
assert.deepEqual(sounder.getCurrentInterrupt(), {type: 'LoadSound', value: {path: 'sounds/ding.wav', index: 4}})
assert.throws(() => sounder.answerInterrupt({type: 'PlaySound', value: true}), (error) => error.type === 'InvalidAnswer')
sounder.answerInterrupt({type: 'LoadSound'})
assert.equal(sounder.run(), InterpreterStatus.Paused)
sounder.dispose()
sounding.program.dispose()

interpreter.dispose()
assembly.program.dispose()

console.log(`ok - ran ${steps} instructions, D1 = ${cpu.getRegisterValue(1, RegisterType.Data)}`)
