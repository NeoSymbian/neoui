// SPDX-License-Identifier: GPL-3.0-only
//! Dev tools' model: what every page shows, refreshed from the kernel and the servers as time passes. Nothing here
//! is saved; the app starts fresh each time.

use crate::probe::{Check, Readings};
use alloc::string::String;
use alloc::vec::Vec;
use neoapp::sys::{Memory, ProcessInfo, ThreadInfo};
use neoui_proto::devtools::device::Device;
use neoui_proto::devtools::log::Lines;
use neoui_proto::devtools::stats::{CpuSample, History};

/// Samples in a Home bar graph (one a second).
pub const HISTORY: usize = 30;

/// The tabs, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Home = 0,
    Devices = 1,
    Console = 2,
    Procs = 3,
    More = 4,
}

impl Tab {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Tab::Devices,
            2 => Tab::Console,
            3 => Tab::Procs,
            4 => Tab::More,
            _ => Tab::Home,
        }
    }
}

/// What covers the tabs, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    None,
    Device,
    Proc,
    Settings,
    Tools,
    About,
    TouchTest,
    DisplayTest,
}

impl View {
    /// The name `DevState.view` uses.
    pub fn name(self) -> &'static str {
        match self {
            View::None => "",
            View::Device => "device",
            View::Proc => "proc",
            View::Settings => "settings",
            View::Tools => "tools",
            View::About => "about",
            View::TouchTest => "touch-test",
            View::DisplayTest => "display-test",
        }
    }
}

/// The kernel's device registry, or why there is none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registry {
    /// Devices listed, malformed lines skipped.
    Listed { devices: usize, bad: usize },
    /// The HAL's error (-5: this kernel has no registry).
    Missing(i32),
}

/// The Console's view of the log.
#[derive(Debug)]
pub struct Console {
    /// The previous boot's log instead of this one.
    pub last: bool,
    /// Keep the newest line in view.
    pub follow: bool,
    /// The chosen tag chip: None = all lines.
    pub tag: Option<String>,
    /// The chips after "All": the most common tags.
    pub tags: Vec<String>,
    /// Rows scrolled back from the newest line.
    pub back: usize,
    /// How many lines matched at the last refresh (to hold a scrolled view still while lines arrive).
    pub matched: usize,
}

pub struct Model {
    pub tab: Tab,
    pub view: View,
    /// This boot's kernel log, followed.
    pub live: Lines,
    pub cursor: u32,
    pub log_error: Option<i32>,
    /// The previous boot's, read when first shown.
    pub last_boot: Option<Result<Lines, i32>>,
    pub console: Console,
    /// Rows of log text that fit the Console.
    pub console_rows: usize,
    pub readings: Readings,
    pub devices: Vec<Device>,
    pub registry: Registry,
    /// Results of the last "Run checks", by device key.
    pub checks: Vec<(String, Check)>,
    /// The device whose view is open (an index into `devices`).
    pub device: usize,
    pub dev_result: String,
    pub memory: Option<Memory>,
    pub mem_history: History,
    pub cpu_history: History,
    pub cpu_prev: Option<CpuSample>,
    pub cpu_load: Option<f32>,
    pub cpus: u32,
    pub cpu_khz: Option<u32>,
    pub procs: Vec<ProcessInfo>,
    pub threads: Vec<ThreadInfo>,
    /// The process whose view is open (an index into `procs`).
    pub proc: usize,
    pub tools_status: String,
    /// The power action waiting for its confirm.
    pub confirm: Option<i32>,
    /// The touch test's lit cells, and the last point.
    pub touch: Vec<bool>,
    pub touch_last: Option<(i32, i32)>,
    /// The display test's colour.
    pub fill: usize,
    /// The last key event (code, down).
    pub last_key: Option<(u32, bool)>,
    /// Kernel, ROM and neoui builds.
    pub build: Vec<String>,
    /// The app's surface, physical pixels.
    pub surface: (usize, usize),
}

impl Model {
    pub fn new(console_rows: usize, surface: (usize, usize)) -> Self {
        Self {
            tab: Tab::Home,
            view: View::None,
            live: Lines::new(),
            cursor: 0,
            log_error: None,
            last_boot: None,
            console: Console {
                last: false,
                follow: true,
                tag: None,
                tags: Vec::new(),
                back: 0,
                matched: 0,
            },
            console_rows,
            readings: Readings::default(),
            devices: Vec::new(),
            registry: Registry::Missing(0),
            checks: Vec::new(),
            device: 0,
            dev_result: String::new(),
            memory: None,
            mem_history: History::new(HISTORY),
            cpu_history: History::new(HISTORY),
            cpu_prev: None,
            cpu_load: None,
            cpus: 1,
            cpu_khz: None,
            procs: Vec::new(),
            threads: Vec::new(),
            proc: 0,
            tools_status: String::new(),
            confirm: None,
            touch: Vec::new(),
            touch_last: None,
            fill: 0,
            last_key: None,
            build: neoapp::build::lines(),
            surface,
        }
    }

    /// The check result for device `key`, if checks ran.
    pub fn check(&self, key: &str) -> Option<&Check> {
        self.checks.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }
}
