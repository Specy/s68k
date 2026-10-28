//! The decode cache: the instructions the Interpreter runs, decoded once and
//! kept by the address they start at (ADR 0006).
//!
//! # Pages and slots
//!
//! The 16 MB of memory are 4,096 pages of 4 KB, and a page has one [`Slot`] per
//! word, 2,048 of them, since an instruction starts on an even address. A page
//! exists only once an instruction is kept in it, so the stack and the data
//! cost nothing. A slot holds the decoded [`Instruction`], the bytes it takes
//! up, and where it came from: the index of the Program's instruction it was
//! filled from, or none when it was decoded from memory at run time.
//!
//! A slot is 48 bytes on a 64-bit host and less in WebAssembly, so a page is
//! 96 KB at most: a program of a few thousand instructions, which fits in one
//! or two pages, takes a few hundred KB. The table of pages is 32 KB and the
//! table of [`near`](DecodeCache) flags 4 KB, both made once.
//!
//! # Fetching
//!
//! [`get`](DecodeCache::get) is two array reads, the page and then the slot;
//! the Interpreter decodes on a miss and [`store`](DecodeCache::store)s the
//! result. `Interpreter::new` fills the slots of every assembled instruction
//! before the first step, so assembled code is never decoded at run time.
//!
//! # Writing
//!
//! Every write to memory passes [`invalidate`](DecodeCache::invalidate). A
//! write of up to a page to an address no page is near costs one branch: that
//! is where the stack and most data live. Otherwise each slot whose
//! instruction covers a written byte is cleared, which means looking back
//! [`LOOK_BACK`] bytes before the write, since an instruction of the 68000 is
//! at most ten bytes, and into the page before when the write starts near the
//! start of one. The first slot a write clears is kept until it is
//! [taken](DecodeCache::take_cleared): that is how the Interpreter learns that
//! the program wrote into its code.
//!
//! # Instructions the memory map checks
//!
//! A fetch from an invalid range of the Memory map, or from a protected one in
//! user mode, is a bus error, so the words of an instruction there are checked
//! every time it is fetched. The Interpreter does not store an instruction a
//! word of which such a range reaches: every fetch of it misses, and the miss
//! is where it checks. A Program without such a range stores everything, and
//! its fetches pay nothing for the map.

use crate::instructions::Instruction;

use super::decode::{Decoded, MAX_68000_SIZE};

/// The bytes of memory the cache covers: the 68000's 16 MB.
const MEMORY_SIZE: usize = 0x0100_0000;

/// log2 of the bytes of a page.
const PAGE_SHIFT: usize = 12;

/// The bytes of a page: 4 KB.
pub const PAGE_BYTES: usize = 1 << PAGE_SHIFT;

/// The slots of a page, one per word.
const SLOTS: usize = PAGE_BYTES / 2;

/// The pages of memory.
const PAGES: usize = MEMORY_SIZE / PAGE_BYTES;

/// How far before a written byte the instruction that covers it can start, at
/// most: an instruction of the 68000 is at most ten bytes, so one that covers
/// the byte at `a` starts at `a - 9` or after, and on an even address.
pub const LOOK_BACK: usize = MAX_68000_SIZE as usize - 1;

/// The `index` of a slot decoded from memory at run time.
const DECODED: u32 = u32::MAX;

/// The `index` of a slot that ends the program: `$FFFF $FFFF` decoded where the
/// program placed nothing (ADR 0009).
const END: u32 = u32::MAX - 1;

/// One decoded instruction of the cache.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    /// The instruction.
    pub instruction: Instruction,
    /// The index of the Program's instruction the slot was filled from, or
    /// [`DECODED`] or [`END`].
    index: u32,
    /// How many bytes the instruction takes up; 0 for a slot that holds
    /// nothing.
    size: u8,
}

/// A slot that holds nothing.
pub const EMPTY: Slot = Slot {
    instruction: Instruction::NOP,
    index: DECODED,
    size: 0,
};

impl Slot {
    /// The slot of the Program's instruction of index `index`, `size` bytes long.
    pub fn assembled(instruction: Instruction, size: usize, index: usize) -> Self {
        Self {
            instruction,
            index: index as u32,
            size: size as u8,
        }
    }

    /// The slot of an instruction decoded from memory at run time; `ends` when
    /// it is SIMHALT where the program placed nothing, which ends the program
    /// rather than running.
    pub fn decoded(decoded: Decoded, ends: bool) -> Self {
        Self {
            instruction: decoded.instruction,
            index: if ends { END } else { DECODED },
            size: decoded.size as u8,
        }
    }

    /// Whether the slot holds nothing: [`EMPTY`].
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// How many bytes the instruction takes up.
    #[inline(always)]
    pub fn size(&self) -> usize {
        self.size as usize
    }

    /// The index of the Program's instruction the slot holds as the Assembler
    /// built it, or `None` when it was decoded at run time.
    #[inline(always)]
    pub fn program_index(&self) -> Option<usize> {
        (self.index < END).then_some(self.index as usize)
    }

    /// Whether fetching this slot ends the program: SIMHALT where the program
    /// placed nothing.
    #[inline(always)]
    pub fn ends_the_program(&self) -> bool {
        self.index == END
    }
}

/// The instruction a write cleared from the cache, the first of a write or of
/// several since the last [`take_cleared`](DecodeCache::take_cleared).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cleared {
    /// The address the instruction started at.
    pub address: usize,
    /// The index of the Program's instruction the slot held, or `None` when it
    /// was decoded at run time.
    pub index: Option<usize>,
}

/// One page of slots.
type Page = [Slot; SLOTS];

/// The cache. See the [module](self).
pub struct DecodeCache {
    /// One entry a page of memory, `None` until a slot of it is stored.
    pages: Box<[Option<Box<Page>>; PAGES]>,
    /// One flag a page of memory: whether that page, the one before it or the
    /// one after it exists, which is whether a write of up to a page that
    /// starts in it can touch a slot.
    near: Box<[bool; PAGES]>,
    /// The first slot cleared since the last take.
    cleared: Option<Cleared>,
}

impl std::fmt::Debug for DecodeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecodeCache")
            .field("pages", &self.pages_in_use())
            .field("cleared", &self.cleared)
            .finish()
    }
}

impl Default for DecodeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeCache {
    /// An empty cache, with no page.
    pub fn new() -> Self {
        Self {
            pages: vec![None; PAGES]
                .into_boxed_slice()
                .try_into()
                .expect("PAGES pages"),
            near: Box::new([false; PAGES]),
            cleared: None,
        }
    }


    /// The slot of the instruction that starts at `address`, an even address
    /// of the 24 bits of the bus, or `None` when none is kept there.
    #[inline(always)]
    pub fn get(&self, address: usize) -> Option<&Slot> {
        let page = self.pages[(address >> PAGE_SHIFT) & (PAGES - 1)].as_deref()?;
        let slot = &page[(address & (PAGE_BYTES - 1)) >> 1];
        (slot.size != 0).then_some(slot)
    }

    /// Keeps `slot` as the instruction that starts at `address`, an even
    /// address of the 24 bits of the bus, making its page if it has none.
    pub fn store(&mut self, address: usize, slot: Slot) {
        let page = address >> PAGE_SHIFT;
        if self.pages[page].is_none() {
            self.pages[page] = Some(
                vec![EMPTY; SLOTS]
                    .into_boxed_slice()
                    .try_into()
                    .expect("a page is SLOTS slots"),
            );
            for near in page.saturating_sub(1)..(page + 2).min(PAGES) {
                self.near[near] = true;
            }
        }
        if let Some(slots) = self.pages[page].as_mut() {
            slots[(address & (PAGE_BYTES - 1)) >> 1] = slot;
        }
    }

    /// Clears the slot of every instruction that covers a byte of the `length`
    /// bytes written at `address`, an address of the 24 bits of the bus whose
    /// run ends inside memory.
    ///
    /// It is on the path of every store, so a write of up to a page that no page
    /// is near costs the one branch on [`near`](DecodeCache).
    #[inline(always)]
    pub fn invalidate(&mut self, address: usize, length: usize) {
        if length <= PAGE_BYTES && !self.near[(address >> PAGE_SHIFT) & (PAGES - 1)] {
            return;
        }
        self.clear(address, length);
    }

    #[cold]
    #[inline(never)]
    fn clear(&mut self, address: usize, length: usize) {
        let Self { pages, cleared, .. } = self;
        let end = (address + length).min(MEMORY_SIZE);
        let mut at = address.saturating_sub(LOOK_BACK).next_multiple_of(2);
        while at < end {
            let page = at >> PAGE_SHIFT;
            let page_end = ((page + 1) << PAGE_SHIFT).min(end);
            let Some(slots) = pages[page].as_mut() else {
                at = page_end.next_multiple_of(2);
                continue;
            };
            while at < page_end {
                let slot = &mut slots[(at & (PAGE_BYTES - 1)) >> 1];
                if slot.size != 0 && at + slot.size as usize > address {
                    if cleared.is_none() {
                        *cleared = Some(Cleared {
                            address: at,
                            index: slot.program_index(),
                        });
                    }
                    slot.size = 0;
                }
                at += 2;
            }
        }
    }

    /// Whether a write has cleared a slot since the last
    /// [`take_cleared`](DecodeCache::take_cleared).
    #[inline(always)]
    pub fn has_cleared(&self) -> bool {
        self.cleared.is_some()
    }

    /// The first slot a write cleared since the last call, and the record of
    /// it emptied.
    #[inline(always)]
    pub fn take_cleared(&mut self) -> Option<Cleared> {
        self.cleared.take()
    }

    /// How many pages exist: the memory the cache takes, a page at a time.
    pub fn pages_in_use(&self) -> usize {
        self.pages.iter().filter(|page| page.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instructions::Size;

    fn slot(size: usize, index: usize) -> Slot {
        Slot::assembled(Instruction::NOP, size, index)
    }

    #[test]
    fn a_slot_is_kept_by_its_address_and_a_page_made_for_it() {
        let mut cache = DecodeCache::new();
        assert_eq!(cache.pages_in_use(), 0);
        assert!(cache.get(0x1000).is_none());
        cache.store(0x1000, slot(2, 7));
        assert_eq!(cache.pages_in_use(), 1);
        assert_eq!(cache.get(0x1000).and_then(|s| s.program_index()), Some(7));
        assert!(cache.get(0x1002).is_none(), "the next word holds nothing");
        assert!(cache.get(0x2000).is_none(), "nor the next page");
    }

    #[test]
    fn a_write_clears_every_instruction_that_covers_a_byte_of_it() {
        let mut cache = DecodeCache::new();
        // a ten-byte instruction at $1000 and a two-byte one at $100A
        cache.store(0x1000, slot(10, 0));
        cache.store(0x100A, slot(2, 1));
        cache.invalidate(0x1009, Size::Byte.to_bytes());
        assert!(cache.get(0x1000).is_none(), "its last byte, nine bytes on");
        assert!(cache.get(0x100A).is_some(), "the next one is untouched");
        assert_eq!(
            cache.take_cleared(),
            Some(Cleared {
                address: 0x1000,
                index: Some(0)
            })
        );
        assert_eq!(cache.take_cleared(), None, "taken once");
        cache.invalidate(0x100A, 4);
        assert!(cache.get(0x100A).is_none());
    }

    #[test]
    fn a_write_past_an_instruction_clears_nothing() {
        let mut cache = DecodeCache::new();
        cache.store(0x1000, slot(4, 0));
        cache.invalidate(0x1004, 4);
        cache.invalidate(0xFFC, 4);
        assert!(cache.get(0x1000).is_some());
        assert_eq!(cache.take_cleared(), None);
    }

    #[test]
    fn a_write_looks_back_into_the_page_before() {
        let mut cache = DecodeCache::new();
        // a six-byte instruction that ends two bytes into the next page
        cache.store(0x1FFC, slot(6, 3));
        cache.invalidate(0x2001, 1);
        assert!(cache.get(0x1FFC).is_none());
        // and a write that crosses into a page clears what starts there
        cache.store(0x3000, slot(2, 4));
        cache.invalidate(0x2FFE, 4);
        assert!(cache.get(0x3000).is_none());
    }

    #[test]
    fn a_long_write_clears_every_page_it_covers() {
        let mut cache = DecodeCache::new();
        cache.store(0x1000, slot(2, 0));
        cache.store(0x9000, slot(2, 1));
        cache.invalidate(0, 0x10000);
        assert!(cache.get(0x1000).is_none());
        assert!(cache.get(0x9000).is_none());
        assert_eq!(cache.take_cleared().map(|c| c.address), Some(0x1000));
    }

    #[test]
    fn a_write_far_from_every_page_is_one_branch() {
        let mut cache = DecodeCache::new();
        cache.store(0x1000, slot(2, 0));
        assert!(cache.near[0], "the page before");
        assert!(cache.near[1] && cache.near[2], "its own and the one after");
        assert!(!cache.near[3]);
        cache.invalidate(0xFFFFFC, 4);
        assert!(cache.get(0x1000).is_some());
    }
}
