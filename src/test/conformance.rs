//! The conformance harness's own runs (ADR 0010): the vendored subset of the
//! SingleStepTests/m68000 suite and Musashi's self-checking programs in every
//! `cargo test`, and the whole suite from a fetched copy behind `--ignored`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::conformance::runner::{self, Report};
use crate::conformance::skips::{self, MUSASHI};
use crate::conformance::suite::{self, Kind};

/// The vendored subset: the first 50 tests of each file that raise no address
/// error (`tests/m68000/README.md`).
const VENDORED: &str = "tests/m68000";

/// Where `scripts/fetch-m68000-tests.sh` puts the whole suite, unless
/// `S68K_M68000_TESTS` names another folder of `.json.bin` files.
const FETCHED: &str = "tests/m68000/fetched/v1";

/// Musashi's programs, `tests/musashi/mc68000/`.
const MUSASHI_PROGRAMS: &str = "tests/musashi/mc68000";

/// How many tests of each file the vendored subset keeps.
const VENDORED_PER_FILE: usize = 50;

fn manifest(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// The `.json.bin` files of a folder, by name without the extension.
fn suite_files(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = fs::read_dir(folder)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", folder.display()))
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name()?.to_str()?.strip_suffix(".json.bin")?.to_string();
            Some((name, path))
        })
        .collect();
    files.sort();
    files
}

/// Runs every file, several at a time.
fn run_files(files: &[(String, PathBuf)]) -> Vec<Report> {
    let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
    let chunk = files.len().div_ceil(threads).max(1);
    let mut reports: Vec<Report> = std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(name, path)| {
                            let bytes = fs::read(path).unwrap();
                            runner::run_file(name, &bytes)
                                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|handle| handle.join().unwrap()).collect()
    });
    reports.sort_by(|a, b| a.file.cmp(&b.file));
    reports
}

/// The pass counts, one row a file and a total, then the skips by reason.
fn summary(reports: &[Report]) -> String {
    let mut text = format!(
        "{:<16} {:>6} {:>6} {:>6} {:>6} {:>7}\n",
        "file", "run", "passed", "failed", "known", "skipped"
    );
    let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut run, mut passed, mut failed, mut known, mut skips) = (0, 0, 0, 0, 0);
    for report in reports {
        let reasons: Vec<String> = report
            .skipped
            .iter()
            .map(|(name, count)| format!("{name} {count}"))
            .collect();
        text += &format!(
            "{:<16} {:>6} {:>6} {:>6} {:>6} {:>7}  {}\n",
            report.file,
            report.run,
            report.passed,
            report.failures.len(),
            report.known.len(),
            report.skipped_count(),
            reasons.join(", ")
        );
        run += report.run;
        passed += report.passed;
        failed += report.failures.len();
        known += report.known.len();
        skips += report.skipped_count();
        for (name, count) in &report.skipped {
            *skipped.entry(name).or_default() += count;
        }
    }
    text += &format!(
        "{:<16} {run:>6} {passed:>6} {failed:>6} {known:>6} {skips:>7}\n",
        "total"
    );
    for (name, count) in skipped {
        text += &format!("skipped, {name}: {count}\n");
    }
    text
}

/// Fails with the first failures of every file, after printing the summary.
fn assert_no_failures(reports: &[Report]) {
    println!("{}", summary(reports));
    let mut failures = String::new();
    for report in reports {
        for (name, problems) in report.failures.iter().take(5) {
            failures += &format!("{}: {name}\n    {}\n", report.file, problems.join("\n    "));
        }
    }
    let failed: usize = reports.iter().map(|report| report.failures.len()).sum();
    assert!(failed == 0, "{failed} tests failed:\n{failures}\n{}", summary(reports));
}

#[test]
fn the_vendored_subset_passes() {
    let files = suite_files(&manifest(VENDORED));
    assert_eq!(files.len(), 127, "the suite has 127 files");
    let reports = run_files(&files);
    for report in &reports {
        assert_eq!(
            report.run + report.skipped_count(),
            VENDORED_PER_FILE,
            "{} holds {VENDORED_PER_FILE} tests",
            report.file
        );
    }
    assert_no_failures(&reports);
}

/// The whole suite, 317,500 tests, from the copy the fetch script makes.
/// `S68K_M68000_ONLY=ASR.b,ASR.w` runs only the files it names.
#[test]
#[ignore = "needs the whole suite: scripts/fetch-m68000-tests.sh"]
fn the_whole_suite_passes() {
    let folder = std::env::var_os("S68K_M68000_TESTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest(FETCHED));
    assert!(
        folder.is_dir(),
        "{} is not there: run scripts/fetch-m68000-tests.sh, or set S68K_M68000_TESTS",
        folder.display()
    );
    let mut files = suite_files(&folder);
    if let Ok(only) = std::env::var("S68K_M68000_ONLY") {
        let only: Vec<&str> = only.split(',').collect();
        files.retain(|(name, _)| only.contains(&name.as_str()));
    }
    assert_no_failures(&run_files(&files));
}

/// Cuts the vendored subset from the fetched suite and checks that it is the
/// one committed, which is how `tests/m68000/` was made. With
/// `S68K_M68000_WRITE_CUT=1` it writes the cut there instead.
#[test]
#[ignore = "needs the whole suite: scripts/fetch-m68000-tests.sh"]
fn the_vendored_subset_is_the_cut_of_the_suite() {
    let write = std::env::var_os("S68K_M68000_WRITE_CUT").is_some();
    for (name, path) in suite_files(&manifest(FETCHED)) {
        let bytes = fs::read(&path).unwrap();
        let cut = suite::cut(&bytes, VENDORED_PER_FILE, |test| {
            !test.transactions.iter().any(|transaction| {
                matches!(transaction.kind, Kind::ReadAddressError | Kind::WriteAddressError)
            })
        })
        .unwrap();
        let vendored = manifest(VENDORED).join(format!("{name}.json.bin"));
        match write {
            true => fs::write(vendored, cut).unwrap(),
            false => assert!(fs::read(&vendored).unwrap() == cut, "{} is not the cut", vendored.display()),
        }
    }
}

#[test]
fn the_reader_reads_a_test_as_the_suite_wrote_it() {
    let bytes = fs::read(manifest(VENDORED).join("ADD.b.json.bin")).unwrap();
    let tests = suite::read(&bytes).unwrap();
    assert_eq!(tests.len(), VENDORED_PER_FILE);
    let test = &tests[0];
    assert_eq!(test.name, "000 ADD.b D3, (d16, A2) d72a");
    assert_eq!(test.initial.pc, 0x32CBB6);
    assert_eq!(test.initial.prefetch, [0xD72A, 0x9CBC]);
    assert_eq!(test.initial.sr, 0x8012);
    assert_eq!(test.initial.usp, 0x00B3_7EDC);
    assert!(test.initial.ram.contains(&(0xDB72AA, 0x6DF0)));
    assert!(test.final_state.ram.contains(&(0xDB72AA, 0x33F0)));
    assert_eq!(test.final_state.sr, 0x8011);
    assert_eq!(test.length, 16);
    assert_eq!(test.transactions.len(), 4);
    assert_eq!(test.transactions[3].kind, Kind::Write);
    assert_eq!(test.transactions[3].address, 0xDB72AA);
    assert_eq!(test.transactions[3].data, 0x3300);
    // a cut of every test is the file itself
    assert_eq!(suite::cut(&bytes, usize::MAX, |_| true).unwrap(), bytes);
}

#[test]
fn the_reader_refuses_a_truncated_file() {
    let bytes = fs::read(manifest(VENDORED).join("NOP.json.bin")).unwrap();
    let error = suite::read(&bytes[..bytes.len() - 1]).unwrap_err();
    assert!(error.what.contains("ends inside"), "{error}");
}

/// Every exception test of the vendored TRAP file is seen as one, through
/// the vector its `trap #n` names.
#[test]
fn exception_processing_is_found_from_the_final_state() {
    let bytes = fs::read(manifest(VENDORED).join("TRAP.json.bin")).unwrap();
    for test in suite::read(&bytes).unwrap() {
        let number = (test.initial.prefetch[0] & 0xF) as u8;
        assert_eq!(skips::exception_vector(&test), Some(32 + number), "{}", test.name);
    }
    let bytes = fs::read(manifest(VENDORED).join("ADD.l.json.bin")).unwrap();
    for test in suite::read(&bytes).unwrap() {
        assert_eq!(skips::exception_vector(&test), None, "{}", test.name);
    }
}

#[test]
fn musashi_programs_pass() {
    let mut programs: Vec<PathBuf> = fs::read_dir(manifest(MUSASHI_PROGRAMS))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
        .collect();
    programs.sort();
    assert_eq!(programs.len(), 60, "Musashi has 60 programs for the 68000");
    let mut failures = Vec::new();
    let mut passed = 0;
    for path in &programs {
        let name = path.file_stem().unwrap().to_str().unwrap();
        if MUSASHI.iter().any(|(skipped, _, _)| *skipped == name) {
            continue;
        }
        match runner::run_musashi(&fs::read(path).unwrap()) {
            Ok(()) => passed += 1,
            Err(reason) => failures.push(format!("{name}: {reason}")),
        }
    }
    println!("Musashi: {passed} passed, {} skipped", MUSASHI.len());
    assert!(failures.is_empty(), "{} failed:\n{}", failures.len(), failures.join("\n"));
}
