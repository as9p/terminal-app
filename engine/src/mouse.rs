//! Mouse actions to xterm mouse reports, in SGR form or the classic one-byte-per-value form.

use crate::geometry::CellPoint;
use crate::keys::Mods;
use crate::screen::{Modes, MouseTracking};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// Wheel counts lines, positive is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press(Button),
    Release(Button),
    Move(Option<Button>),
    Wheel(i32),
}

pub fn encode_mouse(
    action: MouseAction,
    at: CellPoint,
    mods: Mods,
    modes: &Modes,
) -> Option<Vec<u8>> {
    let tracking = modes.mouse;
    if tracking == MouseTracking::Off {
        return None;
    }
    let (code, pressed, count) = match action {
        MouseAction::Press(button) => (button_code(button), true, 1),
        // The classic form cannot say which button went up, so it sends 3 for any release.
        MouseAction::Release(button) if modes.sgr_mouse => (button_code(button), false, 1),
        MouseAction::Release(_) => (3, false, 1),
        MouseAction::Move(button) => {
            let reported = match button {
                Some(_) => matches!(tracking, MouseTracking::Drag | MouseTracking::Motion),
                None => tracking == MouseTracking::Motion,
            };
            if !reported {
                return None;
            }
            (32 + button.map_or(3, button_code), true, 1)
        }
        MouseAction::Wheel(0) => return None,
        MouseAction::Wheel(lines) => (if lines > 0 { 64 } else { 65 }, true, lines.unsigned_abs()),
    };
    let code = code + modifier_bits(mods);
    let x = u32::from(at.col) + 1;
    let y = u32::from(at.row) + 1;
    let report = if modes.sgr_mouse {
        let end = if pressed { 'M' } else { 'm' };
        format!("\x1b[<{code};{x};{y}{end}").into_bytes()
    } else {
        // Each value travels as one byte offset by 32, so a column or row past 223 cannot be sent.
        let byte = |v: u32| u8::try_from(v + 32).ok();
        vec![0x1b, b'[', b'M', byte(code)?, byte(x)?, byte(y)?]
    };
    Some(report.repeat(count as usize))
}

fn button_code(button: Button) -> u32 {
    match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    }
}

/// Super has no bit in mouse reports.
fn modifier_bits(mods: Mods) -> u32 {
    let mut bits = 0;
    if mods.contains(Mods::SHIFT) {
        bits |= 4;
    }
    if mods.contains(Mods::ALT) {
        bits |= 8;
    }
    if mods.contains(Mods::CTRL) {
        bits |= 16;
    }
    bits
}
