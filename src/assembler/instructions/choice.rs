//! EASy68K's choices among a 68000 instruction's encodings.
//!
//! Where the 68000 has more than one encoding of what a line says — a short or
//! a word branch, an absolute address in a word or a long, `move.l #1,d0` or
//! `moveq #1,d0`, `add.l #1,d0` or `addq.l #1,d0` — EASy68K picks one by a rule
//! that reads two things only: whether every Symbol in the Operand is a **back
//! reference**, defined above the line, and the Operand's value (the
//! asm-editor's `docs/research/easy68k-machine-behaviour.md` §13). The
//! functions here are those rules, one per choice, and nothing else: they take
//! the two facts and answer the encoding, so that the lowering
//! ([`super::lowering`]) and the Layout's first pass, which has to know an
//! instruction's size before it can lay out the next line, ask the same
//! question and get the same answer.
//!
//! That is why there is no relaxation. A forward reference is never short,
//! whatever its value turns out to be, so the size pass 1 gives a line is the
//! size pass 2 encodes it at; EASy68K assembles in two passes and so does
//! s68k.
//!
//! A number, a character literal and `*` are back references: only a Symbol
//! can be defined below the line. A Label on the line itself is one too,
//! because EASy68K defines it before it reads the Operands, so `loop bra loop`
//! is a short branch.

use crate::assembler::ast::SizeSuffix;

use super::encoded::{AddressWidth, BranchSize};

/// The displacement a branch at `address` encodes to reach `target`: the
/// distance from the word after the operation word, which is where the 68000's
/// program counter is when it adds it, read in 32 bits.
pub fn branch_displacement(address: u32, target: u32) -> i64 {
    target.wrapping_sub(address.wrapping_add(2)) as i32 as i64
}

/// The size of a `bra`, `bsr` or `Bcc`'s displacement, by EASy68K's rule
/// (`branch()` in its `BUILD.CPP`).
///
/// * `.s` and `.b` force a short branch, and `.w` and `.l` a word one: the
///   68000 has no long branch, so `.l` means a word, as it does in EASy68K.
///   Whether a forced short displacement fits is the caller's question; it
///   is still short.
/// * Unsized, a branch is short when its target is a back reference, the
///   displacement fits a byte and it is not zero, which is the encoding of a
///   word branch. Otherwise it is a word, so a branch forward is always a
///   word.
pub fn branch_size(
    written: Option<SizeSuffix>,
    back_reference: bool,
    displacement: i64,
) -> BranchSize {
    match written {
        Some(SizeSuffix::Short | SizeSuffix::Byte) => BranchSize::Short,
        Some(SizeSuffix::Word | SizeSuffix::Long) => BranchSize::Word,
        None if back_reference && fits_a_short_branch(displacement) => BranchSize::Short,
        None => BranchSize::Word,
    }
}

/// Whether a short branch can encode `displacement`: a byte, and not 0, which
/// is the byte that marks a word branch.
pub fn fits_a_short_branch(displacement: i64) -> bool {
    (-128..=127).contains(&displacement) && displacement != 0
}

/// The width of an absolute address, by EASy68K's rule (`opParse` in its
/// `OPPARSE.CPP`).
///
/// `.w` and `.l` force the width. Unsized, the address is a word when it is a
/// back reference and its 32 bits, read as a signed number, are within
/// −32768 to 32767, which is what a word sign extends to: `$0` to `$7FFF` and
/// `$FFFF8000` to `$FFFFFFFF`. Otherwise it is a long, so a forward reference
/// is always a long. `.b` and `.s` are no width; the analyzer refuses them, and
/// they answer a long here.
pub fn address_width(
    written: Option<SizeSuffix>,
    back_reference: bool,
    address: u32,
) -> AddressWidth {
    match written {
        Some(SizeSuffix::Word) => AddressWidth::Word,
        Some(_) => AddressWidth::Long,
        None if back_reference && fits_a_short_address(address) => AddressWidth::Word,
        None => AddressWidth::Long,
    }
}

/// Whether a word holds `address`: whether sign extending its low sixteen bits
/// gives the address back.
pub fn fits_a_short_address(address: u32) -> bool {
    (-32768..=32767).contains(&(address as i32))
}

/// The count of the `addq` or `subq` an `add`, `addi`, `adda`, `sub`, `subi` or
/// `suba` of an immediate assembles as, or `None` when it keeps its own form.
///
/// EASy68K makes the quick form of every one of the six whose immediate is a
/// back reference from 1 to 8 (`immedInst` and `arithReg` in its `BUILD.CPP`);
/// `#n.l` keeps the long form, which is the one reason to write it. The value
/// is the immediate's 32 bits read as a signed number, as EASy68K reads it.
pub fn quick_count(back_reference: bool, long: bool, value: u32) -> Option<u8> {
    let value = value as i32;
    (back_reference && !long && (1..=8).contains(&value)).then_some(value as u8)
}

/// The value of the `moveq` a `move.l #n,dn` assembles as, or `None` when it
/// stays a `move`.
///
/// Only a `move` written `.l` qualifies: an unsized `move` is a word, and
/// EASy68K does not make a `moveq` of one (`move` in its `BUILD.CPP`). The
/// immediate has to be a back reference from −128 to 127, not written `#n.l`,
/// and the destination a data register, which is the caller's to know.
pub fn moveq_value(
    written: Option<SizeSuffix>,
    back_reference: bool,
    long: bool,
    value: u32,
) -> Option<u8> {
    let value = value as i32;
    (written == Some(SizeSuffix::Long) && back_reference && !long && (-128..=127).contains(&value))
        .then_some(value as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsized_branch_is_short_only_backwards_within_a_byte() {
        assert_eq!(branch_size(None, true, -2), BranchSize::Short);
        assert_eq!(branch_size(None, true, -128), BranchSize::Short);
        assert_eq!(branch_size(None, true, 127), BranchSize::Short);
        assert_eq!(branch_size(None, true, -129), BranchSize::Word);
        assert_eq!(branch_size(None, true, 128), BranchSize::Word);
        // The byte `$00` is the mark of a word branch.
        assert_eq!(branch_size(None, true, 0), BranchSize::Word);
        // A forward reference is a word, however close it turns out to be.
        assert_eq!(branch_size(None, false, 4), BranchSize::Word);
    }

    #[test]
    fn a_written_branch_size_is_what_the_branch_is() {
        for written in [SizeSuffix::Short, SizeSuffix::Byte] {
            assert_eq!(branch_size(Some(written), false, 1000), BranchSize::Short);
        }
        for written in [SizeSuffix::Word, SizeSuffix::Long] {
            assert_eq!(branch_size(Some(written), true, -2), BranchSize::Word);
        }
    }

    #[test]
    fn a_branch_displacement_is_measured_from_the_word_after_the_operation() {
        assert_eq!(branch_displacement(0x1000, 0x1000), -2);
        assert_eq!(branch_displacement(0x1000, 0x1002), 0);
        assert_eq!(branch_displacement(0x1000, 0x1081), 0x7F);
        assert_eq!(branch_displacement(0, 0xFFFF_FFF2), -16);
    }

    #[test]
    fn an_unsized_address_is_a_word_only_when_it_is_known_and_a_word_holds_it() {
        assert_eq!(address_width(None, true, 0x7FFF), AddressWidth::Word);
        assert_eq!(address_width(None, true, 0x8000), AddressWidth::Long);
        assert_eq!(address_width(None, true, 0xFFFF_8000), AddressWidth::Word);
        assert_eq!(address_width(None, true, 0xFFFF_7FFF), AddressWidth::Long);
        assert_eq!(address_width(None, false, 0x1000), AddressWidth::Long);
        assert_eq!(
            address_width(Some(SizeSuffix::Word), false, 0x1000),
            AddressWidth::Word
        );
        assert_eq!(
            address_width(Some(SizeSuffix::Long), true, 0x1000),
            AddressWidth::Long
        );
    }

    #[test]
    fn a_quick_form_takes_one_to_eight_known_and_not_written_long() {
        assert_eq!(quick_count(true, false, 1), Some(1));
        assert_eq!(quick_count(true, false, 8), Some(8));
        assert_eq!(quick_count(true, false, 0), None);
        assert_eq!(quick_count(true, false, 9), None);
        assert_eq!(quick_count(true, false, (-1i32) as u32), None);
        assert_eq!(quick_count(false, false, 1), None, "a forward reference");
        assert_eq!(quick_count(true, true, 1), None, "`#1.l`");
    }

    #[test]
    fn moveq_is_a_move_written_long_of_a_known_byte() {
        let long = Some(SizeSuffix::Long);
        assert_eq!(moveq_value(long, true, false, 5), Some(5));
        assert_eq!(moveq_value(long, true, false, (-128i32) as u32), Some(0x80));
        assert_eq!(moveq_value(long, true, false, 127), Some(0x7F));
        assert_eq!(moveq_value(long, true, false, 128), None);
        assert_eq!(moveq_value(long, true, false, 0xFFFF_FF7F), None);
        assert_eq!(moveq_value(None, true, false, 5), None, "an unsized move");
        assert_eq!(
            moveq_value(long, false, false, 5),
            None,
            "a forward reference"
        );
        assert_eq!(moveq_value(long, true, true, 5), None, "`#5.l`");
    }
}
