// SPDX-License-Identifier: Apache-2.0
//! SE4 / D8, the read-only capabilities view (docs neoui-shell/settings-data.md D8): what an app may do, from INFO 8's
//! `cap_source` and `caps` ([`AppDetail`]). The shell fills them: a running app's from the kernel's security info
//! (`cap_source` 2), a ROM app's from the build rule that gives every ROM EXE all capabilities (1), else unknown (0).
//! Nothing is inferred here: an unknown source reads as unknown, never as "None".

use super::wire::AppDetail;
use alloc::string::String;
use alloc::vec::Vec;

/// The 2010 kernel's capabilities by bit (`TCapability`, e32capability.h): 0 TCB … 19 UserEnvironment.
pub const NAMES: [&str; 20] = [
    "TCB",
    "CommDD",
    "PowerMgmt",
    "MultimediaDD",
    "ReadDeviceData",
    "WriteDeviceData",
    "DRM",
    "TrustedUI",
    "ProtServ",
    "DiskAdmin",
    "NetworkControl",
    "AllFiles",
    "SwEvent",
    "NetworkServices",
    "LocalServices",
    "ReadUserData",
    "WriteUserData",
    "Location",
    "SurroundingsDD",
    "UserEnvironment",
];

/// Every capability the kernel knows.
pub const ALL: u64 = (1 << NAMES.len()) - 1;

/// `cap_source` values.
pub mod source {
    pub const UNKNOWN: u8 = 0;
    /// the deployed image: a ROM EXE (every capability, the ROM build's rule) or its E32 header
    pub const IMAGE: u8 = 1;
    /// the kernel's security info of the running process
    pub const KERNEL: u8 = 2;
}

/// The names of the capabilities in `caps`, in bit order (bits past [`NAMES`] are ignored).
#[must_use]
pub fn names(caps: u64) -> Vec<&'static str> {
    NAMES
        .iter()
        .enumerate()
        .filter(|(i, _)| caps & (1 << i) != 0)
        .map(|(_, n)| *n)
        .collect()
}

/// The app page's "Permissions" row: `(value, subtitle)`. "All" / "None" / "3 of 20" and where it was read
/// ("Every ROM app gets all", "Read from the kernel while it ran: ReadUserData, …"); unknown says so.
#[must_use]
pub fn row(d: &AppDetail) -> (String, String) {
    let caps = d.caps & ALL;
    let value = match caps {
        _ if d.cap_source == source::UNKNOWN => String::new(),
        ALL => String::from("All"),
        0 => String::from("None"),
        c => alloc::format!("{} of {}", c.count_ones(), NAMES.len()),
    };
    let from = match d.cap_source {
        source::KERNEL => "Read from the kernel while it ran",
        source::IMAGE if caps == ALL => "Every ROM app gets all of them",
        source::IMAGE => "From its image",
        _ => return (value, String::from("Not known until it runs")),
    };
    let sub = match caps {
        ALL | 0 => String::from(from),
        c => alloc::format!("{from}: {}", names(c).join(", ")),
    };
    (value, sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(cap_source: u8, caps: u64) -> AppDetail {
        AppDetail {
            app: super::super::wire::AppRef::Native("files"),
            caption: "Files".into(),
            exe: String::new(),
            version: String::new(),
            identity_source: 2,
            sid: 0,
            cap_source,
            caps,
            coverage: 0,
            measured_utc: 0,
            clear_scope: 0,
            drives: Vec::new(),
        }
    }

    #[test]
    fn the_kernel_numbering() {
        assert_eq!(NAMES[0], "TCB");
        assert_eq!(NAMES[13], "NetworkServices");
        assert_eq!(NAMES[19], "UserEnvironment");
        assert_eq!(ALL, 0xF_FFFF);
        assert_eq!(
            names(1 << 15 | 1 << 13),
            ["NetworkServices", "ReadUserData"]
        );
        assert!(names(1 << 40).is_empty());
    }

    #[test]
    fn the_row_says_only_what_is_known() {
        assert_eq!(
            row(&detail(source::IMAGE, ALL)),
            ("All".into(), "Every ROM app gets all of them".into())
        );
        assert_eq!(
            row(&detail(source::KERNEL, ALL | 1 << 40)),
            ("All".into(), "Read from the kernel while it ran".into())
        );
        assert_eq!(
            row(&detail(source::KERNEL, 0)),
            ("None".into(), "Read from the kernel while it ran".into())
        );
        assert_eq!(
            row(&detail(source::KERNEL, 1 << 13 | 1 << 15)),
            (
                "2 of 20".into(),
                "Read from the kernel while it ran: NetworkServices, ReadUserData".into()
            )
        );
        // unknown is never "None"
        assert_eq!(
            row(&detail(source::UNKNOWN, 0)),
            (String::new(), "Not known until it runs".into())
        );
    }
}
