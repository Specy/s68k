//! The 68000's interrupt requests, **IRQs**, as EASy68K simulates them
//! without its hardware window (the asm-editor's `docs/design/m68k-machine.md`,
//! decisions 16 to 22, and its `docs/research/easy68k-machine-behaviour.md`
//! §5 and §7).
//!
//! **Seven levels, all autovectored.** A request at a level from 1 to 7 is
//! latched until it is taken, one bit a level, as EASy68K's `irq` keeps them:
//! a second request at a level already pending is the same request. Between
//! two instructions, the highest pending level above the status register's
//! interrupt mask is taken through its autovector, vector 24 plus the level,
//! at `$64` to `$7C`, and level 7 whatever the mask; the mask rises to the
//! level taken, so a handler is interrupted only by a higher level, or by
//! level 7 again. Vectored and spurious interrupts are left out, as EASy68K
//! leaves them out.
//!
//! **`stop`** loads the status register and waits for a request above the new
//! mask: the processor is **Stopped**, a status of its own, until one comes,
//! and it is taken at once.
//!
//! **The sources** are the program's to set up and the host's to deliver. The
//! program sets timers per level with `trap #15` task 32 and 6 in D1.B, and
//! asks for an IRQ on a key or a mouse event with tasks 62 and 60. Those
//! settings are the [`MachineState`] beside the latch and the stopped flag,
//! journaled like a register so that undo puts them back. The host runs the
//! timers on the Program's time and reports each firing and each event; the
//! Core decides what an event raises from the levels the program set.

use serde::{Deserialize, Serialize};

/// The vector before the first autovector, the spurious interrupt's, which
/// s68k never takes: the autovector of level `n` is this plus `n`.
pub const AUTOVECTOR_BASE: u8 = 24;

/// The level taken whatever the interrupt mask, the 68000's non-maskable one.
pub const NON_MASKABLE_LEVEL: u8 = 7;

/// The version task 32 with 4 in D1.B answers in D1.L, EASy68K's `VERSION`
/// (`def.h`): 5.16.1, the simulator s68k follows.
pub const SIMULATOR_VERSION: u32 = 0x0005_1001;

/// The autovector of an IRQ at `level`, 1 to 7: vectors 25 to 31.
pub const fn autovector(level: u8) -> u8 {
    AUTOVECTOR_BASE + level
}

/// The interrupt mask of a status register, its bits 10 to 8: a request is
/// taken only above it, but at level 7.
pub const fn mask_of(sr: u16) -> u8 {
    ((sr >> 8) & 7) as u8
}

/// Whether `level` is one of the seven a request can be at.
pub const fn is_level(level: u8) -> bool {
    level >= 1 && level <= 7
}

/// An event on the Screen that the program can ask an IRQ for, with task 62
/// for the keyboard and task 60 for the mouse: what the host reports, as
/// `deviceEvent('keyDown' | 'keyUp' | 'mouseDown' | 'mouseUp' | 'mouseMove')`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceEvent {
    /// A key pressed.
    KeyDown,
    /// A key released.
    KeyUp,
    /// A mouse button pressed.
    MouseDown,
    /// A mouse button released.
    MouseUp,
    /// The mouse moved.
    MouseMove,
}

/// The levels task 62 set for the keyboard's events, 0 for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardLevels {
    /// The level a key pressed raises.
    pub key_down: u8,
    /// The level a key released raises.
    pub key_up: u8,
}

/// The levels task 60 set for the mouse's events, 0 for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct MouseLevels {
    /// The level a button pressed raises.
    pub down: u8,
    /// The level a button released raises.
    pub up: u8,
    /// The level a move raises.
    #[serde(rename = "move")]
    pub moved: u8,
}

/// One timer of task 32's sixth part, as `getMachineState()` lists it: an IRQ
/// at `level` every `interval_ms` milliseconds of the Program's time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoIrq {
    /// The level the timer raises, 1 to 7.
    pub level: u8,
    /// How often it fires, in milliseconds, never 0.
    pub interval_ms: u32,
}

/// The machine's interrupt state: the IRQ latch, whether the processor is
/// stopped, and what the program set up with tasks 32, 60 and 62. It is what
/// `getMachineState()` answers, and what a `SetMachineState` mutation carries
/// before and after a change.
///
/// Everything but the latch is the program's: only its instructions change
/// it, and undo puts it back as the step found it. The latch is the host's
/// too, since a request arrives between two instructions from outside the
/// program; see [`MachineState::undone`] for what undo does with it.
///
/// It serialises as `{pendingIrqs, stopped, autoIrqs, keyboardLevels,
/// mouseLevels}`, the pending levels and the timers as lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MachineState {
    /// Bit `n - 1` is set while level `n` is pending.
    pending: u8,
    /// Whether `stop` has stopped the processor and nothing has woken it.
    stopped: bool,
    /// The interval of each level's timer in milliseconds, level 1 first, 0
    /// while it is off. EASy68K's timers are VCL `TTimer`s, which never fire
    /// at an interval of 0, so 0 is off there too.
    timers: [u32; 7],
    keyboard: KeyboardLevels,
    mouse: MouseLevels,
}

impl MachineState {
    /// Whether a request at `level` is waiting to be taken.
    pub fn is_pending(&self, level: u8) -> bool {
        is_level(level) && self.pending & (1 << (level - 1)) != 0
    }

    /// The pending levels, lowest first.
    pub fn pending_levels(&self) -> Vec<u8> {
        (1..=7).filter(|&level| self.is_pending(level)).collect()
    }

    /// Whether `stop` has stopped the processor and no IRQ has woken it.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// The timers that are on, by level.
    pub fn auto_irqs(&self) -> Vec<AutoIrq> {
        (1..=7)
            .filter_map(|level| {
                let interval_ms = self.timers[level as usize - 1];
                (interval_ms != 0).then_some(AutoIrq { level, interval_ms })
            })
            .collect()
    }

    /// The levels task 62 set for the keyboard.
    pub fn keyboard_levels(&self) -> KeyboardLevels {
        self.keyboard
    }

    /// The levels task 60 set for the mouse.
    pub fn mouse_levels(&self) -> MouseLevels {
        self.mouse
    }

    /// The level `event` raises, or `None` when the program asked for no IRQ
    /// on it.
    pub fn level_of(&self, event: DeviceEvent) -> Option<u8> {
        let level = match event {
            DeviceEvent::KeyDown => self.keyboard.key_down,
            DeviceEvent::KeyUp => self.keyboard.key_up,
            DeviceEvent::MouseDown => self.mouse.down,
            DeviceEvent::MouseUp => self.mouse.up,
            DeviceEvent::MouseMove => self.mouse.moved,
        };
        is_level(level).then_some(level)
    }

    /// The level taken now, with the status register at `sr`: the highest
    /// pending one when it is above the mask or is level 7, or `None`. Only
    /// the highest can be: a lower one is not above a mask the highest is not
    /// above.
    pub fn ready_level(&self, sr: u16) -> Option<u8> {
        if self.pending == 0 {
            return None;
        }
        let level = 8 - self.pending.leading_zeros() as u8;
        (level > mask_of(sr) || level == NON_MASKABLE_LEVEL).then_some(level)
    }

    /// This state with a request at `level`, 1 to 7, latched.
    pub fn latched(self, level: u8) -> MachineState {
        MachineState {
            pending: self.pending | (1 << (level - 1)),
            ..self
        }
    }

    /// This state with the request at `level` taken, which wakes a stopped
    /// processor.
    pub fn taken(self, level: u8) -> MachineState {
        MachineState {
            pending: self.pending & !(1 << (level - 1)),
            stopped: false,
            ..self
        }
    }

    /// This state with the processor stopped by `stop`.
    pub fn stopped(self) -> MachineState {
        MachineState {
            stopped: true,
            ..self
        }
    }

    /// This state after task 32's sixth part, EASy68K's `setAutoIRQ(D2.B,
    /// D3.L)` (`hardwareu.cpp`): 0 turns every timer off, 1 to 7 turns that
    /// level's off, and `$81` to `$87` turns that level's on every D3.L
    /// milliseconds. EASy68K ignores any other D2.B, and so does this.
    pub fn with_auto_irq(self, selector: u8, interval_ms: u32) -> MachineState {
        let mut timers = self.timers;
        match selector {
            0 => timers = [0; 7],
            1..=7 => timers[selector as usize - 1] = 0,
            0x81..=0x87 => timers[(selector & 7) as usize - 1] = interval_ms,
            _ => {}
        }
        MachineState { timers, ..self }
    }

    /// This state after task 60, EASy68K's mouse IRQs (`CODE9.CPP`): the level
    /// is D1 bits 15 to 8, and bits 0, 1 and 2 choose the events it is set for,
    /// a button down, a button up and a move; the others keep theirs, and a
    /// level of 0 turns the chosen ones off. A level above 7 makes the whole
    /// call do nothing, as there.
    pub fn with_mouse_levels(self, d1: u32) -> MachineState {
        let Some(level) = requested_level(d1) else {
            return self;
        };
        let mut mouse = self.mouse;
        if d1 & 1 != 0 {
            mouse.down = level;
        }
        if d1 & 2 != 0 {
            mouse.up = level;
        }
        if d1 & 4 != 0 {
            mouse.moved = level;
        }
        MachineState { mouse, ..self }
    }

    /// This state after task 62, EASy68K's keyboard IRQs: task 60's encoding,
    /// with bit 0 for a key down and bit 1 for a key up.
    pub fn with_keyboard_levels(self, d1: u32) -> MachineState {
        let Some(level) = requested_level(d1) else {
            return self;
        };
        let mut keyboard = self.keyboard;
        if d1 & 1 != 0 {
            keyboard.key_down = level;
        }
        if d1 & 2 != 0 {
            keyboard.key_up = level;
        }
        MachineState { keyboard, ..self }
    }

    /// The state undoing a step puts back, the step having changed `old` into
    /// `new`, and this being the state now.
    ///
    /// What the program set up and whether it was stopped are put back as
    /// `old` has them. The latch is put back only where the step changed it,
    /// which is a request it took: undoing an IRQ step makes its level pending
    /// again, so that running on takes it again. A request that arrived after
    /// the step, which the program never saw, stays pending: it is live input,
    /// as a key pressed is, and undo does not take it back.
    pub fn undone(self, old: &MachineState, new: &MachineState) -> MachineState {
        let changed = old.pending ^ new.pending;
        MachineState {
            pending: (self.pending & !changed) | (old.pending & changed),
            ..*old
        }
    }
}

/// The level in D1 bits 15 to 8 of task 60 or 62, or `None` when it is above
/// 7, which EASy68K reads as a call to ignore.
fn requested_level(d1: u32) -> Option<u8> {
    let level = (d1 >> 8) as u8;
    (level <= 7).then_some(level)
}

/// The shape `getMachineState()` answers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineStateView {
    pending_irqs: Vec<u8>,
    stopped: bool,
    auto_irqs: Vec<AutoIrq>,
    keyboard_levels: KeyboardLevels,
    mouse_levels: MouseLevels,
}

impl Serialize for MachineState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        MachineStateView {
            pending_irqs: self.pending_levels(),
            stopped: self.stopped,
            auto_irqs: self.auto_irqs(),
            keyboard_levels: self.keyboard,
            mouse_levels: self.mouse,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_highest_pending_level_above_the_mask_is_taken_and_level_7_always() {
        let state = MachineState::default().latched(2).latched(5);
        assert_eq!(state.pending_levels(), [2, 5]);
        assert_eq!(state.ready_level(0x2000), Some(5));
        assert_eq!(state.ready_level(0x2400), Some(5));
        assert_eq!(state.ready_level(0x2500), None, "5 is not above a mask of 5");
        assert_eq!(state.ready_level(0x2700), None);
        assert_eq!(state.taken(5).ready_level(0x2100), Some(2));
        let seven = MachineState::default().latched(7);
        assert_eq!(seven.ready_level(0x2700), Some(7), "level 7 is not maskable");
        assert_eq!(MachineState::default().ready_level(0x2000), None);
    }

    #[test]
    fn a_request_is_latched_once_and_taking_it_wakes_the_processor() {
        let state = MachineState::default().stopped().latched(3).latched(3);
        assert!(state.is_stopped());
        assert_eq!(state.pending_levels(), [3]);
        let taken = state.taken(3);
        assert!(!taken.is_stopped());
        assert!(taken.pending_levels().is_empty());
    }

    #[test]
    fn task_32_sets_and_clears_the_timers_as_set_auto_irq_does() {
        let state = MachineState::default()
            .with_auto_irq(0x82, 500)
            .with_auto_irq(0x85, 20);
        assert_eq!(
            state.auto_irqs(),
            [
                AutoIrq { level: 2, interval_ms: 500 },
                AutoIrq { level: 5, interval_ms: 20 }
            ]
        );
        assert_eq!(state.with_auto_irq(2, 0).auto_irqs().len(), 1);
        assert_eq!(state.with_auto_irq(0x80, 9), state, "$80 is no level");
        assert_eq!(state.with_auto_irq(0x88, 9), state);
        assert_eq!(state.with_auto_irq(8, 9), state);
        assert!(state.with_auto_irq(0, 0).auto_irqs().is_empty());
        assert!(
            MachineState::default().with_auto_irq(0x81, 0).auto_irqs().is_empty(),
            "an interval of 0 never fires"
        );
    }

    #[test]
    fn tasks_60_and_62_set_the_levels_of_the_events_they_choose() {
        let state = MachineState::default().with_mouse_levels(0x0103);
        assert_eq!(state.level_of(DeviceEvent::MouseDown), Some(1));
        assert_eq!(state.level_of(DeviceEvent::MouseUp), Some(1));
        assert_eq!(state.level_of(DeviceEvent::MouseMove), None);
        let state = state.with_mouse_levels(0x0304);
        assert_eq!(state.level_of(DeviceEvent::MouseMove), Some(3));
        assert_eq!(state.level_of(DeviceEvent::MouseDown), Some(1), "kept");
        let state = state.with_mouse_levels(0x0002);
        assert_eq!(state.level_of(DeviceEvent::MouseUp), None, "level 0 is off");
        assert_eq!(state.with_mouse_levels(0x0807), state, "above 7 is ignored");
        let keys = MachineState::default().with_keyboard_levels(0x0501);
        assert_eq!(keys.level_of(DeviceEvent::KeyDown), Some(5));
        assert_eq!(keys.level_of(DeviceEvent::KeyUp), None);
        let keys = keys.with_keyboard_levels(0x0706);
        assert_eq!(keys.level_of(DeviceEvent::KeyUp), Some(7));
        assert_eq!(keys.level_of(DeviceEvent::KeyDown), Some(5), "bit 2 is no key event");
        assert_eq!(keys.level_of(DeviceEvent::MouseDown), None);
    }

    #[test]
    fn undo_puts_back_what_the_step_changed_and_keeps_later_requests() {
        let before = MachineState::default().stopped().latched(2);
        let after = before.taken(2);
        //a request at 4 arrives after the step
        let now = after.latched(4);
        let undone = now.undone(&before, &after);
        assert_eq!(undone.pending_levels(), [2, 4]);
        assert!(undone.is_stopped());
        //a step that set a timer leaves the latch as it is now
        let set = before.with_auto_irq(0x81, 10);
        let undone = set.latched(6).undone(&before, &set);
        assert_eq!(undone.pending_levels(), [2, 6]);
        assert!(undone.auto_irqs().is_empty());
    }

    #[test]
    fn it_serialises_as_get_machine_state_answers_it() {
        let state = MachineState::default()
            .latched(1)
            .latched(6)
            .with_auto_irq(0x83, 250)
            .with_keyboard_levels(0x0201)
            .with_mouse_levels(0x0404);
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({
                "pendingIrqs": [1, 6],
                "stopped": false,
                "autoIrqs": [{"level": 3, "intervalMs": 250}],
                "keyboardLevels": {"keyDown": 2, "keyUp": 0},
                "mouseLevels": {"down": 0, "up": 0, "move": 4}
            })
        );
        assert_eq!(
            serde_json::from_value::<DeviceEvent>(serde_json::json!("mouseMove")).unwrap(),
            DeviceEvent::MouseMove
        );
    }
}
