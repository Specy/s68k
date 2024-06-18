use console::Term;

use crate::compiler::Compiler;
use crate::instructions::{Interrupt, InterruptResult};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus};
use crate::S68k;

//TODO add better tests for all cases and if i find bugs etc
#[cfg(test)]
mod tests {
    use crate::test::test::lex_and_run;


    #[test]
    fn equ_substitution(){
        lex_and_run("ten equ #10
register_1 equ d1
	move.l ten, register_1
");
    }

    #[test]
    fn test_complex_code() {
        lex_and_run("ORG    $1000
    length: dc.w 20
    arr: dc.w 11, 71, 26, 44, 45, 65, 86, 10, 36, 26, 87, 86, 99, 48, 70, 89, 68, 92, 47, 80
START:
    * sort the array
    MOVE.l #arr, -(sp)
    MOVE.w length,-(sp)
    bsr sort_array
    add.l #6, sp
    * print the sorted array
    MOVE.l #arr, -(sp)
    MOVE.w length,-(sp)
    bsr print_array
    bra end
sa_off_length equ 44
sa_off_array_pointer equ 46
sort_array:
    MOVE.w d0, -(sp)
    MOVE.l d1, -(sp)
    MOVE.l d2, -(sp)
    MOVE.l d3, -(sp)
    MOVE.w d4, -(sp)
    MOVE.l d5, -(sp)
    MOVE.l a0, -(sp)
    MOVE.l a1, -(sp)
    MOVE.l a2, -(sp)
    MOVE.l d7, -(sp)
    MOVE.l a6, -(sp)
    MOVE.w sa_off_length(sp), d7
    muls #2, d7
    MOVE.l sa_off_array_pointer(sp), a6
    * uses:
* d0 (w) = i
* d1 (l) = end
* d2 (l) = j
* d3 (l) = diff
* d4 (w) = tmp
* d5 (l) = swaps
* a0 (l) = toSort
* a1 (l) = beforeElement
* a2 (l) = currentElement
* d7 (w) = parameter length
* a6 (l) = parameter array pointer
* total offset = 34
    MOVE.w #0, d0 *i = 0
    MOVE.w #0, d5 *swaps = 0
for_i_start:
    cmp.w d7, d0
    bge for_i_end *if(i >= length) goto for_i_end
    MOVE.w #2, d2 * j = 1
    MOVE.w d7, d1 * end = length
    sub.w d0, d1  * end -= i
    for_j_start:
        cmp d1, d2
        bge for_j_end   * if(j >= end) goto_for_j_end
        MOVE.l a6, a1   * beforeElement = array pointer
        add.l d2, a1    * beforeElement += j
        sub.l #2, a1    * beforeElement -= 1
        MOVE.l a6, a2   * currentElement = array pointer
        add.l d2, a2    * currentElement += j
        MOVE.w (a1), d3 * diff = *beforeElement
        sub.w (a2), d3  * diff -= *currentElement
        tst d3
        blt if_smaller  * if(diff < 0)
            MOVE.w (a1), d4 * tmp = *beforeElement
            add.l #1, d5 * swaps++
            MOVE.w (a2), (a1) * *beforeElement = *currentElement
            MOVE.w d4, (a2) * *currentElement = tmp
        if_smaller:
        add.l #2, d2 * j++
        bra for_j_start
    for_j_end:
    add.l #2, d0 * i++
    bra for_i_start
for_i_end:
    MOVE.l (sp)+, a6
    MOVE.l (sp)+, d7
    MOVE.l (sp)+, a2
    MOVE.l (sp)+, a1
    MOVE.l (sp)+, a0
    MOVE.l (sp)+, d5
    MOVE.w (sp)+, d4
    MOVE.l (sp)+, d3
    MOVE.l (sp)+, d2
    MOVE.l (sp)+, d1
    MOVE.w (sp)+, d0
    rts
* uses:  d0(l) d1(w) d7(w) a2(l) register offset = 12
* total offset = register offset + return = 16
pa_off_length equ 16
pa_off_array_pointer equ 18
print_array:
    MOVE.l d0, -(sp)
    MOVE.w d1, -(sp)
    MOVE.w d7, -(sp)
    MOVE.l a2, -(sp)
    MOVE.w pa_off_length(sp), d7
    MOVE.l pa_off_array_pointer(sp), a2
for_start:
    MOVE.l #3, d0
    MOVE.w (a2), d1
    add.l #2, a2
    trap #15
    MOVE.l #6, d0
    MOVE.l #',', d1
    trap #15
    tst d7
    sub.l #1, d7
    bgt for_start
for_end:
    MOVE.l (sp)+,a2
    MOVE.w (sp)+,d7
    MOVE.w (sp)+,d1
    MOVE.l (sp)+,d0
    rts
end:");
    }
}

const TEST_LIMIT: usize = 3000000;

fn lex_and_run(code: &str) -> Interpreter {
    let options = InterpreterOptions {
        keep_history: false,
        ..Default::default()
    };
    let s68k = S68k::new(code.to_string());
    let errors = s68k.semantic_check();
    if !errors.is_empty() {
        panic!("Code did not pass semantic check: {:#?}", errors);
    }
    let compiled = s68k.compile().expect("To compile correctly");
    let mut interpreter = s68k.create_interpreter(compiled, 0xFFFFFF, Some(options));
    while !interpreter.has_terminated() {
        let status = interpreter.run().unwrap();
        match status {
            InterpreterStatus::Interrupt => {
                let interrupt = interpreter.get_current_interrupt().unwrap();
                handle_interrupt(&mut interpreter, &interrupt);
            }
            InterpreterStatus::TerminatedWithException => {
                panic!("Program Terminated with exception");
            }
            _ => {}
        }
    }
    interpreter
}

fn lex_only(code: &str) -> Compiler {
    let s68k = S68k::new(code.to_string());
    let errors = s68k.semantic_check();
    if !errors.is_empty() {
        panic!("Code did not pass semantic check: {:#?}", errors);
    }
    s68k.compile().expect("To compile correctly")
}


fn handle_interrupt(interpreter: &mut Interpreter, interrupt: &Interrupt) {
    match interrupt {
        Interrupt::DisplayNumber(number) => {
            print!("{}", number);
            interpreter
                .answer_interrupt(InterruptResult::DisplayNumber)
                .unwrap();
        }
        Interrupt::DisplayStringWithCRLF(string) => {
            println!("{}", string);
            interpreter
                .answer_interrupt(InterruptResult::DisplayStringWithCRLF)
                .unwrap();
        }
        Interrupt::DisplayStringWithoutCRLF(string) => {
            print!("{}", string);
            interpreter
                .answer_interrupt(InterruptResult::DisplayStringWithoutCRLF)
                .unwrap();
        }
        Interrupt::GetTime => {
            interpreter
                .answer_interrupt(InterruptResult::GetTime(0))
                .unwrap();
        }
        Interrupt::DisplayChar(char) => {
            print!("{}", char);
            interpreter
                .answer_interrupt(InterruptResult::DisplayChar)
                .unwrap();
        }
        Interrupt::ReadChar => {
            let char = Term::stdout().read_char().expect("Unable to read char");
            interpreter
                .answer_interrupt(InterruptResult::ReadChar(char))
                .unwrap();
        }
        Interrupt::ReadNumber => {
            let num = Term::stdout().read_line().expect("Unable to read line");
            let num = num.trim().parse::<i32>().expect("Unable to parse number");
            interpreter
                .answer_interrupt(InterruptResult::ReadNumber(num))
                .unwrap();
        }
        Interrupt::ReadKeyboardString => {
            let string = Term::stdout().read_line().expect("Unable to read line");
            interpreter
                .answer_interrupt(InterruptResult::ReadKeyboardString(string))
                .unwrap();
        }
        Interrupt::Terminate => {
            interpreter
                .answer_interrupt(InterruptResult::Terminate)
                .unwrap();
        }
    }
}
