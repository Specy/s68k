//! The Program as an S-record file, in the layout EASy68K's assembler writes.
//!
//! EASy68K's simulator loads a `.S68` file: Motorola S-records, one to a line,
//! which its assembler writes in `OBJECT.CPP`. [`write`](fn@write) writes the same
//! records for a Program, so a program built here downloads and runs in
//! EASy68K (the asm-editor's `docs/design/m68k-machine.md`, decision 32, and
//! `docs/research/easy68k-machine-behaviour.md` §9 and §14):
//!
//! 1. the **header**, an `S0` record whose data is `68KPROG   20CREATED BY
//!    EASY68K`: the module name `68KPROG` padded to ten characters, the
//!    version and revision `2` and `0`, and the description. EASy68K's loader
//!    reads a memory map only from a file that starts with exactly this;
//! 2. the **data**, every byte of the Program's [image](Program::image) in
//!    address order, in records of up to 32 bytes that never run across a gap.
//!    A record is `S1` when its first address fits in 16 bits, `S2` when it
//!    fits in 24 and `S3` otherwise, as EASy68K chooses them;
//! 3. the **Memory map**, one more `S0` record for each range, in EASy68K's
//!    order: a name of ten characters, `      ROM `, `     READ `,
//!    `PROTECTED ` or `  INVALID `, then the start and the end as six
//!    upper-case hexadecimal digits each, `,` between them;
//! 4. the **Entry point**, an `S8` record whose address is the program's first
//!    instruction to run, in 24 bits.
//!
//! Every record is `S`, its type, a count of the bytes after the count, the
//! address, the data and a checksum, the ones' complement of the low byte of
//! the sum of the count, address and data bytes, all as upper-case hexadecimal
//! pairs. A record ends with CR LF, as EASy68K writes a text file on Windows.
//!
//! EASy68K closes a data record when the next item it assembles would take it
//! past 36 bytes of count, address and data, so where its records break inside
//! a run depends on the sizes of the items it was given. These break every 32
//! bytes instead. The two load the same bytes at the same addresses.

use super::program::{MemoryRangeKind, Program};

/// The most data bytes one record of the image holds.
pub const DATA_BYTES_PER_RECORD: usize = 32;

/// The header record's data, EASy68K's: module name, version, revision and
/// description.
pub const HEADER: &str = "68KPROG   20CREATED BY EASY68K";

/// What ends each record: CR LF, as EASy68K's files have it.
pub const LINE_END: &str = "\r\n";

/// The Program as EASy68K's S-record file.
pub fn write(program: &Program) -> String {
    let mut out = String::new();
    record(&mut out, '0', &[0, 0], HEADER.as_bytes());
    for run in program.image() {
        for (index, chunk) in run.bytes.chunks(DATA_BYTES_PER_RECORD).enumerate() {
            let address = (run.address + index * DATA_BYTES_PER_RECORD) as u32;
            let (kind, address) = match address {
                0..=0xFFFF => ('1', &address.to_be_bytes()[2..]),
                0x1_0000..=0xFF_FFFF => ('2', &address.to_be_bytes()[1..]),
                _ => ('3', &address.to_be_bytes()[..]),
            };
            record(&mut out, kind, address, chunk);
        }
    }
    for range in program.memory_map() {
        let data = format!(
            "{}{:06X},{:06X}",
            map_name(range.kind),
            range.start & 0xFF_FFFF,
            range.end & 0xFF_FFFF
        );
        record(&mut out, '0', &[0, 0], data.as_bytes());
    }
    let entry = (program.entry() as u32 & 0xFF_FFFF).to_be_bytes();
    record(&mut out, '8', &entry[1..], &[]);
    out
}

/// The ten characters a range's `S0` record names its kind with, EASy68K's
/// own, which its loader compares the first nine of.
fn map_name(kind: MemoryRangeKind) -> &'static str {
    match kind {
        MemoryRangeKind::Rom => "      ROM ",
        MemoryRangeKind::Read => "     READ ",
        MemoryRangeKind::Protected => "PROTECTED ",
        MemoryRangeKind::Invalid => "  INVALID ",
    }
}

/// One record of type `kind`, with its count and its checksum.
fn record(out: &mut String, kind: char, address: &[u8], data: &[u8]) {
    let count = (address.len() + data.len() + 1) as u8;
    let mut sum = count;
    out.push('S');
    out.push(kind);
    out.push_str(&format!("{count:02X}"));
    for byte in address.iter().chain(data) {
        sum = sum.wrapping_add(*byte);
        out.push_str(&format!("{byte:02X}"));
    }
    out.push_str(&format!("{:02X}", !sum));
    out.push_str(LINE_END);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::assembler::assemble_source;
    use crate::assembler::program::MemoryRange;

    /// What an S-record file says, read back by a reader that knows nothing of
    /// the writer: the text of every `S0`, every byte by its address, and the
    /// address of the `S8`.
    #[derive(Debug, Default, PartialEq)]
    struct Read {
        headers: Vec<String>,
        bytes: BTreeMap<u32, u8>,
        entry: Option<u32>,
        /// The type of every record, in order.
        kinds: Vec<char>,
    }

    /// Reads `text` record by record, checking each record's count and
    /// checksum, that every line ends with CR LF, and that no byte is loaded
    /// twice.
    fn read(text: &str) -> Read {
        let mut found = Read::default();
        assert!(text.ends_with("\r\n"), "the last record ends its line too");
        for line in text.split_terminator("\r\n") {
            assert!(!line.contains('\n') && !line.contains('\r'), "{line:?}");
            let mut characters = line.chars();
            assert_eq!(characters.next(), Some('S'), "{line}");
            let kind = characters.next().expect("a type");
            let pairs: Vec<u8> = (2..line.len())
                .step_by(2)
                .map(|at| {
                    let pair = &line[at..at + 2];
                    assert_eq!(pair, pair.to_uppercase(), "upper-case hex: {line}");
                    u8::from_str_radix(pair, 16).expect("hex pairs")
                })
                .collect();
            assert_eq!(line.len() % 2, 0, "whole pairs: {line}");
            let count = pairs[0] as usize;
            assert_eq!(count, pairs.len() - 1, "the count is what follows it: {line}");
            let sum = pairs.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
            assert_eq!(sum, 0xFF, "the checksum makes the sum $FF: {line}");
            let body = &pairs[1..pairs.len() - 1];
            let address_length = match kind {
                '0' | '1' => 2,
                '2' | '8' => 3,
                '3' => 4,
                other => panic!("no S{other} record is written: {line}"),
            };
            let address = body[..address_length]
                .iter()
                .fold(0u32, |address, byte| (address << 8) | *byte as u32);
            let data = &body[address_length..];
            match kind {
                '0' => {
                    assert_eq!(address, 0, "an S0 has no address");
                    found.headers.push(String::from_utf8(data.to_vec()).expect("ASCII"));
                }
                '8' => {
                    assert!(data.is_empty());
                    assert!(found.entry.replace(address).is_none(), "one S8");
                }
                _ => {
                    assert!(!data.is_empty() && data.len() <= 32, "{line}");
                    for (offset, byte) in data.iter().enumerate() {
                        let at = address + offset as u32;
                        assert!(found.bytes.insert(at, *byte).is_none(), "${at:X} twice");
                    }
                }
            }
            found.kinds.push(kind);
        }
        found
    }

    /// Every byte of a Program's image, by its address.
    fn image_bytes(program: &Program) -> BTreeMap<u32, u8> {
        program
            .image()
            .iter()
            .flat_map(|run| {
                run.bytes
                    .iter()
                    .enumerate()
                    .map(move |(offset, byte)| ((run.address + offset) as u32, *byte))
            })
            .collect()
    }

    fn program(source: &str) -> Program {
        let assembly = assemble_source(source);
        assert!(
            !assembly.has_errors(),
            "{:?}",
            assembly
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message())
                .collect::<Vec<_>>()
        );
        assembly.program.expect("a program")
    }

    /// The memory map an `S0` record names, as EASy68K's loader reads it: the
    /// first nine characters, then six digits at 11 and six at 18.
    fn map_of(headers: &[String]) -> Vec<(MemoryRangeKind, u32, u32)> {
        headers[1..]
            .iter()
            .map(|header| {
                let kind = match &header[..9] {
                    "      ROM" => MemoryRangeKind::Rom,
                    "     READ" => MemoryRangeKind::Read,
                    "PROTECTED" => MemoryRangeKind::Protected,
                    "  INVALID" => MemoryRangeKind::Invalid,
                    other => panic!("no range is named {other:?}"),
                };
                assert_eq!(&header[9..10], " ");
                assert_eq!(&header[16..17], ",");
                let number = |text: &str| u32::from_str_radix(text, 16).expect("six digits");
                (kind, number(&header[10..16]), number(&header[17..23]))
            })
            .collect()
    }

    #[test]
    fn a_small_program_is_the_records_easy68k_writes() {
        let program = program(
            "    org $1000
start:
    moveq #5,d0
    simhalt
    memory rom $8000,$8FFF
    end start
",
        );
        assert_eq!(
            program.to_srecord(),
            [
                // The header EASy68K writes, checksum and all, as its own
                // loader quotes it in `SIMOPS2.CPP`.
                "S021000036384B50524F47202020323043524541544544204259204541535936384B6D",
                // `moveq #5,d0` and `simhalt`, six bytes at `$1000`.
                "S10910007005FFFFFFFF75",
                // `      ROM 008000,008FFF`.
                "S01A0000202020202020524F4D203030383030302C30303846464659",
                // The entry point.
                "S804001000EB",
                "",
            ]
            .join("\r\n")
        );
    }

    #[test]
    fn what_is_written_reads_back_as_the_program() {
        let program = program(
            "    org $1000
start:
    lea text,a1
    move.b #14,d0
    trap #15
    bra.s start
    org $2000
text: dc.b 'A string long enough to take two records, and a third',0
    ds.l 4
words: dc.w 1,2,3
    dc.b 5
    dc.w 6
    org $FFF8
across: dc.l $11223344,$55667788,$99AABBCC
    org $123456
far: dc.b 1,2,3
    memory invalid $F00000,$FFFFFF
    memory Protected 0,$3FF
    memory read $FF000400,$FF0007FF
    end start
",
        );
        let found = read(&program.to_srecord());
        assert_eq!(found.headers[0], HEADER);
        assert_eq!(found.bytes, image_bytes(&program), "every byte, nothing else");
        assert_eq!(found.entry, Some(0x1000));
        assert_eq!(
            map_of(&found.headers),
            program
                .memory_map()
                .iter()
                .map(|MemoryRange { kind, start, end, .. }| (*kind, *start, *end))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            map_of(&found.headers),
            [
                (MemoryRangeKind::Read, 0x400, 0x7FF),
                (MemoryRangeKind::Protected, 0, 0x3FF),
                (MemoryRangeKind::Invalid, 0xF00000, 0xFFFFFF),
            ],
            "in EASy68K's order, masked to 24 bits"
        );
        // The header first, the data, the map, the entry last.
        assert_eq!(found.kinds.first(), Some(&'0'));
        assert_eq!(found.kinds.last(), Some(&'8'));
        let data: Vec<char> = found.kinds[1..found.kinds.len() - 4].to_vec();
        assert!(data.iter().all(|kind| *kind == '1' || *kind == '2'), "{data:?}");
        // A record below `$10000` is S1, even the one that runs past it, and
        // one that starts above it is S2: the code and `text` are S1, as is
        // the record of `across` at `$FFF8`, and `far` is S2.
        assert_eq!(&found.kinds[found.kinds.len() - 5..], ['2', '0', '0', '0', '8']);
        assert_eq!(found.kinds.iter().filter(|kind| **kind == '2').count(), 1);
        // `text` is 54 bytes from `$2000`, then `ds.l 4` reserves `$2036` to
        // `$2045` and the pad before the last `dc.w` is `$204D`: neither is
        // ever written, so each ends a record.
        assert!(found.bytes.contains_key(&0x2035) && found.bytes.contains_key(&0x2046));
        assert!((0x2036..0x2046).all(|address| !found.bytes.contains_key(&address)));
        assert!(!found.bytes.contains_key(&0x204D));
        assert_eq!(found.bytes.get(&0x204C), Some(&5));
    }

    #[test]
    fn a_run_longer_than_a_record_is_cut_every_32_bytes() {
        let program = program("    org $2000\n    dcb.b 70,$AB\n    nop\n");
        let text = program.to_srecord();
        let lengths: Vec<usize> = text
            .split_terminator("\r\n")
            .filter(|line| line.starts_with("S1"))
            .map(|line| (line.len() - 10) / 2)
            .collect();
        assert_eq!(lengths, [32, 32, 8], "72 bytes: the 70 of the block and the nop's two");
        assert_eq!(read(&text).bytes, image_bytes(&program));
    }

    #[test]
    fn every_editor_program_reads_back_as_itself() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/editor");
        let mut checked = 0;
        for entry in std::fs::read_dir(directory).expect("the corpus") {
            let path = entry.expect("an entry").path();
            let source = std::fs::read_to_string(&path).expect("a program");
            let program = program(&source);
            let found = read(&program.to_srecord());
            assert_eq!(found.bytes, image_bytes(&program), "{}", path.display());
            assert_eq!(found.entry, Some(program.entry() as u32), "{}", path.display());
            assert_eq!(found.headers, [HEADER], "{}", path.display());
            checked += 1;
        }
        assert_eq!(checked, 30);
    }
}
