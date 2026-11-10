// SPDX-License-Identifier: Apache-2.0
//! The phone's components and their status. Three sources, best first:
//! 1. the kernel's device registry (HAL `ENeoHalDeviceList` = 0x4e19, `kernel/baseport/arm64/common/inc/a64_device.h`):
//!    one line per driver, `tag \t state \t name \t driver \t detail`;
//! 2. probes the app runs itself (a HAL value, a server's status, a drive's volume info);
//! 3. the kernel log: a driver that logged under its tag is there, even if nothing reports its state.
//!
//! [`CATALOGUE`] lists every component a phone has, so the parts without a driver still show as "No driver".

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A component's state, worst last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Ok,
    /// Works with a known gap (polled, no data path yet).
    Partial,
    Starting,
    /// Found, but off or not started on purpose.
    Off,
    /// Nothing reports a state, but its driver logged.
    Unknown,
    /// No driver on this board.
    NoDriver,
    /// The driver is there, the part was not found.
    Absent,
    Error,
}

impl State {
    /// The kernel registry's word for it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "ok" => Self::Ok,
            "partial" => Self::Partial,
            "starting" => Self::Starting,
            "off" => Self::Off,
            "absent" => Self::Absent,
            "error" => Self::Error,
            _ => return None,
        })
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Partial => "Partial",
            Self::Starting => "Starting",
            Self::Off => "Off",
            Self::Unknown => "Unknown",
            Self::NoDriver => "No driver",
            Self::Absent => "Not found",
            Self::Error => "Error",
        }
    }

    /// A state a developer should look at.
    #[must_use]
    pub fn is_problem(self) -> bool {
        matches!(self, Self::Absent | Self::Error)
    }
}

/// Where a component's state came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Kernel,
    Probe,
    Log,
    None,
}

impl Source {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Probe => "probe",
            Self::Log => "log",
            Self::None => "",
        }
    }
}

/// The sections of the Devices page, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Core,
    DisplayInput,
    Storage,
    Connectivity,
    Power,
    Audio,
    SensorsMedia,
    /// Kernel devices the catalogue does not know yet.
    Other,
}

impl Group {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Core => "Core",
            Self::DisplayInput => "Display and input",
            Self::Storage => "Storage",
            Self::Connectivity => "Connectivity",
            Self::Power => "Power",
            Self::Audio => "Audio",
            Self::SensorsMedia => "Sensors and media",
            Self::Other => "Other",
        }
    }
}

/// A part every phone has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    /// Stable id: probes and the app's tests use it.
    pub key: &'static str,
    pub name: &'static str,
    pub group: Group,
    /// Kernel registry tags that report this component (one per board's driver).
    pub kernel: &'static [&'static str],
    /// Log tags its drivers and servers write (`[touch]`, `[modemd]`).
    pub log: &'static [&'static str],
}

const fn c(
    key: &'static str,
    name: &'static str,
    group: Group,
    kernel: &'static [&'static str],
    log: &'static [&'static str],
) -> Component {
    Component {
        key,
        name,
        group,
        kernel,
        log,
    }
}

/// Every component, in page order.
pub static CATALOGUE: &[Component] = &[
    c("cpu", "CPU clocks", Group::Core, &["cpufreq"], &["cpufreq"]),
    c("memory", "Memory", Group::Core, &[], &[]),
    c("rtc", "Real-time clock", Group::Core, &["rtc"], &["rtc"]),
    c("slot", "A/B boot slots", Group::Core, &["slot"], &["slot"]),
    c(
        "display",
        "Display",
        Group::DisplayInput,
        &["display"],
        &["display"],
    ),
    c(
        "touch",
        "Touchscreen",
        Group::DisplayInput,
        &["touch"],
        &["touch"],
    ),
    c(
        "keys",
        "Buttons and keys",
        Group::DisplayInput,
        &["keys", "kbd"],
        &["keys", "kbd", "spmi"],
    ),
    c(
        "storage",
        "Storage",
        Group::Storage,
        &["emmc", "blk"],
        &["emmc", "blk", "media", "verity"],
    ),
    c(
        "modem",
        "Modem",
        Group::Connectivity,
        &["mss"],
        &["mss", "modemd", "glink", "qrtr"],
    ),
    c(
        "ipa",
        "Packet engine (IPA)",
        Group::Connectivity,
        &["ipa"],
        &["ipa"],
    ),
    c(
        "usb",
        "USB",
        Group::Connectivity,
        &["usb"],
        &["usb", "usbnet"],
    ),
    c("wifi", "Wi-Fi", Group::Connectivity, &[], &[]),
    c("bluetooth", "Bluetooth", Group::Connectivity, &[], &[]),
    c("gnss", "Location (GNSS)", Group::Connectivity, &[], &[]),
    c("battery", "Battery and charger", Group::Power, &[], &[]),
    c("backlight", "Backlight", Group::Power, &[], &[]),
    c("vibrator", "Vibrator", Group::Power, &[], &[]),
    c("thermal", "Thermal", Group::Power, &[], &[]),
    c(
        "adsp",
        "Audio DSP",
        Group::Audio,
        &["adsp"],
        &["adsp", "apr"],
    ),
    c(
        "audio",
        "Speaker, earpiece, mic",
        Group::Audio,
        &["audio"],
        &["audio"],
    ),
    c("sensors", "Sensors", Group::SensorsMedia, &[], &[]),
    c("camera", "Camera", Group::SensorsMedia, &[], &[]),
    c("gpu", "GPU", Group::SensorsMedia, &[], &[]),
];

/// The catalogue entry `key`.
#[must_use]
pub fn component(key: &str) -> Option<&'static Component> {
    CATALOGUE.iter().find(|c| c.key == key)
}

/// One line of the kernel's device registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelDevice {
    pub tag: String,
    pub state: State,
    pub name: String,
    pub driver: String,
    pub detail: String,
}

/// The registry's text (several HAL reads joined). Malformed lines are skipped; the second value counts them.
#[must_use]
pub fn parse_kernel_list(text: &str) -> (Vec<KernelDevice>, usize) {
    let mut list = Vec::new();
    let mut bad = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut f = line.splitn(5, '\t');
        let parsed = (|| {
            let tag = f.next()?.trim();
            let state = State::parse(f.next()?.trim())?;
            let name = f.next()?.trim();
            let driver = f.next()?.trim();
            let detail = f.next().unwrap_or("").trim();
            (!tag.is_empty()).then(|| KernelDevice {
                tag: tag.to_string(),
                state,
                name: name.to_string(),
                driver: driver.to_string(),
                detail: detail.to_string(),
            })
        })();
        match parsed {
            Some(d) => list.push(d),
            None => bad += 1,
        }
    }
    (list, bad)
}

/// What one of the app's own probes found for catalogue component `key`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub key: &'static str,
    pub state: State,
    pub detail: String,
}

/// A row of the Devices page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The catalogue key, or the kernel tag for a device the catalogue does not know.
    pub key: String,
    pub name: String,
    pub group: Group,
    /// The kernel driver's file, when the kernel reported it.
    pub driver: String,
    pub state: State,
    pub detail: String,
    pub source: Source,
    /// The log tags to show in its detail view.
    pub log: &'static [&'static str],
}

/// Every catalogue component with its best state (kernel, then probe, then "its tag is in the log"), then the kernel
/// devices the catalogue does not know, under [`Group::Other`].
#[must_use]
pub fn merge(
    kernel: &[KernelDevice],
    probes: &[Probe],
    logged: impl Fn(&str) -> bool,
) -> Vec<Device> {
    let mut rows: Vec<Device> = CATALOGUE
        .iter()
        .map(|c| {
            let from_kernel = kernel.iter().find(|k| c.kernel.contains(&k.tag.as_str()));
            let (state, detail, driver, source) = if let Some(k) = from_kernel {
                (k.state, k.detail.clone(), k.driver.clone(), Source::Kernel)
            } else if let Some(p) = probes.iter().find(|p| p.key == c.key) {
                (p.state, p.detail.clone(), String::new(), Source::Probe)
            } else if let Some(tag) = c.log.iter().find(|t| logged(t)) {
                let detail = alloc::format!("its driver logged [{tag}]; no status reported");
                (State::Unknown, detail, String::new(), Source::Log)
            } else {
                (State::NoDriver, String::new(), String::new(), Source::None)
            };
            Device {
                key: c.key.to_string(),
                name: c.name.to_string(),
                group: c.group,
                driver,
                state,
                detail,
                source,
                log: c.log,
            }
        })
        .collect();
    for k in kernel {
        if !CATALOGUE.iter().any(|c| c.kernel.contains(&k.tag.as_str())) {
            rows.push(Device {
                key: k.tag.clone(),
                name: k.name.clone(),
                group: Group::Other,
                driver: k.driver.clone(),
                state: k.state,
                detail: k.detail.clone(),
                source: Source::Kernel,
                log: &[],
            });
        }
    }
    rows
}

/// The Devices page's headline counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub ok: usize,
    pub partial: usize,
    /// Errors and parts not found.
    pub problems: usize,
    pub no_driver: usize,
    /// Off, starting or unknown.
    pub other: usize,
}

impl Summary {
    #[must_use]
    pub fn of(rows: &[Device]) -> Self {
        let mut s = Self::default();
        for r in rows {
            match r.state {
                State::Ok => s.ok += 1,
                State::Partial => s.partial += 1,
                State::NoDriver => s.no_driver += 1,
                st if st.is_problem() => s.problems += 1,
                _ => s.other += 1,
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "touch\tok\tTouchscreen (Synaptics RMI4)\ta64_touch\t1080x2160, one finger\n\
                        emmc\tpartial\tStorage (eMMC)\tqcom_sdhci\t59640 MB, HS52\n\
                        garbage line\n\
                        adsp\tsleeping\tAudio DSP\tqcom_adsp\tx\n\
                        npu\terror\tNeural engine\tqcom_npu\tfirmware missing\n\
                        rtc\toff\tRTC\tqcom_rtc\n";

    #[test]
    fn the_catalogue_has_unique_keys_and_tags() {
        for (i, c) in CATALOGUE.iter().enumerate() {
            assert!(
                CATALOGUE[i + 1..].iter().all(|d| d.key != c.key),
                "{}",
                c.key
            );
            for t in c.kernel {
                let owners = CATALOGUE.iter().filter(|d| d.kernel.contains(t)).count();
                assert_eq!(owners, 1, "kernel tag {t}");
            }
        }
        assert!(
            CATALOGUE.windows(2).all(|w| w[0].group <= w[1].group),
            "page order"
        );
    }

    #[test]
    fn registry_lines_parse_and_bad_ones_are_counted() {
        let (list, bad) = parse_kernel_list(LIST);
        assert_eq!(bad, 2, "no tabs, and an unknown state word");
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].tag, "touch");
        assert_eq!(list[0].state, State::Ok);
        assert_eq!(list[0].detail, "1080x2160, one finger");
        assert_eq!(list[3].detail, "", "a missing detail is empty");
        assert_eq!(parse_kernel_list("").0.len(), 0);
    }

    #[test]
    fn merge_prefers_the_kernel_then_probes_then_the_log() {
        let (kernel, _) = parse_kernel_list(LIST);
        let probes = [
            Probe {
                key: "touch",
                state: State::Error,
                detail: "ignored".into(),
            },
            Probe {
                key: "modem",
                state: State::Ok,
                detail: "LTE, 3 bars".into(),
            },
        ];
        let rows = merge(&kernel, &probes, |tag| tag == "usb");
        let row = |k: &str| rows.iter().find(|r| r.key == k).expect(k);
        assert_eq!(
            (row("touch").state, row("touch").source),
            (State::Ok, Source::Kernel)
        );
        assert_eq!(row("touch").driver, "a64_touch");
        assert_eq!(
            (row("modem").state, row("modem").source),
            (State::Ok, Source::Probe)
        );
        assert_eq!(
            (row("usb").state, row("usb").source),
            (State::Unknown, Source::Log)
        );
        assert_eq!(
            (row("wifi").state, row("wifi").source),
            (State::NoDriver, Source::None)
        );
        assert_eq!(row("rtc").state, State::Off);
        let npu = row("npu");
        assert_eq!((npu.group, npu.state), (Group::Other, State::Error));
        assert_eq!(rows.len(), CATALOGUE.len() + 1);
    }

    #[test]
    fn the_summary_counts_each_kind() {
        let (kernel, _) = parse_kernel_list(LIST);
        let rows = merge(&kernel, &[], |_| false);
        let s = Summary::of(&rows);
        assert_eq!((s.ok, s.partial, s.problems), (1, 1, 1));
        assert_eq!(s.other, 1, "rtc off");
        assert_eq!(
            s.ok + s.partial + s.problems + s.no_driver + s.other,
            rows.len()
        );
    }
}
