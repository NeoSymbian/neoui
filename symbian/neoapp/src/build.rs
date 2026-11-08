// SPDX-License-Identifier: GPL-3.0-only
//! Which build is running, one line per part: the kernel's stamp (NeoSymbian HAL `ENeoHalBuildInfo`), the user-side
//! ROM's (`Z:\Resource\NeoSymbian\build.txt`, written by NeoSymbian/build) and neoui's own (compiled in, with the
//! e32-rs commit). Shown in Settings -> About phone, Dev tools and the UI server's boot log.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use e32::fs::{self, File, Fs};
use e32::sys::TPtr8;

/// `EHalGroupVariant` function: "kernel <commit>[+dirty] <UTC time> <board>" into a TDes8 (kernel a64_variant.cpp).
const BUILD_INFO: i32 = 0x4e0a;
const GROUP_VARIANT: i32 = 1;
const ROM_FILE: &str = r"Z:\Resource\NeoSymbian\build.txt";

/// Kernel, ROM and neoui, in that order; a part that can't be read says why.
pub fn lines() -> Vec<String> {
    vec![kernel(), rom(), String::from(neoui_proto::BUILD)]
}

fn kernel() -> String {
    let mut buf = [0u8; 160];
    let mut des = TPtr8::new(&mut buf);
    let des_ptr: *mut u8 = (&mut des as *mut TPtr8).cast();
    // SAFETY: the kernel writes into the TDes8, which lives across the call; a2 is unused.
    let r = unsafe {
        e32::sys::ui::hal_function(GROUP_VARIANT, BUILD_INFO, des_ptr, core::ptr::null_mut())
    };
    if r < 0 {
        // the 2010 kernel on Syborg has no NeoSymbian HAL functions
        return format!("kernel: no build info ({r})");
    }
    let n = des.len();
    String::from_utf8_lossy(&buf[..n]).into_owned()
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
