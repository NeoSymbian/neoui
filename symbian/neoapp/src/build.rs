// SPDX-License-Identifier: GPL-3.0-only
//! Which build is running, one line per part: the kernel's stamp (NeoSymbian HAL `ENeoHalBuildInfo`), the user-side
//! ROM's (`Z:\Resource\NeoSymbian\build.txt`, written by NeoSymbian/build) and neoui's own (compiled in, with the
//! e32-rs commit). Shown in Settings -> About phone, Dev tools and the UI server's boot log.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use e32::fs::{self, File, Fs};

const ROM_FILE: &str = r"Z:\Resource\NeoSymbian\build.txt";

/// Kernel, ROM and neoui, in that order; a part that can't be read says why.
pub fn lines() -> Vec<String> {
    vec![kernel(), rom(), String::from(neoui_proto::BUILD)]
}

fn kernel() -> String {
    // the 2010 kernel on Syborg has no NeoSymbian HAL functions
    crate::hal::build_info().unwrap_or_else(|r| format!("kernel: no build info ({r})"))
}

fn rom() -> String {
    let read = || -> Result<String, i32> {
        let fs = Fs::connect()?;
        let file = File::open(&fs, ROM_FILE, fs::mode::SHARE_READERS_ONLY)?;
        let mut buf = [0u8; 160];
        let n = file.read_at(0, &mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..n]).trim().into())
    };
    read().unwrap_or_else(|e| format!("rom: no build info ({e})"))
}
