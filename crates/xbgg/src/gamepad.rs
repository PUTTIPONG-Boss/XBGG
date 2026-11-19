//! Controller input via XInput, translated into UI actions.
//!
//! We poll XInput ourselves instead of using a gamepad library: the libraries
//! we evaluated keep a polling thread alive forever (one even leaked a thread
//! per re-initialisation). Here a single thread polls at 60 Hz only while
//! XBGG is in front, and merely checks the foreground window 4x per second
//! while a game is playing.
//!
//! Xbox-style controllers work natively; others (DualSense, Switch Pro, ...)
//! work through Steam Input or DS4Windows, which present them as XInput.

use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadAction {
    Left,
    Right,
    PageLeft,
    PageRight,
    Activate,
    Back,
    Add,
    Secondary,
    Refresh,
    Menu,
}

/// Buttons we use, named by position (Xbox layout labels in comments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    South,  // A
    East,   // B
    West,   // X
    North,  // Y
    DPadLeft,
    DPadRight,
    LeftBumper,
    RightBumper,
    Select, // View
    Start,  // Menu
}

/// XInput button bits (XINPUT_GAMEPAD_*), paired with our names.
const BUTTON_BITS: [(u16, Button); 10] = [
    (0x1000, Button::South),
    (0x2000, Button::East),
    (0x4000, Button::West),
    (0x8000, Button::North),
    (0x0004, Button::DPadLeft),
    (0x0008, Button::DPadRight),
    (0x0100, Button::LeftBumper),
    (0x0200, Button::RightBumper),
    (0x0020, Button::Select),
    (0x0010, Button::Start),
];

const STICK_PRESS: f32 = 0.6;
const STICK_RELEASE: f32 = 0.3;
const REPEAT_DELAY: Duration = Duration::from_millis(380);
const REPEAT_RATE: Duration = Duration::from_millis(110);
/// Poll interval while XBGG is in front.
const POLL: Duration = Duration::from_millis(16);
/// Check interval while something else (a game) is in front.
const IDLE_WAIT: Duration = Duration::from_millis(250);
/// XInputGetState on an empty slot is slow, so empty slots are probed rarely.
const PROBE_EMPTY_SLOTS: Duration = Duration::from_secs(1);
const MAX_PADS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    Pressed(Button),
    Released(Button),
    StickX(f32),
    Disconnected,
}

/// One controller reading, reduced to what we use.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PadState {
    pub buttons: u16,
    pub stick_x: f32,
}

/// Differences between two readings, as inputs.
pub fn diff(prev: PadState, cur: PadState, out: &mut Vec<Input>) {
    for (bit, button) in BUTTON_BITS {
        let (was, is) = (prev.buttons & bit != 0, cur.buttons & bit != 0);
        if is && !was {
            out.push(Input::Pressed(button));
        } else if was && !is {
            out.push(Input::Released(button));
        }
    }
    if cur.stick_x != prev.stick_x {
        out.push(Input::StickX(cur.stick_x));
    }
}

/// Turns raw input into actions, including hold-to-repeat for directions.
#[derive(Debug, Default)]
pub struct Repeater {
    held: Option<(PadAction, Instant)>,
    stick: Option<PadAction>,
}

impl Repeater {
    pub fn input(&mut self, input: Input, now: Instant) -> Option<PadAction> {
        match input {
            Input::Pressed(button) => {
                let action = match button {
                    Button::DPadLeft => return Some(self.hold(PadAction::Left, now)),
                    Button::DPadRight => return Some(self.hold(PadAction::Right, now)),
                    Button::LeftBumper => PadAction::PageLeft,
                    Button::RightBumper => PadAction::PageRight,
                    Button::South => PadAction::Activate,
                    Button::East => PadAction::Back,
                    Button::West => PadAction::Add,
                    Button::North => PadAction::Secondary,
                    Button::Select => PadAction::Refresh,
                    Button::Start => PadAction::Menu,
                };
                Some(action)
            }
            Input::Released(Button::DPadLeft | Button::DPadRight) => {
                self.held = None;
                None
            }
            Input::Released(_) => None,
            Input::StickX(value) => {
                let dir = if value <= -STICK_PRESS {
                    Some(PadAction::Left)
                } else if value >= STICK_PRESS {
                    Some(PadAction::Right)
                } else if value.abs() <= STICK_RELEASE {
                    None
                } else {
                    self.stick // dead band: keep the current state
                };
                if dir == self.stick {
                    return None;
                }
                self.stick = dir;
                match dir {
                    Some(d) => Some(self.hold(d, now)),
                    None => {
                        self.held = None;
                        None
                    }
                }
            }
            Input::Disconnected => {
                self.reset();
                None
            }
        }
    }

    /// Emits a repeat when a held direction is due.
    pub fn tick(&mut self, now: Instant) -> Option<PadAction> {
        let (dir, due) = self.held?;
        if now < due {
            return None;
        }
        self.held = Some((dir, now + REPEAT_RATE));
        Some(dir)
    }

    pub fn reset(&mut self) {
        self.held = None;
        self.stick = None;
    }

    fn hold(&mut self, dir: PadAction, now: Instant) -> PadAction {
        self.held = Some((dir, now + REPEAT_DELAY));
        dir
    }
}

/// Spawns the input thread. `is_active` says whether XBGG is in front;
/// `emit` is called from the input thread.
pub fn spawn(is_active: impl Fn() -> bool + Send + 'static, emit: impl Fn(PadAction) + Send + 'static) {
    thread::Builder::new()
        .name("xbgg-gamepad".into())
        .spawn(move || run(is_active, emit))
        .expect("failed to spawn gamepad thread");
}

fn run(is_active: impl Fn() -> bool, emit: impl Fn(PadAction)) {
    let mut rep = Repeater::default();
    // `None` = not connected, or no baseline yet since XBGG came to the front.
    let mut prev: [Option<PadState>; MAX_PADS] = [None; MAX_PADS];
    let mut connected = [false; MAX_PADS];
    let mut last_probe: Option<Instant> = None;
    let mut inputs = Vec::new();

    loop {
        if !is_active() {
            // Forget everything: buttons held in the game must not count as
            // presses when XBGG comes back.
            rep.reset();
            prev = [None; MAX_PADS];
            thread::sleep(IDLE_WAIT);
            continue;
        }

        let now = Instant::now();
        let probe = last_probe.is_none_or(|t| now.duration_since(t) >= PROBE_EMPTY_SLOTS);
        if probe {
            last_probe = Some(now);
        }

        for slot in 0..MAX_PADS {
            if !connected[slot] && !probe {
                continue;
            }
            match read_pad(slot as u32) {
                Some(state) => {
                    connected[slot] = true;
                    if let Some(p) = prev[slot] {
                        inputs.clear();
                        diff(p, state, &mut inputs);
                        for input in inputs.drain(..) {
                            if let Some(action) = rep.input(input, now) {
                                emit(action);
                            }
                        }
                    }
                    prev[slot] = Some(state);
                }
                None => {
                    if connected[slot] {
                        rep.input(Input::Disconnected, now);
                    }
                    connected[slot] = false;
                    prev[slot] = None;
                }
            }
        }

        if let Some(action) = rep.tick(now) {
            emit(action);
        }
        thread::sleep(POLL);
    }
}

#[cfg(windows)]
fn read_pad(slot: u32) -> Option<PadState> {
    use windows_sys::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
    // ERROR_SUCCESS = 0; anything else means no controller in this slot.
    if unsafe { XInputGetState(slot, &mut state) } != 0 {
        return None;
    }
    let lx = state.Gamepad.sThumbLX as f32 / 32767.0;
    Some(PadState { buttons: state.Gamepad.wButtons as u16, stick_x: lx.clamp(-1.0, 1.0) })
}

#[cfg(not(windows))]
fn read_pad(_slot: u32) -> Option<PadState> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_map_to_actions() {
        let mut r = Repeater::default();
        let t = Instant::now();
        assert_eq!(r.input(Input::Pressed(Button::South), t), Some(PadAction::Activate));
        assert_eq!(r.input(Input::Pressed(Button::East), t), Some(PadAction::Back));
        assert_eq!(r.input(Input::Pressed(Button::Start), t), Some(PadAction::Menu));
        assert_eq!(r.input(Input::Pressed(Button::LeftBumper), t), Some(PadAction::PageLeft));
        assert_eq!(r.tick(t + Duration::from_secs(5)), None, "face buttons never repeat");
    }

    #[test]
    fn dpad_repeats_after_delay_until_released() {
        let mut r = Repeater::default();
        let t = Instant::now();
        assert_eq!(r.input(Input::Pressed(Button::DPadRight), t), Some(PadAction::Right));
        assert_eq!(r.tick(t + REPEAT_DELAY / 2), None);
        assert_eq!(r.tick(t + REPEAT_DELAY), Some(PadAction::Right));
        assert_eq!(r.tick(t + REPEAT_DELAY + REPEAT_RATE / 2), None);
        assert_eq!(r.tick(t + REPEAT_DELAY + REPEAT_RATE), Some(PadAction::Right));
        r.input(Input::Released(Button::DPadRight), t + Duration::from_secs(1));
        assert_eq!(r.tick(t + Duration::from_secs(5)), None);
    }

    #[test]
    fn stick_has_hysteresis() {
        let mut r = Repeater::default();
        let t = Instant::now();
        assert_eq!(r.input(Input::StickX(0.5), t), None, "below press threshold");
        assert_eq!(r.input(Input::StickX(-0.9), t), Some(PadAction::Left));
        assert_eq!(r.input(Input::StickX(-0.95), t), None, "no duplicate while held");
        assert_eq!(r.input(Input::StickX(-0.45), t), None, "dead band keeps holding");
        assert_eq!(r.tick(t + REPEAT_DELAY), Some(PadAction::Left));
        assert_eq!(r.input(Input::StickX(0.1), t), None);
        assert_eq!(r.tick(t + Duration::from_secs(5)), None, "released inside release threshold");
    }

    #[test]
    fn disconnect_stops_repeat() {
        let mut r = Repeater::default();
        let t = Instant::now();
        r.input(Input::Pressed(Button::DPadLeft), t);
        r.input(Input::Disconnected, t);
        assert_eq!(r.tick(t + Duration::from_secs(2)), None);
    }

    #[test]
    fn diff_reports_edges_only() {
        let a = PadState { buttons: 0x1000, stick_x: 0.0 };
        let b = PadState { buttons: 0x1000 | 0x0008, stick_x: 0.7 };
        let mut out = Vec::new();
        diff(a, b, &mut out);
        assert_eq!(out, vec![Input::Pressed(Button::DPadRight), Input::StickX(0.7)]);
        out.clear();
        diff(b, a, &mut out);
        assert_eq!(out, vec![Input::Released(Button::DPadRight), Input::StickX(0.0)]);
        out.clear();
        diff(a, a, &mut out);
        assert!(out.is_empty(), "a held button is not a new press");
    }
}
