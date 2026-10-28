//! The **Memory map** as the Interpreter enforces it: EASy68K's four kinds of
//! range, which a Program declares with `memory` lines (the asm-editor's
//! `docs/design/m68k-machine.md`, decision 15, and its
//! `docs/research/easy68k-machine-behaviour.md` §9).
//!
//! * **ROM** ignores a write: the store does nothing, and is no error.
//! * **Read only** refuses a write with a bus error.
//! * **Protected** refuses any access from user mode with a bus error.
//! * **Invalid** refuses any access with a bus error.
//!
//! The checks are EASy68K's, in its order, invalid, protected, read only and
//! ROM, and of its extents (`memoryMapCheck`, `mem_put` and `mem_req` in its
//! `UTILS.CPP`):
//!
//! * a **read** is checked against invalid and protected, and only its first
//!   byte is, so a long read that starts below a range and runs into it is not
//!   refused;
//! * a **write** is checked against all four, and any byte of it that falls in
//!   a range counts, `loc + bytes - 1 >= start && loc <= end`;
//! * an **instruction's words** are read as a read of each word, as EASy68K
//!   fetches each through `mem_req`;
//! * the `trap #15` tasks check the **buffers** they read or write against the
//!   kinds EASy68K names for each, which are never protected
//!   ([`MemoryMap::buffer`]).
//!
//! Protected reads the processor's real S bit, where EASy68K reads it from
//! the text of its window's SR field (design decision 1). Only the program is
//! checked: a Poke, a Testcase's starting memory, the host's reads and undo
//! write and read past the map, as the debugger is not the program.

use crate::assembler::program::{MemoryRange, MemoryRangeKind};

/// The mask of the 24 bits of the address bus.
const BUS: usize = 0x00FF_FFFF;

/// What the map says of one access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// No range has anything to say: the access goes ahead.
    Allowed,
    /// A ROM range takes the write and keeps what it held.
    Ignored,
    /// The range of this kind refuses the access: a bus error.
    Refused(MemoryRangeKind),
}

/// The kinds of range an access is checked against, as EASy68K's
/// `memoryMapCheck` takes them: a set of [`MemoryRangeKind`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kinds(u8);

impl Kinds {
    /// A write's: every kind.
    pub const WRITE: Kinds = Kinds(0b1111);
    /// A read's: invalid and protected.
    pub const READ: Kinds = Kinds(0b1100);
    /// What most `trap #15` tasks check their buffer against: invalid alone.
    pub const INVALID: Kinds = Kinds(0b1000);
    /// What task 53, which reads a file into memory, checks against: invalid,
    /// read only and ROM, which takes nothing.
    pub const INVALID_READ_ROM: Kinds = Kinds(0b1011);

    fn bit(kind: MemoryRangeKind) -> u8 {
        match kind {
            MemoryRangeKind::Rom => 0b0001,
            MemoryRangeKind::Read => 0b0010,
            MemoryRangeKind::Protected => 0b0100,
            MemoryRangeKind::Invalid => 0b1000,
        }
    }

    fn has(self, kind: MemoryRangeKind) -> bool {
        self.0 & Self::bit(kind) != 0
    }
}

/// The ranges, by kind, each its first and its last address in the 24 bits
/// of the bus, `None` where the Program declared none. A start above the end
/// covers nothing, as in EASy68K.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryMap {
    ranges: [Option<(usize, usize)>; 4],
}

impl MemoryMap {
    /// The map of a Program's `memory` lines.
    pub fn new(ranges: &[MemoryRange]) -> Self {
        let mut map = Self::default();
        for range in ranges {
            map.ranges[Self::slot(range.kind)] =
                Some((range.start as usize & BUS, range.end as usize & BUS));
        }
        map
    }

    /// Whether no range is declared, which is the one thing an access asks
    /// when none is.
    pub fn is_empty(&self) -> bool {
        self.ranges.iter().all(Option::is_none)
    }

    fn slot(kind: MemoryRangeKind) -> usize {
        match kind {
            MemoryRangeKind::Invalid => 0,
            MemoryRangeKind::Protected => 1,
            MemoryRangeKind::Read => 2,
            MemoryRangeKind::Rom => 3,
        }
    }

    /// The range of `kind`, if the Program declared one.
    pub fn range(&self, kind: MemoryRangeKind) -> Option<(usize, usize)> {
        self.ranges[Self::slot(kind)]
    }

    /// EASy68K's `memoryMapCheck`: the `length` bytes at `address`, an address
    /// of the 24 bits of the bus, against the ranges of `kinds`, in its order,
    /// invalid, protected, read only and ROM, the first that has a byte of
    /// them deciding. Protected refuses only when `supervisor` is false.
    ///
    /// The test is EASy68K's own, `loc + bytes - 1 >= start && loc <= end` in
    /// signed numbers, so a buffer of no bytes, a `trap #15` task's string
    /// that is only its NUL or a read of none, touches a range it starts in
    /// past the range's first byte, and not one it starts on the first byte of.
    pub fn check(&self, kinds: Kinds, address: usize, length: usize, supervisor: bool) -> Verdict {
        let last = address as i64 + length as i64 - 1;
        let touches = |kind| match self.range(kind) {
            Some((start, end)) => kinds.has(kind) && last >= start as i64 && address <= end,
            None => false,
        };
        if touches(MemoryRangeKind::Invalid) {
            return Verdict::Refused(MemoryRangeKind::Invalid);
        }
        if !supervisor && touches(MemoryRangeKind::Protected) {
            return Verdict::Refused(MemoryRangeKind::Protected);
        }
        if touches(MemoryRangeKind::Read) {
            return Verdict::Refused(MemoryRangeKind::Read);
        }
        if touches(MemoryRangeKind::Rom) {
            return Verdict::Ignored;
        }
        Verdict::Allowed
    }

    /// A read of the program's at `address`: its first byte against invalid,
    /// and against protected in user mode, as EASy68K's `mem_req` checks one.
    pub fn read(&self, address: usize, supervisor: bool) -> Verdict {
        self.check(Kinds::READ, address, 1, supervisor)
    }

    /// A write of the program's of `length` bytes at `address`, every byte of
    /// it against all four kinds, as EASy68K's `mem_put` checks one.
    pub fn write(&self, address: usize, length: usize, supervisor: bool) -> Verdict {
        self.check(Kinds::WRITE, address, length, supervisor)
    }

    /// A buffer of `length` bytes at `address` that a `trap #15` task reads or
    /// writes, against `kinds`: invalid for a string or the bytes a file is
    /// written from, and invalid, read only and ROM for those task 53 reads a
    /// file into (`CODE9.CPP`). Protected is never among them, so the mode
    /// does not matter.
    pub fn buffer(&self, kinds: Kinds, address: usize, length: usize) -> Verdict {
        self.check(kinds, address, length, true)
    }

    /// The ranges an instruction's words are checked against when they are
    /// fetched, which the decode cache keeps no instruction in: invalid, and
    /// protected, which depends on the mode.
    pub fn fetched_ranges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        [MemoryRangeKind::Invalid, MemoryRangeKind::Protected]
            .into_iter()
            .filter_map(|kind| self.range(kind))
            .filter(|(start, end)| start <= end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::source::Location;

    fn map(ranges: &[(MemoryRangeKind, u32, u32)]) -> MemoryMap {
        let ranges: Vec<MemoryRange> = ranges
            .iter()
            .map(|&(kind, start, end)| MemoryRange {
                kind,
                start,
                end,
                location: Location::new("main.x68", 0, 0, 0),
            })
            .collect();
        MemoryMap::new(&ranges)
    }

    #[test]
    fn each_kind_does_what_easy68k_does() {
        use MemoryRangeKind::*;
        let map = map(&[
            (Rom, 0x1000, 0x1fff),
            (Read, 0x2000, 0x2fff),
            (Protected, 0x3000, 0x3fff),
            (Invalid, 0x4000, 0x4fff),
        ]);
        for supervisor in [false, true] {
            assert_eq!(map.write(0x1000, 2, supervisor), Verdict::Ignored);
            assert_eq!(map.read(0x1000, supervisor), Verdict::Allowed);
            assert_eq!(map.write(0x2000, 2, supervisor), Verdict::Refused(Read));
            assert_eq!(map.read(0x2000, supervisor), Verdict::Allowed);
            assert_eq!(map.write(0x4000, 1, supervisor), Verdict::Refused(Invalid));
            assert_eq!(map.read(0x4fff, supervisor), Verdict::Refused(Invalid));
            assert_eq!(map.read(0x5000, supervisor), Verdict::Allowed);
        }
        assert_eq!(map.read(0x3000, false), Verdict::Refused(Protected));
        assert_eq!(map.write(0x3000, 4, false), Verdict::Refused(Protected));
        assert_eq!(map.read(0x3000, true), Verdict::Allowed);
        assert_eq!(map.write(0x3000, 4, true), Verdict::Allowed);
    }

    /// A write counts every byte it stores; a read only the first.
    #[test]
    fn a_write_overlaps_and_a_read_starts() {
        let map = map(&[(MemoryRangeKind::Invalid, 0x4000, 0x4fff)]);
        assert_eq!(
            map.write(0x3ffe, 4, true),
            Verdict::Refused(MemoryRangeKind::Invalid)
        );
        assert_eq!(map.read(0x3ffe, true), Verdict::Allowed);
        assert_eq!(map.write(0x3ffc, 4, true), Verdict::Allowed);
    }

    /// The first kind in EASy68K's order decides, and the buffers of the trap
    /// tasks are checked against the kinds they name alone.
    #[test]
    fn the_order_and_the_buffers_are_easy68ks() {
        use MemoryRangeKind::*;
        let map = map(&[(Rom, 0x1000, 0x1fff), (Read, 0x1800, 0x2fff)]);
        assert_eq!(map.write(0x1800, 2, true), Verdict::Refused(Read));
        assert_eq!(map.write(0x1000, 2, true), Verdict::Ignored);
        assert_eq!(map.buffer(Kinds::INVALID, 0x1000, 0x100), Verdict::Allowed);
        assert_eq!(
            map.buffer(Kinds::INVALID_READ_ROM, 0x1000, 0x10),
            Verdict::Ignored
        );
        assert_eq!(map.buffer(Kinds::INVALID_READ_ROM, 0x1000, 0), Verdict::Allowed);
        let protected = super::tests::map(&[(Protected, 0, 0xfff)]);
        assert_eq!(protected.buffer(Kinds::INVALID, 0, 0x10), Verdict::Allowed);
    }

    #[test]
    fn a_range_whose_start_is_above_its_end_covers_nothing() {
        let map = map(&[(MemoryRangeKind::Invalid, 0x5000, 0x4000)]);
        assert!(!map.is_empty());
        assert_eq!(map.read(0x4800, false), Verdict::Allowed);
        assert_eq!(map.fetched_ranges().count(), 0);
        assert!(MemoryMap::default().is_empty());
    }
}
