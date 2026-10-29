//! The build script, which does nothing unless the `musashi` or the `moira`
//! feature is on.
//!
//! With the first, it compiles Musashi (MIT, https://github.com/kstenerud/Musashi),
//! fetched by `scripts/fetch-musashi.sh` into `tests/musashi/fetched` or found
//! where `S68K_MUSASHI` says, for the differential harness of the 68010
//! dialect (`src/conformance/musashi.rs`): it builds Musashi's table generator
//! for the host, runs it on `m68k_in.c` to write `m68kops.c` and `m68kops.h`,
//! and compiles those with `m68kcpu.c`, `m68kdasm.c` and SoftFloat into a
//! static library. It is a test-only reference, never part of the
//! WebAssembly build.

fn main() {
    #[cfg(feature = "musashi")]
    musashi::build();
    #[cfg(feature = "moira")]
    moira::build();
}

/// With the `moira` feature, Moira's core (MIT,
/// https://github.com/dirkwhoffmann/Moira), fetched by
/// `scripts/fetch-moira.sh` into `tests/moira/fetched` or found where
/// `S68K_MOIRA` says, is copied into the build's own folder with its
/// configuration set for a 68010 that counts its cycles as the hardware does
/// (precise timing, no mimicry of Musashi, address errors), and compiled as
/// C++20 with `tests/moira/shim.cpp`, its C interface.
#[cfg(feature = "moira")]
mod moira {
    use std::path::{Path, PathBuf};

    pub fn build() {
        println!("cargo:rerun-if-env-changed=S68K_MOIRA");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = match std::env::var_os("S68K_MOIRA") {
            Some(path) => PathBuf::from(path),
            None => manifest.join("tests/moira/fetched"),
        }
        .join("Moira");
        if !source.join("Moira.cpp").exists() {
            panic!(
                "the `moira` feature compiles Moira, which is not at {}: run \
                 scripts/fetch-moira.sh, or name a copy with S68K_MOIRA",
                source.display()
            );
        }
        let shim = manifest.join("tests/moira/shim.cpp");
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-changed={}", shim.display());
        let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
        let copy = out.join("moira");
        std::fs::create_dir_all(&copy).expect("the build folder takes a copy");
        for entry in std::fs::read_dir(&source).expect("Moira's folder reads") {
            let path = entry.expect("an entry").path();
            if path.is_file() {
                std::fs::copy(&path, copy.join(path.file_name().unwrap()))
                    .expect("Moira's files copy");
            }
        }
        let config = copy.join("MoiraConfig.h");
        let mut text = std::fs::read_to_string(&config).expect("MoiraConfig.h reads");
        for (name, value) in [
            ("MOIRA_PRECISE_TIMING", "true"),
            ("MOIRA_MIMIC_MUSASHI", "false"),
            ("MOIRA_EMULATE_ADDRESS_ERROR", "true"),
        ] {
            let line = text
                .lines()
                .find(|line| line.starts_with(&format!("#define {name} ")))
                .unwrap_or_else(|| panic!("MoiraConfig.h defines {name}"))
                .to_string();
            text = text.replace(&line, &format!("#define {name} {value}"));
        }
        std::fs::write(&config, text).expect("MoiraConfig.h writes");
        let mut build = cc::Build::new();
        build
            .cpp(true)
            .std("c++20")
            .include(&copy)
            .define("NDEBUG", None)
            .warnings(false)
            .flag_if_supported("-w")
            .opt_level(2)
            .file(&shim);
        for file in [
            "Moira.cpp",
            "MoiraCore68000.cpp",
            "MoiraCore68010.cpp",
            "MoiraCore68020.cpp",
            "MoiraDebugger.cpp",
        ] {
            build.file(copy.join(file));
        }
        build.compile("moira");
    }
}

#[cfg(feature = "musashi")]
mod musashi {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub fn build() {
        println!("cargo:rerun-if-env-changed=S68K_MUSASHI");
        let source = match std::env::var_os("S68K_MUSASHI") {
            Some(path) => PathBuf::from(path),
            None => Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/musashi/fetched"),
        };
        if !source.join("m68k_in.c").exists() {
            panic!(
                "the `musashi` feature compiles Musashi, which is not at {}: run \
                 scripts/fetch-musashi.sh, or name a copy with S68K_MUSASHI",
                source.display()
            );
        }
        println!("cargo:rerun-if-changed={}", source.display());
        let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));

        // m68kmake writes the opcode handlers, for the host whatever the target
        let compiler = cc::Build::new()
            .host(&std::env::var("HOST").expect("cargo sets HOST"))
            .target(&std::env::var("HOST").expect("cargo sets HOST"))
            .opt_level(1)
            .cargo_metadata(false)
            .get_compiler();
        let generator = out.join("m68kmake");
        let built = compiler
            .to_command()
            .arg(source.join("m68kmake.c"))
            .arg("-o")
            .arg(&generator)
            .status()
            .expect("the host's C compiler runs");
        assert!(built.success(), "m68kmake does not compile");
        let generated = Command::new(&generator)
            .arg(&out)
            .arg(source.join("m68k_in.c"))
            .output()
            .expect("m68kmake runs");
        assert!(
            generated.status.success(),
            "m68kmake fails: {}",
            String::from_utf8_lossy(&generated.stderr)
        );

        cc::Build::new()
            .include(&out)
            .include(&source)
            // T takes a trace exception after the instruction, as on a 68010
            .define("M68K_EMULATE_TRACE", "M68K_OPT_ON")
            .file(source.join("m68kcpu.c"))
            .file(out.join("m68kops.c"))
            .file(source.join("m68kdasm.c"))
            .file(source.join("softfloat/softfloat.c"))
            .warnings(false)
            .flag_if_supported("-w")
            .opt_level(2)
            .compile("musashi");
    }
}
