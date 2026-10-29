//! Musashi (MIT, <https://github.com/kstenerud/Musashi>) as a 68010, or a
//! 68EC020, beside the Interpreter: the reference of the differential harness
//! of the 68010 and 68020 dialects (the asm-editor's
//! `docs/design/m68k-machine.md`, decision 3, and planning decision 6 of its
//! plan), behind the `musashi` feature.
//!
//! `build.rs` compiles Musashi from the copy `scripts/fetch-musashi.sh`
//! fetched; this module declares the few of its functions the harness calls
//! and gives it the memory it reads and writes, through the callbacks its
//! `m68k.h` asks the host for. Musashi keeps one processor in global state, so
//! one test at a time may use it: [`Musashi::lock`] hands it out behind a
//! mutex, and the memory is the locking thread's.
//!
//! The tests are `src/test/differential.rs`'s, run with
//! `cargo test --release --features musashi differential -- --ignored`.

use std::cell::RefCell;
use std::sync::{Mutex, MutexGuard, Once};

pub use super::reference::State;

mod ffi {
    use std::ffi::c_void;
    extern "C" {
        pub fn m68k_init();
        pub fn m68k_set_cpu_type(cpu_type: u32);
        pub fn m68k_pulse_reset();
        pub fn m68k_execute(num_cycles: i32) -> i32;
        pub fn m68k_get_reg(context: *mut c_void, register: u32) -> u32;
        pub fn m68k_set_reg(register: u32, value: u32);
        pub fn m68k_is_valid_instruction(instruction: u32, cpu_type: u32) -> u32;
        pub fn m68k_set_irq(level: u32);
    }
}

/// Musashi's `M68K_CPU_TYPE_68000`.
pub const CPU_TYPE_68000: u32 = 1;
/// Musashi's `M68K_CPU_TYPE_68010`.
pub const CPU_TYPE_68010: u32 = 2;
/// Musashi's `M68K_CPU_TYPE_68EC020`: the 68020 with a 24-bit bus, which is
/// s68k's.
pub const CPU_TYPE_68EC020: u32 = 3;

/// The numbers of `m68k_register_t` the harness reads and writes.
mod register {
    pub const D0: u32 = 0;
    pub const A0: u32 = 8;
    pub const PC: u32 = 16;
    pub const SR: u32 = 17;
    pub const USP: u32 = 19;
    pub const ISP: u32 = 20;
    pub const MSP: u32 = 21;
    pub const SFC: u32 = 22;
    pub const DFC: u32 = 23;
    pub const VBR: u32 = 24;
    pub const CACR: u32 = 25;
    pub const CAAR: u32 = 26;
}

/// The 16 MB Musashi reads and writes, masked to the 24 bits of the 68010's
/// bus, and every byte it wrote since the last [`Musashi::load`].
struct Bus {
    memory: Vec<u8>,
    written: Vec<u32>,
    /// The bytes [`Musashi::load`] wrote, which the next load clears.
    loaded: Vec<(u32, usize)>,
}

thread_local! {
    static BUS: RefCell<Bus> = RefCell::new(Bus {
        memory: vec![0; 0x0100_0000],
        written: Vec::new(),
        loaded: Vec::new(),
    });
}

fn read(address: u32, bytes: u32) -> u32 {
    BUS.with(|bus| {
        let bus = bus.borrow();
        (0..bytes).fold(0, |value, byte| {
            value << 8 | bus.memory[(address.wrapping_add(byte) & 0x00FF_FFFF) as usize] as u32
        })
    })
}

fn write(address: u32, bytes: u32, value: u32) {
    BUS.with(|bus| {
        let mut bus = bus.borrow_mut();
        for byte in 0..bytes {
            let at = address.wrapping_add(byte) & 0x00FF_FFFF;
            bus.memory[at as usize] = (value >> (8 * (bytes - 1 - byte))) as u8;
            bus.written.push(at);
        }
    })
}

/// Musashi's read of a byte.
#[no_mangle]
pub extern "C" fn m68k_read_memory_8(address: u32) -> u32 {
    read(address, 1)
}
/// Musashi's read of a word.
#[no_mangle]
pub extern "C" fn m68k_read_memory_16(address: u32) -> u32 {
    read(address, 2)
}
/// Musashi's read of a long.
#[no_mangle]
pub extern "C" fn m68k_read_memory_32(address: u32) -> u32 {
    read(address, 4)
}
/// Musashi's disassembler's read of a byte.
#[no_mangle]
pub extern "C" fn m68k_read_disassembler_8(address: u32) -> u32 {
    read(address, 1)
}
/// Musashi's disassembler's read of a word.
#[no_mangle]
pub extern "C" fn m68k_read_disassembler_16(address: u32) -> u32 {
    read(address, 2)
}
/// Musashi's disassembler's read of a long.
#[no_mangle]
pub extern "C" fn m68k_read_disassembler_32(address: u32) -> u32 {
    read(address, 4)
}
/// Musashi's write of a byte.
#[no_mangle]
pub extern "C" fn m68k_write_memory_8(address: u32, value: u32) {
    write(address, 1, value)
}
/// Musashi's write of a word.
#[no_mangle]
pub extern "C" fn m68k_write_memory_16(address: u32, value: u32) {
    write(address, 2, value)
}
/// Musashi's write of a long.
#[no_mangle]
pub extern "C" fn m68k_write_memory_32(address: u32, value: u32) {
    write(address, 4, value)
}

static LOCK: Mutex<()> = Mutex::new(());
static INIT: Once = Once::new();

/// Musashi as a 68010, held by one test at a time.
pub struct Musashi {
    _held: MutexGuard<'static, ()>,
}

impl Musashi {
    /// Musashi, once the test that held it has let it go, made a 68010.
    pub fn lock() -> Musashi {
        let held = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        INIT.call_once(|| unsafe { ffi::m68k_init() });
        unsafe { ffi::m68k_set_cpu_type(CPU_TYPE_68010) };
        Musashi { _held: held }
    }

    /// Makes it a processor of `cpu_type`, until the next [`lock`](Musashi::lock)
    /// makes it a 68010 again.
    pub fn set_cpu_type(&mut self, cpu_type: u32) {
        unsafe { ffi::m68k_set_cpu_type(cpu_type) };
    }

    /// Whether Musashi's disassembler takes `word` as the first word of an
    /// instruction of `cpu_type`, [`CPU_TYPE_68000`] or [`CPU_TYPE_68010`].
    pub fn is_valid(&self, word: u16, cpu_type: u32) -> bool {
        unsafe { ffi::m68k_is_valid_instruction(word as u32, cpu_type) != 0 }
    }

    /// Puts `regions` in memory, after clearing what the last test loaded and
    /// wrote, and the processor in `state`: a reset, which clears a stopped
    /// processor and the cycles of the reset itself, then every register, the
    /// status register first, which chooses the stack pointer each of the
    /// three is written to.
    pub fn load(&mut self, state: &State, regions: &[(u32, Vec<u8>)]) {
        BUS.with(|bus| {
            let mut bus = bus.borrow_mut();
            let Bus {
                memory,
                written,
                loaded,
            } = &mut *bus;
            for &at in written.iter() {
                memory[at as usize] = 0;
            }
            for &(start, length) in loaded.iter() {
                memory[start as usize..start as usize + length].fill(0);
            }
            written.clear();
            loaded.clear();
            for (start, bytes) in regions {
                memory[*start as usize..*start as usize + bytes.len()].copy_from_slice(bytes);
                loaded.push((*start, bytes.len()));
            }
        });
        unsafe {
            ffi::m68k_pulse_reset();
            ffi::m68k_execute(0);
            ffi::m68k_set_reg(register::SR, state.sr as u32);
            ffi::m68k_set_reg(register::USP, state.usp);
            ffi::m68k_set_reg(register::ISP, state.ssp);
            ffi::m68k_set_reg(register::MSP, state.msp);
            ffi::m68k_set_reg(register::CACR, state.cacr);
            ffi::m68k_set_reg(register::CAAR, state.caar);
            for (index, value) in state.d.iter().enumerate() {
                ffi::m68k_set_reg(register::D0 + index as u32, *value);
            }
            for (index, value) in state.a.iter().enumerate() {
                ffi::m68k_set_reg(register::A0 + index as u32, *value);
            }
            ffi::m68k_set_reg(register::VBR, state.vbr);
            ffi::m68k_set_reg(register::SFC, state.sfc as u32);
            ffi::m68k_set_reg(register::DFC, state.dfc as u32);
            ffi::m68k_set_reg(register::PC, state.pc);
        }
        BUS.with(|bus| bus.borrow_mut().written.clear());
    }

    /// Runs one instruction, and answers the cycles Musashi counts for it.
    pub fn step(&mut self) -> u32 {
        unsafe { ffi::m68k_execute(1) as u32 }
    }

    /// Holds the interrupt request lines at `level`, 0 for none: Musashi
    /// takes a request above the interrupt mask, or one that rises to 7,
    /// through its autovector at the start of the next
    /// [`step`](Musashi::step), which then runs the handler's first
    /// instruction.
    pub fn set_irq(&mut self, level: u8) {
        unsafe { ffi::m68k_set_irq(level as u32) }
    }

    /// The registers as they are now.
    pub fn state(&self) -> State {
        let get = |register| unsafe { ffi::m68k_get_reg(std::ptr::null_mut(), register) };
        let mut state = State {
            d: [0; 8],
            a: [0; 7],
            usp: get(register::USP),
            ssp: get(register::ISP),
            msp: get(register::MSP),
            cacr: get(register::CACR),
            caar: get(register::CAAR),
            sr: get(register::SR) as u16,
            pc: get(register::PC),
            vbr: get(register::VBR),
            sfc: get(register::SFC) as u8,
            dfc: get(register::DFC) as u8,
        };
        for (index, value) in state.d.iter_mut().enumerate() {
            *value = get(register::D0 + index as u32);
        }
        for (index, value) in state.a.iter_mut().enumerate() {
            *value = get(register::A0 + index as u32);
        }
        state
    }

    /// The bytes at `address` and on, `length` of them.
    pub fn read(&self, address: u32, length: usize) -> Vec<u8> {
        (0..length as u32)
            .map(|offset| read(address.wrapping_add(offset), 1) as u8)
            .collect()
    }

    /// Every byte the last [`step`](Musashi::step) wrote, by address.
    pub fn written(&self) -> Vec<u32> {
        BUS.with(|bus| bus.borrow().written.clone())
    }
}
