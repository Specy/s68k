//! The reader of the SingleStepTests/m68000 files, `<name>.json.bin`: the
//! suite's own binary form of Tom Harte's JSON tests, which the suite's
//! `decode.py` turns into JSON and which is read here directly.
//!
//! Everything is little endian. A file is a magic number and a count, then
//! the tests. Each test, and each part of one, is a block that starts with its
//! own length in bytes, header included, and a magic number of its own:
//!
//! * the test: the name, the initial state, the final state, the
//!   transactions;
//! * the name: a length and that many bytes of UTF-8;
//! * a state: D0–D7, A0–A6, USP, SSP, SR and PC as 19 `u32`s, the two
//!   prefetched words as two `u32`s, and the RAM as a count of `(u32 address,
//!   u16 word)` pairs;
//! * the transactions: the test's length in cycles and a count of records,
//!   each a `u8` kind and a `u32` of cycles, then, for every kind but idle,
//!   five `u32`s: function code, address, data, UDS and LDS.
//!
//! A state's PC is MAME's `m_au`, the address of the next prefetch, which is
//! four past the instruction the test runs (the suite's README).

use std::fmt;
use std::ops::Range;

const FILE_MAGIC: u32 = 0x1A3F_5D71;
const TEST_MAGIC: u32 = 0xABC1_2367;
const NAME_MAGIC: u32 = 0x89AB_CDEF;
const STATE_MAGIC: u32 = 0x0123_4567;
const TRANSACTIONS_MAGIC: u32 = 0x4567_89AB;

/// One test: one instruction run from a state, and what it left.
#[derive(Debug, Clone)]
pub struct Test {
    /// Its number in the file, the instruction and the operation word, as in
    /// `"000 ADD.b D3, (d16, A2) d72a"`.
    pub name: String,
    pub initial: State,
    pub final_state: State,
    /// The cycles the instruction took, with an immediate DTACK.
    pub length: u32,
    /// The bus cycles, in order.
    pub transactions: Vec<Transaction>,
}

/// The registers, the prefetch queue and the RAM of one side of a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub d: [u32; 8],
    /// A0 to A6: A7 is [`usp`](State::usp) or [`ssp`](State::ssp), by S.
    pub a: [u32; 7],
    pub usp: u32,
    pub ssp: u32,
    pub sr: u16,
    /// MAME's `m_au`: four past the instruction about to run.
    pub pc: u32,
    /// The two words the processor has prefetched, the first fetched first.
    pub prefetch: [u16; 2],
    /// The words the test reads or writes, at even addresses.
    pub ram: Vec<(u32, u16)>,
}

/// One bus cycle of a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transaction {
    pub kind: Kind,
    pub cycles: u32,
    /// The function code, FC2–FC0: 1 and 2 user data and program, 5 and 6
    /// supervisor data and program.
    pub function_code: u32,
    pub address: u32,
    /// The data bus as the pins carry it: a byte on the upper half is `$AB00`.
    pub data: u32,
    pub upper: bool,
    pub lower: bool,
}

/// What a bus cycle does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Idle,
    Write,
    Read,
    /// The read-modify-write of `tas`.
    Tas,
    /// A read that raised an address error: AS was never asserted.
    ReadAddressError,
    /// A write that raised an address error.
    WriteAddressError,
}

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    /// The byte the reader had got to.
    pub at: usize,
    pub what: String,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.at, self.what)
    }
}

impl std::error::Error for FormatError {}

/// Every test of a file, in order.
pub fn read(bytes: &[u8]) -> Result<Vec<Test>, FormatError> {
    Ok(read_spans(bytes)?.into_iter().map(|(test, _)| test).collect())
}

/// A file of the tests of `bytes` for which `keep` answers true, the first
/// `limit` of them, in order: each test's block copied as it is, under a new
/// count. It is how the vendored subset is cut (`tests/m68000/README.md`).
pub fn cut(
    bytes: &[u8],
    limit: usize,
    mut keep: impl FnMut(&Test) -> bool,
) -> Result<Vec<u8>, FormatError> {
    let kept: Vec<Range<usize>> = read_spans(bytes)?
        .into_iter()
        .filter(|(test, _)| keep(test))
        .take(limit)
        .map(|(_, span)| span)
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&FILE_MAGIC.to_le_bytes());
    out.extend_from_slice(&(kept.len() as u32).to_le_bytes());
    for span in kept {
        out.extend_from_slice(&bytes[span]);
    }
    Ok(out)
}

/// Every test with the bytes its block takes up in the file.
fn read_spans(bytes: &[u8]) -> Result<Vec<(Test, Range<usize>)>, FormatError> {
    let mut reader = Reader { bytes, at: 0 };
    reader.magic(FILE_MAGIC, "file")?;
    let count = reader.u32()?;
    let mut tests = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let start = reader.at;
        let end = reader.block(TEST_MAGIC, "test")?;
        let block = reader.block(NAME_MAGIC, "name")?;
        let length = reader.u32()? as usize;
        let at = reader.at;
        let name = String::from_utf8(reader.take(length)?.to_vec()).map_err(|_| FormatError {
            at,
            what: "a name that is not UTF-8".to_string(),
        })?;
        reader.end(block)?;
        let initial = reader.state()?;
        let final_state = reader.state()?;
        let block = reader.block(TRANSACTIONS_MAGIC, "transactions")?;
        let length_in_cycles = reader.u32()?;
        let count = reader.u32()?;
        let mut transactions = Vec::with_capacity(count as usize);
        for _ in 0..count {
            transactions.push(reader.transaction()?);
        }
        reader.end(block)?;
        reader.end(end)?;
        let test = Test { name, initial, final_state, length: length_in_cycles, transactions };
        tests.push((test, start..reader.at));
    }
    if reader.at != bytes.len() {
        return Err(reader.error("bytes after the last test"));
    }
    Ok(tests)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn error(&self, what: &str) -> FormatError {
        FormatError { at: self.at, what: what.to_string() }
    }

    fn take(&mut self, length: usize) -> Result<&[u8], FormatError> {
        let end = self.at.checked_add(length).filter(|&end| end <= self.bytes.len());
        let end = end.ok_or_else(|| self.error("the file ends inside a test"))?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn magic(&mut self, expected: u32, what: &str) -> Result<(), FormatError> {
        match self.u32()? {
            found if found == expected => Ok(()),
            found => Err(self.error(&format!("{what} magic {found:#010x}, not {expected:#010x}"))),
        }
    }

    /// Reads a block's header and answers where the block ends.
    fn block(&mut self, magic: u32, what: &str) -> Result<usize, FormatError> {
        let start = self.at;
        let length = self.u32()? as usize;
        self.magic(magic, what)?;
        Ok(start + length)
    }

    fn end(&self, end: usize) -> Result<(), FormatError> {
        match self.at == end {
            true => Ok(()),
            false => Err(self.error(&format!("a block that should end at byte {end}"))),
        }
    }

    fn state(&mut self) -> Result<State, FormatError> {
        let block = self.block(STATE_MAGIC, "state")?;
        let mut registers = [0; 19];
        for register in &mut registers {
            *register = self.u32()?;
        }
        let prefetch = [self.u32()? as u16, self.u32()? as u16];
        let count = self.u32()?;
        let mut ram = Vec::with_capacity(count as usize);
        for _ in 0..count {
            ram.push((self.u32()?, self.u16()?));
        }
        self.end(block)?;
        Ok(State {
            d: registers[0..8].try_into().unwrap(),
            a: registers[8..15].try_into().unwrap(),
            usp: registers[15],
            ssp: registers[16],
            sr: registers[17] as u16,
            pc: registers[18],
            prefetch,
            ram,
        })
    }

    fn transaction(&mut self) -> Result<Transaction, FormatError> {
        let kind = match self.u8()? {
            0 => Kind::Idle,
            1 => Kind::Write,
            2 => Kind::Read,
            3 => Kind::Tas,
            4 => Kind::ReadAddressError,
            5 => Kind::WriteAddressError,
            other => return Err(self.error(&format!("a transaction of kind {other}"))),
        };
        let cycles = self.u32()?;
        let mut transaction = Transaction {
            kind,
            cycles,
            function_code: 0,
            address: 0,
            data: 0,
            upper: false,
            lower: false,
        };
        if kind != Kind::Idle {
            transaction.function_code = self.u32()?;
            transaction.address = self.u32()?;
            transaction.data = self.u32()?;
            transaction.upper = self.u32()? != 0;
            transaction.lower = self.u32()? != 0;
        }
        Ok(transaction)
    }
}
