//! What a line assembled as, when the Assembler chose something the line did
//! not write: the "assembled as" of the editor's hover
//! ([`Program::assembled_as`](crate::assembler::program::Program::assembled_as)).

use crate::interpreter::Interpreter;

/// What each instruction line of `code`, in order, assembled as, through the
/// Interpreter, which names addresses by their Labels.
fn assembled_as(code: &str) -> Vec<Option<String>> {
    let program = crate::assembler::assemble_source(code)
        .program
        .expect("the code assembles");
    let interpreter = Interpreter::new(program, None);
    interpreter
        .get_program()
        .instructions()
        .iter()
        .map(|instruction| interpreter.get_assembled_as(instruction))
        .collect()
}

fn one(line: &str) -> Option<String> {
    assembled_as(&format!("    org $1000\nstart:\n{line}\n"))
        .into_iter()
        .next()
        .expect("one instruction")
}

#[test]
fn another_mnemonic_is_shown() {
    assert_eq!(one("    add.l #1,d0"), Some("ADDQ.L #1,D0".to_string()));
    assert_eq!(one("    move.l #5,d0"), Some("MOVEQ #5,D0".to_string()));
    assert_eq!(one("    sub.w #8,d3"), Some("SUBQ.W #8,D3".to_string()));
    assert_eq!(one("    add.w #100,d0"), Some("ADDI.W #$64,D0".to_string()));
    assert_eq!(one("    move.l d0,a0"), Some("MOVEA.L D0,A0".to_string()));
    assert_eq!(one("    add.l d1,a2"), Some("ADDA.L D1,A2".to_string()));
}

#[test]
fn what_the_line_wrote_is_not_shown() {
    assert_eq!(one("    move.w d0,d1"), None);
    assert_eq!(one("    move d0,d1"), None, "the default size is no choice");
    assert_eq!(one("    addq.l #1,d0"), None);
    assert_eq!(one("    moveq #5,d0"), None);
    assert_eq!(one("    MOVE.L D0,(A1)+"), None, "in any case");
    assert_eq!(one("    lea $1000.w,a0"), None);
    assert_eq!(one("    lea $12345678,a0"), None, "a long address is the default form");
    assert_eq!(one("    move.l 4(a0,d1.w),d0"), None, "an index's .w is not an address");
    assert_eq!(one("    trap #15"), None);
    assert_eq!(one("    simhalt"), None);
}

#[test]
fn an_alias_is_the_same_mnemonic() {
    let lines = assembled_as(
        "    org $1000
start:
loop:
    dbra d0,loop
    bhs.s loop
    blo.s loop
    shs d0
",
    );
    assert_eq!(lines, vec![None, None, None, None]);
}

#[test]
fn a_branch_size_the_line_did_not_write_is_shown() {
    let lines = assembled_as(
        "    org $1000
start:
loop:
    bra loop
    beq ahead
    bsr.s loop
    bra.b loop
    bne.w loop
    bra.l loop
ahead:
    nop
",
    );
    assert_eq!(
        lines,
        vec![
            Some("BRA.S loop".to_string()),
            Some("BEQ.W ahead".to_string()),
            None,
            None,
            None,
            Some("BRA.W loop".to_string()),
            None,
        ],
        "the Label is the first name of the address in byte order, `loop` \
         before `start`; `.b` is `.s`, and the 68000 has no `.l`"
    );
}

#[test]
fn an_address_in_a_word_the_line_did_not_write_is_shown() {
    assert_eq!(one("    lea $2000,a0"), Some("LEA $2000.W,A0".to_string()));
    assert_eq!(one("    jsr start"), Some("JSR start.W".to_string()));
    assert_eq!(
        one("    move.w $3000,$4000.w"),
        Some("MOVE.W $3000.W,$4000.W".to_string()),
        "one written, one chosen"
    );
}
