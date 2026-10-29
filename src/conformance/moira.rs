//! Moira (MIT, <https://github.com/dirkwhoffmann/Moira>) as a 68010 beside the
//! Interpreter: the reference of the differential harness for cycles, behind
//! the `moira` feature.
//!
//! Musashi counts a 68010's cycles from a table, with the manual's maxima
//! where the manual gives maxima, and has no loop mode. Moira counts them
//! access by access, its 68010 is checked against cputester on hardware, and
//! it has loop mode: what the design asks of the 68010's cycles ("Moira checks
//! the 68010's loop mode and timing"). `build.rs` compiles its core with
//! precise timing, without its mimicry of Musashi and with address errors,
//! and `tests/moira/shim.cpp`, the C interface this module calls. Moira's
//! processor is one, so one test at a time may use it: [`Moira::lock`] hands
//! it out behind a mutex.

use std::sync::{Mutex, MutexGuard, Once};

pub use super::reference::State;

mod ffi {
    extern "C" {
        pub fn s68k_moira_memory() -> *mut u8;
        pub fn s68k_moira_init();
        #[allow(clippy::too_many_arguments)]
        pub fn s68k_moira_load(
            d: *const u32,
            a: *const u32,
            usp: u32,
            ssp: u32,
            sr: u16,
            pc: u32,
            vbr: u32,
            sfc: u8,
            dfc: u8,
        );
        pub fn s68k_moira_step() -> i64;
        #[allow(clippy::too_many_arguments)]
        pub fn s68k_moira_state(
            d: *mut u32,
            a: *mut u32,
            usp: *mut u32,
            ssp: *mut u32,
            sr: *mut u16,
            pc: *mut u32,
            vbr: *mut u32,
            sfc: *mut u8,
            dfc: *mut u8,
        );
        pub fn s68k_moira_written(addresses: *mut *const u32) -> usize;
    }
}

static LOCK: Mutex<()> = Mutex::new(());
static INIT: Once = Once::new();

/// Moira as a 68010, held by one test at a time.
pub struct Moira {
    _held: MutexGuard<'static, ()>,
    /// What the last load put in memory, which the next clears.
    loaded: Vec<(u32, usize)>,
}

impl Moira {
    /// Moira, once the test that held it has let it go.
    pub fn lock() -> Moira {
        let held = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        INIT.call_once(|| unsafe { ffi::s68k_moira_init() });
        Moira {
            _held: held,
            loaded: Vec::new(),
        }
    }

    /// The 16 MB Moira reads and writes.
    fn memory(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(ffi::s68k_moira_memory(), 0x0100_0000) }
    }

    /// Every byte written since the last load, by address.
    pub fn written(&self) -> Vec<u32> {
        let mut addresses = std::ptr::null();
        let length = unsafe { ffi::s68k_moira_written(&mut addresses) };
        match length {
            0 => Vec::new(),
            _ => unsafe { std::slice::from_raw_parts(addresses, length) }.to_vec(),
        }
    }

    /// Puts `regions` in memory, after clearing what the last test loaded and
    /// wrote, and the processor in `state`, the instruction at its program
    /// counter in the prefetch queue.
    pub fn load(&mut self, state: &State, regions: &[(u32, Vec<u8>)]) {
        let written = self.written();
        let loaded = std::mem::take(&mut self.loaded);
        let memory = self.memory();
        for at in written {
            memory[at as usize] = 0;
        }
        for (start, length) in loaded {
            memory[start as usize..start as usize + length].fill(0);
        }
        for (start, bytes) in regions {
            memory[*start as usize..*start as usize + bytes.len()].copy_from_slice(bytes);
        }
        self.loaded = regions
            .iter()
            .map(|(start, bytes)| (*start, bytes.len()))
            .collect();
        unsafe {
            ffi::s68k_moira_load(
                state.d.as_ptr(),
                state.a.as_ptr(),
                state.usp,
                state.ssp,
                state.sr,
                state.pc,
                state.vbr,
                state.sfc,
                state.dfc,
            )
        };
    }

    /// Runs one instruction, and answers the cycles Moira counts for it, or
    /// `u32::MAX` when an exception of Moira's own escaped it: an address
    /// error in loop mode, which its loop dispatch does not catch.
    pub fn step(&mut self) -> u32 {
        unsafe { ffi::s68k_moira_step() as u32 }
    }

    /// The registers as they are now.
    pub fn state(&self) -> State {
        let mut state = State {
            d: [0; 8],
            a: [0; 7],
            usp: 0,
            ssp: 0,
            sr: 0,
            pc: 0,
            vbr: 0,
            sfc: 0,
            dfc: 0,
            //a 68010's, which has none of them
            msp: 0,
            cacr: 0,
            caar: 0,
        };
        unsafe {
            ffi::s68k_moira_state(
                state.d.as_mut_ptr(),
                state.a.as_mut_ptr(),
                &mut state.usp,
                &mut state.ssp,
                &mut state.sr,
                &mut state.pc,
                &mut state.vbr,
                &mut state.sfc,
                &mut state.dfc,
            )
        };
        state
    }

    /// The bytes at `address` and on, `length` of them.
    pub fn read(&mut self, address: u32, length: usize) -> Vec<u8> {
        let memory = self.memory();
        (0..length as u32)
            .map(|offset| memory[(address.wrapping_add(offset) & 0x00FF_FFFF) as usize])
            .collect()
    }
}
