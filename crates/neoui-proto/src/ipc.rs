// SPDX-License-Identifier: Apache-2.0
//! The app ↔ UI server protocol (NeoSymbian R6). An app process connects to the Symbian IPC server [`SERVER`],
//! asks for its [`GEOMETRY`] and its drawing [`SURFACE`] (a shared chunk, `width * height` XRGB8888 pixels, stride =
//! width), draws, reports [`DAMAGE`], and keeps one [`NEXT_EVENT`] request outstanding for input and quit.

use crate::{InputEvent, TouchPhase};

/// The server's name (no leading `!`: that needs ProtServ).
pub const SERVER: &str = "NeoUi";
/// Protocol version (major, minor); a connect with another major is refused.
pub const VERSION: (u8, u8) = (1, 0);

/// `[Des8 out: GEOMETRY_LEN bytes = width, height, dp × 256 (u32 LE)]` → 0. `dp` is the shell's design unit in
/// pixels (`min(screen w, h) / 360`), so apps size things like the shell does.
pub const GEOMETRY: i32 = 1;
pub const GEOMETRY_LEN: usize = 12;
/// `[]` → completes with a handle (in the app's thread) to the surface chunk.
pub const SURFACE: i32 = 2;
/// `[x, y, w, h]` (surface pixels) → 0 once the server has shown them.
pub const DAMAGE: i32 = 3;
/// `[Des8 out: EVENT_LEN bytes]` → asynchronous: completes when there is an [`AppEvent`] for the app.
pub const NEXT_EVENT: i32 = 4;
/// `[show: 1 | 0]` → the height in pixels of the system keyboard now covering the bottom of the surface (0 when
/// hidden). Its keys arrive as [`InputEvent::Key`] like hardware keys (NeoSymbian R6c).
pub const KEYBOARD: i32 = 5;

/// Encoded size of an [`AppEvent`].
pub const EVENT_LEN: usize = 16;

/// What the server tells an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// Input, positions relative to the surface.
    Input(InputEvent),
    /// Leave now (the user went back or home); the app should exit.
    Quit,
}

const KIND_TOUCH: u32 = 1;
const KIND_KEY: u32 = 2;
const KIND_QUIT: u32 = 3;

impl AppEvent {
    /// `[kind, a, b, c]` as little-endian u32s: touch = (phase | id << 8, x, y), key = (down, code, 0).
    pub fn encode(&self) -> [u8; EVENT_LEN] {
        let w: [u32; 4] = match *self {
            AppEvent::Input(InputEvent::Touch { id, x, y, phase }) => {
                let p = match phase {
                    TouchPhase::Down => 0,
                    TouchPhase::Move => 1,
                    TouchPhase::Up => 2,
                };
                [KIND_TOUCH, p | u32::from(id) << 8, x as u32, y as u32]
            }
            AppEvent::Input(InputEvent::Key { code, down }) => [KIND_KEY, down as u32, code, 0],
            AppEvent::Quit => [KIND_QUIT, 0, 0, 0],
        };
        let mut out = [0u8; EVENT_LEN];
        for (i, v) in w.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub fn decode(b: &[u8; EVENT_LEN]) -> Option<Self> {
        let w = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        Some(match w(0) {
            KIND_TOUCH => AppEvent::Input(InputEvent::Touch {
                id: (w(1) >> 8) as u8,
                x: w(2) as i32,
                y: w(3) as i32,
                phase: match w(1) & 0xFF {
                    0 => TouchPhase::Down,
                    1 => TouchPhase::Move,
                    2 => TouchPhase::Up,
                    _ => return None,
                },
            }),
            KIND_KEY => AppEvent::Input(InputEvent::Key {
                code: w(2),
                down: w(1) != 0,
            }),
            KIND_QUIT => AppEvent::Quit,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip() {
        let all = [
            AppEvent::Input(InputEvent::Touch {
                id: 2,
                x: -5,
                y: 470,
                phase: TouchPhase::Move,
            }),
            AppEvent::Input(InputEvent::Key {
                code: 'é' as u32,
                down: true,
            }),
            AppEvent::Quit,
        ];
        for e in all {
            assert_eq!(AppEvent::decode(&e.encode()), Some(e));
        }
        assert_eq!(AppEvent::decode(&[0; EVENT_LEN]), None);
    }
}
