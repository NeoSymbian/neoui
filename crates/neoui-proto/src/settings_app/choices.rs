// SPDX-License-Identifier: Apache-2.0
//! The fixed choices of the Settings app's sheets, with the setting value each stands for.

/// Display → Screen timeout (`display.timeout`, seconds; 0 = never).
pub const TIMEOUTS: [(i32, &str); 7] = [
    (15, "15 seconds"),
    (30, "30 seconds"),
    (60, "1 minute"),
    (120, "2 minutes"),
    (300, "5 minutes"),
    (600, "10 minutes"),
    (0, "Never"),
];

/// The row's value for a stored timeout, which may not be one of [`TIMEOUTS`] (another writer, an old store).
#[must_use]
pub fn timeout_text(seconds: i32) -> alloc::string::String {
    match TIMEOUTS.iter().find(|(s, _)| *s == seconds) {
        Some((_, t)) => (*t).into(),
        None if seconds < 60 => alloc::format!("{seconds} seconds"),
        None => alloc::format!("{} minutes", seconds / 60),
    }
}

/// The index of a stored timeout in [`TIMEOUTS`] (-1 when it isn't one), for the sheet's tick.
#[must_use]
pub fn timeout_index(seconds: i32) -> i32 {
    TIMEOUTS
        .iter()
        .position(|(s, _)| *s == seconds)
        .map_or(-1, |i| i as i32)
}

/// Home & navigation (`nav.mode` 0..=2): the name and what the edges do (`set2-navigation`).
pub const NAV_MODES: [(&str, &str); 3] = [
    (
        "Buttons",
        "Keys at the bottom: Back, Home, Open apps. No edge gestures.",
    ),
    (
        "Gestures",
        "Either side edge: Back. Up from the bottom: Home; pause: Open apps; along it: switch apps.",
    ),
    (
        "N9 gestures",
        "Left edge: Back. Right edge: Open apps; flick: last app. Up: Home; keep going: Apps menu.",
    ),
];

/// Sounds → Media → Play media on (`sound.media_route` 0..=2).
pub const MEDIA_ROUTES: [&str; 3] = ["Both speakers", "Earpiece", "Bottom speaker"];
/// Sounds → Media → Microphone (`sound.mic` 0..=1).
pub const MICS: [&str; 2] = ["Main", "Second"];

/// Old apps' key pads (`legacy.pad.*`, read by LEGACY.EXE).
pub const PAD_STRENGTHS: [&str; 3] = ["Light", "Medium", "Strong"];
pub const PAD_DEFAULTS: [&str; 4] = ["Auto", "Navigator", "Gamepad", "Number"];
pub const PAD_DIRS: [&str; 2] = ["Arrows", "Number keys"];
pub const PAD_A: [&str; 2] = ["OK", "5"];
pub const PAD_B: [&str; 3] = ["Right softkey", "0", "#"];

/// The system keyboard's languages: the setting that turns each on, and its name (`ui/components/keyboard/
/// layouts.slint` keeps the same order; `keyboard.layout` is an index into it).
pub const KEYBOARDS: [(&str, &str, &str); 3] = [
    ("keyboard.en", "English", "QWERTY"),
    ("keyboard.hy", "Armenian", "Phonetic"),
    ("keyboard.ru", "Russian", "ЙЦУКЕН"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts() {
        assert_eq!(timeout_text(60), "1 minute");
        assert_eq!(timeout_text(0), "Never");
        assert_eq!(timeout_text(45), "45 seconds");
        assert_eq!(timeout_text(900), "15 minutes");
        assert_eq!(timeout_index(300), 4);
        assert_eq!(timeout_index(45), -1);
    }
}
