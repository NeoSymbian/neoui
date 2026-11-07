// SPDX-License-Identifier: Apache-2.0
//! Running apps in least-recently-used order (NeoSymbian R10): the UI server keeps every app it started, shows the
//! one whose screen is current, and asks the least recently used to quit when too many run.

use alloc::vec::Vec;

/// Items oldest first; the last one is the most recently used.
#[derive(Debug)]
pub struct Recents<T> {
    items: Vec<T>,
}

impl<T> Default for Recents<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T> Recents<T> {
    /// Add `item` as the most recent.
    pub fn push(&mut self, item: T) {
        self.items.push(item);
    }

    /// Make the first item matching `f` the most recent; false if none matches.
    pub fn touch(&mut self, f: impl Fn(&T) -> bool) -> bool {
        match self.items.iter().position(f) {
            Some(i) => {
                let item = self.items.remove(i);
                self.items.push(item);
                true
            }
            None => false,
        }
    }

    /// Take out every item matching `f`.
    pub fn remove(&mut self, f: impl Fn(&T) -> bool) -> Vec<T> {
        let (gone, kept) = self.items.drain(..).partition(|i| f(i));
        self.items = kept;
        gone
    }

    /// The items that make the list longer than `limit`, oldest first, skipping those `keep` protects; they stay in
    /// the list (the caller asks them to quit and removes them once they have).
    pub fn over_limit(&self, limit: usize, keep: impl Fn(&T) -> bool) -> impl Iterator<Item = &T> {
        let excess = self.items.len().saturating_sub(limit);
        self.items.iter().filter(move |i| !keep(i)).take(excess)
    }

    pub fn find(&self, f: impl Fn(&T) -> bool) -> Option<&T> {
        self.items.iter().find(|i| f(i))
    }

    pub fn find_mut(&mut self, f: impl Fn(&T) -> bool) -> Option<&mut T> {
        self.items.iter_mut().find(|i| f(i))
    }

    /// Most recent first.
    pub fn newest_first(&self) -> impl Iterator<Item = &T> {
        self.items.iter().rev()
    }

    /// Oldest first; `.rev()` for newest first.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.items.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn list(r: &Recents<&'static str>) -> Vec<&'static str> {
        r.newest_first().copied().collect()
    }

    #[test]
    fn most_recent_first_and_touch() {
        let mut r = Recents::default();
        for k in ["clock", "terminal", "files"] {
            r.push(k);
        }
        assert_eq!(list(&r), ["files", "terminal", "clock"]);
        assert!(r.touch(|k| *k == "clock"));
        assert_eq!(list(&r), ["clock", "files", "terminal"]);
        assert!(!r.touch(|k| *k == "phone"));
        assert_eq!(r.find(|k| k.starts_with('t')), Some(&"terminal"));
    }

    #[test]
    fn remove_and_limit() {
        let mut r = Recents::default();
        for k in ["a", "b", "c", "d", "e"] {
            r.push(k);
        }
        // one too many: the oldest goes, unless it is protected (e.g. the foreground app)
        let over: Vec<_> = r.over_limit(4, |_| false).copied().collect();
        assert_eq!(over, ["a"]);
        let over: Vec<_> = r.over_limit(4, |k| *k == "a").copied().collect();
        assert_eq!(over, ["b"]);
        assert_eq!(r.over_limit(5, |_| false).count(), 0);
        assert_eq!(r.remove(|k| *k == "a" || *k == "c"), vec!["a", "c"]);
        assert_eq!(list(&r), ["e", "d", "b"]);
        assert!(r.remove(|_| false).is_empty());
        assert_eq!(r.len(), 3);
    }
}
