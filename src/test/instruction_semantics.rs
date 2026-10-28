//! Instructions as a 68000 runs them, where the conformance suite
//! (SingleStepTests/m68000, ADR 0010) found s68k running them otherwise.
//!
//! Each case is one test of the suite, named as the suite names it, cut down
//! to what it reads: the registers and RAM of its initial state, the
//! instruction's words at its address, and what its final state holds.

use crate::interpreter::{Cpu, Interpreter, InterpreterStatus};

/// The initial state of a suite test: the instruction's words at `pc`, the
/// registers (A7 the stack pointer of the test's mode) and the status
/// register, and the RAM words it lists.
struct Case<'a> {
    pc: usize,
    words: [u16; 2],
    d: [u32; 8],
    a: [u32; 8],
    sr: u16,
    ram: &'a [(usize, u16)],
}

impl Case<'_> {
    /// The Interpreter after the one instruction, which must have run.
    fn step(&self) -> Interpreter {
        let cpu = Cpu::from_registers(self.d, self.a, self.sr);
        let mut interpreter = Interpreter::from_state(cpu, self.pc, self.ram, self.words);
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        interpreter
    }
}

fn d(interpreter: &Interpreter, register: usize) -> u32 {
    interpreter.get_cpu().get_register_values()[register]
}

fn a(interpreter: &Interpreter, register: usize) -> u32 {
    interpreter.get_cpu().get_register_values()[8 + register]
}

fn word(interpreter: &Interpreter, address: usize) -> u16 {
    let bytes = interpreter.get_memory().read_bytes(address, 2).unwrap();
    u16::from_be_bytes([bytes[0], bytes[1]])
}

// Shifts and rotates

/// `008 ASR.b D0, D3 e023`: a count of 58 shifts a negative byte past its
/// width, which leaves copies of the sign, and the last of them is X and C.
#[test]
fn asr_by_the_size_or_more_leaves_the_sign_in_x_and_c() {
    let after = Case {
        pc: 0xCCE03E,
        words: [0xE023, 0x2453],
        d: [0xBA5D_E77A, 0xA2F2_BB8E, 0x3E2C_763F, 0x8146_37CD, 0x7528_BA8B, 0x3788_1F1F, 0x2958_71D6, 0xD4B2_4204],
        a: [0; 8],
        sr: 0x041D,
        ram: &[],
    }
    .step();
    assert_eq!(d(&after, 3), 0x8146_37FF);
    assert_eq!(after.get_sr(), 0x0419);
}

/// `006 ASR.l D4, D7 e8a7`: the long form, a count of 38.
#[test]
fn asr_long_by_more_than_32_leaves_the_sign_in_x_and_c() {
    let after = Case {
        pc: 0xF6D1B2,
        words: [0xE8A7, 0x37D4],
        d: [0xF33D_7267, 0xD638_FC76, 0x2345_981A, 0xFB39_73E4, 0xA3DD_08A6, 0x4D8F_DB0E, 0xFE76_9481, 0xDFC8_F2F5],
        a: [0; 8],
        sr: 0x060E,
        ram: &[],
    }
    .step();
    assert_eq!(d(&after, 7), 0xFFFF_FFFF);
    assert_eq!(after.get_sr(), 0x0619);
}

// Binary coded decimal

/// N and V, which the BCD instructions leave as they were (the skip list's
/// relaxation): the other bits are compared.
const ALL_BUT_N_AND_V: u16 = !0x000A;

/// `069 ABCD D7, D7 cf07`: $4A + $4A + X is $9B, both digits corrected
/// as the 68000 corrects them, not $FB.
#[test]
fn abcd_corrects_a_digit_above_9_as_the_68000_does() {
    let after = Case {
        pc: 0x3C1048,
        words: [0xCF07, 0x6AF3],
        d: [0x95A2_BE0B, 0x579B_B3DE, 0x9EAE_88C3, 0x8B6B_7543, 0x5B2E_8FAE, 0xF963_7970, 0x0939_E940, 0x5F8B_E94A],
        a: [0; 8],
        sr: 0x8712,
        ram: &[],
    }
    .step();
    assert_eq!(d(&after, 7), 0x5F8B_E99B);
    assert_eq!(after.get_sr() & ALL_BUT_N_AND_V, 0x8708 & ALL_BUT_N_AND_V);
}

/// `051 ABCD -(A1), -(A6) cd09`: the memory form, $5D + $36 + X.
#[test]
fn abcd_in_memory_corrects_as_the_68000_does() {
    let after = Case {
        pc: 0x947596,
        words: [0xCD09, 0x7B32],
        d: [0; 8],
        a: [0xA183_3C8A, 0x69CB_70CD, 0x5A9E_7EA6, 0x79DF_83C0, 0x2A45_8527, 0x7D54_EC3C, 0x2EE9_7688, 0x0076_616E],
        sr: 0x271B,
        ram: &[(0xCB70CC, 0x36C8), (0xE97686, 0x3B5D)],
    }
    .step();
    assert_eq!(word(&after, 0xE97686), 0x3B9A);
    assert_eq!((a(&after, 1), a(&after, 6)), (0x69CB_70CC, 0x2EE9_7687));
    assert_eq!(after.get_sr() & ALL_BUT_N_AND_V, 0x2708 & ALL_BUT_N_AND_V);
}

/// `012 SBCD D4, D6 8d04`: $EE - $19 is $D5, the low digit borrowing and
/// corrected, the high one above 9 and left.
#[test]
fn sbcd_corrects_a_digit_above_9_as_the_68000_does() {
    let after = Case {
        pc: 0x0CE620,
        words: [0x8D04, 0xF229],
        d: [0x001D_E42C, 0xC4D9_21B5, 0x9849_AC29, 0xC58B_F6E5, 0xD7FF_AA19, 0x733C_DC0F, 0xE4B2_53EE, 0xAE09_84DC],
        a: [0; 8],
        sr: 0x830E,
        ram: &[],
    }
    .step();
    assert_eq!(d(&after, 6), 0xE4B2_53D5);
    assert_eq!(after.get_sr() & ALL_BUT_N_AND_V, 0x8308 & ALL_BUT_N_AND_V);
}

/// `015 SBCD -(A0), -(A3) 8708`: the memory form, $6F - $B0, which borrows.
#[test]
fn sbcd_in_memory_borrows_as_the_68000_does() {
    let after = Case {
        pc: 0xAA6700,
        words: [0x8708, 0xC686],
        d: [0; 8],
        a: [0x6860_3CF1, 0x926F_3487, 0xDC5A_C5B4, 0xB253_E15B, 0x94BE_B5B8, 0xE574_019F, 0xF6C9_A173, 0x0088_2A48],
        sr: 0x8507,
        ram: &[(0x603CF0, 0xB0B2), (0x53E15A, 0x6F96)],
    }
    .step();
    assert_eq!(word(&after, 0x53E15A), 0x5F96);
    assert_eq!(after.get_sr() & ALL_BUT_N_AND_V, 0x8513 & ALL_BUT_N_AND_V);
}

// Division

/// `013 DIVS (xxx).w, D4 89f8`: $9FA6A46D / $27C6 does not fit a word. The
/// register is left alone and N is set, from clear, with V.
#[test]
fn divs_overflow_sets_n_and_v() {
    let after = Case {
        pc: 0x30F974,
        words: [0x89F8, 0x3FBC],
        d: [0xA21A_BD6C, 0x4A11_5542, 0x6302_1EC8, 0x3B05_47A9, 0x9FA6_A46D, 0x01C4_8B1B, 0x1F4F_9C44, 0x7B87_2EA4],
        a: [0; 8],
        sr: 0x0601,
        ram: &[(0x003FBC, 0x27C6)],
    }
    .step();
    assert_eq!(d(&after, 4), 0x9FA6_A46D);
    assert_eq!(after.get_sr(), 0x060A);
}

/// `011 DIVU (A4), D1 82d4`: $F215E67E / $EF72 overflows, and Z, set before,
/// is cleared.
#[test]
fn divu_overflow_clears_z() {
    let after = Case {
        pc: 0xDD147E,
        words: [0x82D4, 0x53C6],
        d: [0x6FDD_2FD5, 0xF215_E67E, 0x83A0_7C54, 0x8CD2_A23F, 0x1047_BD4A, 0x6D7C_56A1, 0xC4C4_C93E, 0xB514_5608],
        a: [0xA8A4_B3D1, 0x554F_9F22, 0xABBE_FF62, 0x5F2B_4068, 0x52DE_674E, 0x39A7_0FA9, 0xA684_9885, 0x00EF_8796],
        sr: 0x0115,
        ram: &[(0xDE674E, 0xEF72)],
    }
    .step();
    assert_eq!(d(&after, 1), 0xF215_E67E);
    assert_eq!(after.get_sr(), 0x011A);
}

// CHK

/// `055 CHK D6, D6 4d86`: $2A2D is within 0 to itself, and N and Z, set
/// before, are cleared; X stays.
#[test]
fn chk_in_bounds_clears_n_z_v_and_c() {
    let after = Case {
        pc: 0xC657AA,
        words: [0x4D86, 0x670F],
        d: [0x53E5_C873, 0x8049_4AB9, 0x09D8_6B4E, 0x82B9_210D, 0x3F98_B494, 0x0B0F_03D8, 0x4A3A_2A2D, 0x9ACE_0D3B],
        a: [0; 8],
        sr: 0x211C,
        ram: &[],
    }
    .step();
    assert_eq!(after.get_sr(), 0x2110);
}

/// `078 CHK D1, D7 4f81`: $573B within 0 to $6F78, and V, set before, is
/// cleared.
#[test]
fn chk_in_bounds_clears_v() {
    let after = Case {
        pc: 0xBB583A,
        words: [0x4F81, 0x95EC],
        d: [0x9BBA_837F, 0x1CDC_6F78, 0x955F_6651, 0xE8D9_1EBF, 0xC3E9_8D29, 0xEED1_0B3C, 0x90A7_24E9, 0x2524_573B],
        a: [0; 8],
        sr: 0xA412,
        ram: &[],
    }
    .step();
    assert_eq!(after.get_sr(), 0xA410);
}

// LINK and UNLK on A7

/// `009 LINK A7, # 4e57`: the long pushed is A7 before the push, $F4A390,
/// and A7 ends at the frame plus the displacement.
#[test]
fn link_a7_pushes_a7_as_it_was() {
    let after = Case {
        pc: 0x1E19E8,
        words: [0x4E57, 0xF672],
        d: [0; 8],
        a: [0x52ED_61CB, 0x521F_DE1F, 0x8607_9AC0, 0x5A5D_C5C7, 0xD623_1FA3, 0xF365_1D03, 0x9950_0807, 0x00F4_A390],
        sr: 0x201D,
        ram: &[],
    }
    .step();
    assert_eq!((word(&after, 0xF4A38C), word(&after, 0xF4A38E)), (0x00F4, 0xA390));
    assert_eq!(a(&after, 7), 0x00F4_99FE);
}

/// `005 UNLINK A7 4e5f`, in user mode: A7 is loaded with the long at A7 and
/// not moved past it afterwards.
#[test]
fn unlk_a7_leaves_the_long_it_popped() {
    let after = Case {
        pc: 0x7756DA,
        words: [0x4E5F, 0xD114],
        d: [0; 8],
        a: [0x207F_5839, 0x4C15_3F19, 0xCDE9_F3D7, 0x5FAD_0FEB, 0x43AD_9BF4, 0x925F_A9DE, 0x59A9_3201, 0x005D_810A],
        sr: 0x0316,
        ram: &[(0x5D810A, 0x35F6), (0x5D810C, 0x64AA)],
    }
    .step();
    assert_eq!(a(&after, 7), 0x35F6_64AA);
}
