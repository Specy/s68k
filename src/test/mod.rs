//! The tests that are not beside the code they test: the golden fixtures of the
//! corpus, one case per Diagnostic, and the Interpreter's own programs.

mod corpus;
mod decoded_instructions;
mod diagnostics;
mod encoding;
mod history;
mod include;
mod instruction_access;
mod mutations;
mod pokes;
mod test;
mod trap_tasks;
mod watched_memory;
