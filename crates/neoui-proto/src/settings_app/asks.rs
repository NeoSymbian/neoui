// SPDX-License-Identifier: Apache-2.0
//! The Settings app's questions before a step that can't be undone (SE4a), and what an app's Clear data deletes. They
//! are here so a test can check each one fits the shell's dialog (`page::Query`, at most `page::QUERY_MAX` bytes).

use crate::ipc;
use alloc::format;
use alloc::string::String;

/// What an app's Clear data deletes, `""` when it has no such row: the app clears its own
/// ([`crate::apps::clears_data`]), or MODEMD clears Phone's call history and Messages' texts, which it owns.
#[must_use]
pub fn clear_what(key: &str) -> &'static str {
    match key {
        "clock" => "Alarms, timers, stopwatch, world clocks, its settings",
        "phone" => "The call history (contacts stay)",
        "messages" => "Every message but blocked senders'",
        k if crate::apps::clears_data(k) => "Its data and its own settings",
        _ => "",
    }
}

/// Stop: the row says what may be lost.
#[must_use]
pub fn stop(name: &str) -> String {
    format!("Stop {name}?")
}

/// Clear data for the app called `name` with screen key `key`.
#[must_use]
pub fn clear(name: &str, key: &str) -> String {
    format!(
        "Delete {name}'s data? {}. This can't be undone.",
        clear_what(key)
    )
}

/// Reset ([`ipc::RESET_NETWORK`] or [`ipc::RESET_SETTINGS`]).
#[must_use]
pub fn reset(what: u8) -> &'static str {
    if what == ipc::RESET_NETWORK {
        "Forget every saved Wi-Fi network and its password? Contacts, messages, alarms and files stay. The phone restarts."
    } else {
        "Put the phone's settings back to their defaults? Sound profiles, networks, contacts, messages, alarms and files stay. The phone restarts."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Query;
    use alloc::string::ToString;
    use alloc::vec;

    fn fits(text: &str) {
        let q = Query {
            text: text.to_string(),
            buttons: vec!["Reset".to_string(), "Cancel".to_string()],
            destructive: true,
        };
        assert!(q.encode().is_ok(), "{} bytes: {text}", text.len());
    }

    #[test]
    fn every_question_fits_the_dialog() {
        fits(reset(ipc::RESET_NETWORK));
        fits(reset(ipc::RESET_SETTINGS));
        for a in crate::apps::listed() {
            fits(&stop(a.name));
            if !clear_what(a.key).is_empty() {
                fits(&clear(a.name, a.key));
            }
        }
        assert_eq!(clear_what("files"), "");
    }
}
