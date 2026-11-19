//! Controller input on a background thread, translated into UI actions.

use std::thread;
use std::time::{Duration, Instant};

use gilrs::{Axis, Button, EventType, Gilrs};

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
}

const STICK_PRESS: f32 = 0.6;
const STICK_RELEASE: f32 = 0.3;
const REPEAT_DELAY: Duration = Duration::from_millis(380);
const REPEAT_RATE: Duration = Duration::from_millis(110);
const POLL: Duration = Duration::from_millis(8);

/// Spawns the input thread. `emit` is called from that thread.
pub fn spawn(emit: impl Fn(PadAction) + Send + 'static) {
    thread::Builder::new()
        .name("xbgg-gamepad".into())
        .spawn(move || run(emit))
        .expect("failed to spawn gamepad thread");
}

fn run(emit: impl Fn(PadAction)) {
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(gilrs::Error::NotImplemented(g)) => g,
        Err(e) => {
            eprintln!("xbgg: controller support unavailable: {e}");
            return;
        }
    };

    // Direction currently held (d-pad or stick) and when it next repeats.
    let mut held: Option<(PadAction, Instant)> = None;
    let mut stick: Option<PadAction> = None;

    loop {
        while let Some(ev) = gilrs.next_event() {
            match ev.event {
                EventType::ButtonPressed(button, _) => match button {
                    Button::DPadLeft => press(&mut held, PadAction::Left, &emit),
                    Button::DPadRight => press(&mut held, PadAction::Right, &emit),
                    Button::LeftTrigger => emit(PadAction::PageLeft),
                    Button::RightTrigger => emit(PadAction::PageRight),
                    Button::South => emit(PadAction::Activate),
                    Button::East => emit(PadAction::Back),
                    Button::West => emit(PadAction::Add),
                    Button::North => emit(PadAction::Secondary),
                    Button::Select => emit(PadAction::Refresh),
                    _ => {}
                },
                EventType::ButtonReleased(Button::DPadLeft | Button::DPadRight, _) => held = None,
                EventType::AxisChanged(Axis::LeftStickX, value, _) => {
                    let dir = if value <= -STICK_PRESS {
                        Some(PadAction::Left)
                    } else if value >= STICK_PRESS {
                        Some(PadAction::Right)
                    } else if value.abs() <= STICK_RELEASE {
                        None
                    } else {
                        stick // dead band between press and release: keep state
                    };
                    if dir != stick {
                        stick = dir;
                        match dir {
                            Some(d) => press(&mut held, d, &emit),
                            None => held = None,
                        }
                    }
                }
                EventType::Disconnected => {
                    held = None;
                    stick = None;
                }
                _ => {}
            }
        }

        if let Some((dir, next)) = held {
            let now = Instant::now();
            if now >= next {
                emit(dir);
                held = Some((dir, now + REPEAT_RATE));
            }
        }
        thread::sleep(POLL);
    }
}

fn press(held: &mut Option<(PadAction, Instant)>, dir: PadAction, emit: &impl Fn(PadAction)) {
    emit(dir);
    *held = Some((dir, Instant::now() + REPEAT_DELAY));
}
