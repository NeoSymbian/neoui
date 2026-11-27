// SPDX-License-Identifier: Apache-2.0
//! An owner's side of protocol 1.20's owner work (docs neoui-shell/protocol.md "Owner work"): a report the shell
//! hasn't acknowledged ([`Unsent`]) is kept exactly as it was and sent again, never made again by repeating the
//! work. The shell answers the same report with 0 (and retries its save); a refusal for good ends it.

use super::wire::{KERR_ALREADY_EXISTS, OwnerReport};
use crate::settings::kerr;

/// How many times a report is sent before the owner gives up (the shell interrupts the job when the owner goes).
pub const TRIES_MAX: u8 = 40;

/// A finished piece of work whose report the shell hasn't taken yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsent {
    pub ticket: u32,
    pub report: OwnerReport,
    tries: u8,
}

/// An answer that sending the same report again can't change: not the owner, no such ticket, a report that doesn't
/// fit, another report already taken.
#[must_use]
pub fn refused_for_good(e: i32) -> bool {
    matches!(
        e,
        kerr::PERMISSION_DENIED | kerr::NOT_FOUND | kerr::ARGUMENT | KERR_ALREADY_EXISTS
    )
}

impl Unsent {
    #[must_use]
    pub fn new(ticket: u32, report: OwnerReport) -> Self {
        Self {
            ticket,
            report,
            tries: 0,
        }
    }

    /// After a send answered `r`: `Some` to send the same report again later, `None` when it is done with
    /// (acknowledged, refused for good, or out of tries).
    #[must_use]
    pub fn after(mut self, r: Result<(), i32>) -> Option<Self> {
        self.tries = self.tries.saturating_add(1);
        match r {
            Ok(()) => None,
            Err(e) if refused_for_good(e) || self.tries >= TRIES_MAX => None,
            Err(_) => Some(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_app::wire::state;

    fn report() -> OwnerReport {
        OwnerReport {
            state: state::COMMITTED,
            ..OwnerReport::default()
        }
    }

    #[test]
    fn kept_until_taken_and_never_changed() {
        let u = Unsent::new(7, report());
        // the shell's save failed (KErrDiskFull), or it wasn't ready: the same report again
        let u = u.after(Err(-26)).unwrap().after(Err(-18)).unwrap();
        assert_eq!((u.ticket, &u.report), (7, &report()));
        assert_eq!(u.clone().after(Ok(())), None, "acknowledged");
        for e in [
            kerr::PERMISSION_DENIED,
            kerr::NOT_FOUND,
            kerr::ARGUMENT,
            KERR_ALREADY_EXISTS,
        ] {
            assert_eq!(u.clone().after(Err(e)), None, "{e}");
        }
    }

    #[test]
    fn bounded() {
        let mut u = Some(Unsent::new(1, report()));
        let mut sends = 0;
        while let Some(x) = u {
            sends += 1;
            u = x.after(Err(-26));
        }
        assert_eq!(sends, usize::from(TRIES_MAX));
    }
}
