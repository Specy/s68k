//! The tests that are not beside the code they test: the golden fixtures of the
//! corpus, one case per Diagnostic, and the Interpreter's own programs.

mod assembled_as;
mod bus_wrap;
mod conformance;
mod corpus;
mod cycles;
mod decoded_instructions;
mod diagnostics;
#[cfg(any(feature = "musashi", feature = "moira"))]
mod differential;
mod encoding;
mod exceptions;
mod history;
mod include;
mod instruction_semantics;
mod interrupts;
mod m68010;
mod m68020;
mod m68020_instructions;
mod m68020_supervisor;
mod memory_map;
mod mutations;
mod pokes;
mod self_modifying_code;
mod test;
mod trap_tasks;
mod watched_memory;
