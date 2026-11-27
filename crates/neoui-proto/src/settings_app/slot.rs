// SPDX-License-Identifier: Apache-2.0
//! About phone's active A/B slot (settings-data.md D2): the kernel's slot line (HAL `0x4e09`, kernel
//! `qcom_bootctrl.cpp` `A64Slot_Show`) starts "current a; …" when it read and validated the GPT; anything else (a GPT
//! read error "GPT -> -N …", a board without slots "no A/B slots", a garbled line) is no reading.

/// The booted slot, 'a' or 'b', from the kernel's slot line.
#[must_use]
pub fn current(line: &str) -> Option<char> {
    let rest = line.strip_prefix("current ")?;
    let mut c = rest.chars();
    let slot = c.next().filter(|s| matches!(s, 'a' | 'b'))?;
    matches!(c.next(), None | Some(';')).then_some(slot)
}

#[cfg(test)]
mod tests {
    use super::current;

    #[test]
    fn reads_the_booted_slot() {
        let ok = "current b; a: prio 15 tries 0 ok; b: prio 15 active tries 6; GPT copies at 1 and 122879 agree";
        assert_eq!(current(ok), Some('b'));
        assert_eq!(current("current a"), Some('a'));
        // unreadable GPT, no slots, empty, garbled
        assert_eq!(current("GPT -> -20 (primary at 1, backup at 0)"), None);
        assert_eq!(current("no A/B slots"), None);
        assert_eq!(current(""), None);
        assert_eq!(current("current "), None);
        assert_eq!(current("current c; x"), None);
        assert_eq!(current("current ab"), None);
        assert_eq!(current(" current a"), None);
    }
}
