//! The tests that are not beside the code they test: the golden fixtures of the
//! corpus, one case per Diagnostic, and the Interpreter's own programs.

mod assembled_as;
mod conformance;
mod corpus;
mod decoded_instructions;
mod diagnostics;
mod encoding;
mod exceptions;
mod history;
mod include;
mod instruction_semantics;
mod interrupts;
mod memory_map;
mod mutations;
mod pokes;
mod self_modifying_code;
mod test;
mod trap_tasks;
mod watched_memory;
