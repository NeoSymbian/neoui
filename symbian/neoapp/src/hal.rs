// SPDX-License-Identifier: GPL-3.0-only
//! The NeoSymbian kernel's own `EHalGroupVariant` functions (docs/rust-integration.md "Dev tools"), typed. Boards
//! without them, and the 2010 kernel on Syborg, answer `KErrNotSupported` (-5); every function returns the error so
//! the caller can say so. Reads need ReadDeviceData, the modem and ADSP states CommDD: ROM EXEs hold both.

use alloc::string::String;
use alloc::vec::Vec;
use e32::sys::TPtr8;
use e32::sys::ui::hal_function;

const GROUP_VARIANT: i32 = 1;

/// The `EHalGroupVariant` function numbers (kernel `a64_variant.cpp`, `qcom_telhal.cpp`).
pub mod func {
    pub const LOG_READ: i32 = 0x4e00;
    pub const LAST_LOG_READ: i32 = 0x4e01;
    pub const CONSOLE_SET: i32 = 0x4e02;
    pub const CONSOLE_GET: i32 = 0x4e03;
    pub const SLOT_INFO: i32 = 0x4e09;
    pub const BUILD_INFO: i32 = 0x4e0a;
    pub const MODEM_STATE: i32 = 0x4e0e;
    pub const ADSP_STATE: i32 = 0x4e15;
    pub const DEVICE_LIST: i32 = 0x4e19;
}

/// Bytes one log or device-list read returns at most (the kernel's chunk).
const CHUNK: usize = 1024;

/// One call that fills a descriptor: the cursor in a1 and the descriptor in a2, or (no cursor) the descriptor in a1.
fn read_des(function: i32, cursor: Option<&mut u32>, buf: &mut [u8]) -> Result<usize, i32> {
    let mut des = TPtr8::new(buf);
    let des_ptr: *mut u8 = (&mut des as *mut TPtr8).cast();
    // SAFETY: the kernel functions take a TUint32* cursor and/or a TDes8*; both live across the call.
    let r = unsafe {
        match cursor {
            Some(c) => hal_function(GROUP_VARIANT, function, (c as *mut u32).cast(), des_ptr),
            None => hal_function(GROUP_VARIANT, function, des_ptr, core::ptr::null_mut()),
        }
    };
    if r < 0 { Err(r) } else { Ok(des.len()) }
}

/// Everything a cursor-paged function returns from `*cursor` on, appended to `out`; the cursor is left after it.
fn read_paged(function: i32, cursor: &mut u32, out: &mut Vec<u8>) -> Result<(), i32> {
    let mut buf = [0u8; CHUNK];
    loop {
        let n = read_des(function, Some(cursor), &mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// One line of text (up to 400 bytes).
fn read_text(function: i32) -> Result<String, i32> {
    let mut buf = [0u8; 400];
    let n = read_des(function, None, &mut buf)?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// This boot's kernel log from `*cursor` on (0 = the oldest kept); the cursor follows it.
///
/// # Errors
/// The HAL's error (-5 without the function).
pub fn log_read(cursor: &mut u32, out: &mut Vec<u8>) -> Result<(), i32> {
    read_paged(func::LOG_READ, cursor, out)
}

/// The previous boot's kernel log, whole.
///
/// # Errors
/// The HAL's error.
pub fn last_log() -> Result<Vec<u8>, i32> {
    let mut out = Vec::new();
    read_paged(func::LAST_LOG_READ, &mut 0, &mut out)?;
    Ok(out)
}

/// The kernel log drawn on screen (dev mode)?
///
/// # Errors
/// The HAL's error.
pub fn console_shown() -> Result<bool, i32> {
    let mut on: i32 = 0;
    // SAFETY: the kernel writes one TInt through a1.
    let r = unsafe {
        hal_function(
            GROUP_VARIANT,
            func::CONSOLE_GET,
            (&mut on as *mut i32).cast(),
            core::ptr::null_mut(),
        )
    };
    if r < 0 { Err(r) } else { Ok(on != 0) }
}

/// Shows or hides the kernel log on screen.
///
/// # Errors
/// The HAL's error.
pub fn set_console(on: bool) -> Result<(), i32> {
    // SAFETY: a1 is the value itself (1 on, 0 off).
    let r = unsafe {
        hal_function(
            GROUP_VARIANT,
            func::CONSOLE_SET,
            usize::from(on) as *mut u8,
            core::ptr::null_mut(),
        )
    };
    if r < 0 { Err(r) } else { Ok(()) }
}

/// The booted A/B slot and both slots' state, one line.
///
/// # Errors
/// The HAL's error.
pub fn slot_info() -> Result<String, i32> {
    read_text(func::SLOT_INFO)
}

/// "kernel <commit>[+dirty] <UTC time> <board>".
///
/// # Errors
/// The HAL's error.
pub fn build_info() -> Result<String, i32> {
    read_text(func::BUILD_INFO)
}

/// The kernel's device registry, all of it: lines `tag \t state \t name \t driver \t detail`
/// (`neoui_proto::devtools::device::parse_kernel_list` reads them).
///
/// # Errors
/// The HAL's error: -5 on kernels without the registry.
pub fn device_list() -> Result<String, i32> {
    let mut out = Vec::new();
    read_paged(func::DEVICE_LIST, &mut 0, &mut out)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Four words a state function writes through a1.
fn read_words(function: i32) -> Result<[u32; 4], i32> {
    let mut w = [0u32; 4];
    // SAFETY: the kernel writes four TUint32 through a1.
    let r = unsafe {
        hal_function(
            GROUP_VARIANT,
            function,
            w.as_mut_ptr().cast(),
            core::ptr::null_mut(),
        )
    };
    if r < 0 { Err(r) } else { Ok(w) }
}

/// The modem's boot as the kernel sees it: step (8 = running), error, SMP2P slave-kernel bits, IPCRTR open.
///
/// # Errors
/// The HAL's error (-5 without a modem).
pub fn modem_state() -> Result<[u32; 4], i32> {
    read_words(func::MODEM_STATE)
}

/// The audio DSP's boot: step (4 = running), error, SMP2P slave-kernel bits, open channels (bit 0 IPCRTR, 1 APR).
///
/// # Errors
/// The HAL's error (-5 without an ADSP).
pub fn adsp_state() -> Result<[u32; 4], i32> {
    read_words(func::ADSP_STATE)
}

/// Read-only PM660 battery observations. Per-field validity is preserved.
pub fn battery() -> Result<neoui_proto::battery::Snapshot, i32> {
    let mut buf = [0u8; neoui_proto::battery::Snapshot::SIZE];
    let n = read_des(0x4e1a, None, &mut buf)?;
    neoui_proto::battery::Snapshot::decode(&buf[..n]).ok_or(-20)
}

/// The latest Wi-Fi scan (kernel HAL WlanScan 0x4e1e, docs wifi.md W3); `rescan` asks the driver for a new one.
///
/// # Errors
/// The HAL's error (-5 without Wi-Fi, e.g. on QEMU).
pub fn wifi_scan(rescan: bool) -> Result<neoui_proto::wifi::Scan, i32> {
    let mut buf = alloc::vec![0u8; neoui_proto::wifi::SIZE];
    let mut des = TPtr8::new(&mut buf);
    let des_ptr: *mut u8 = (&mut des as *mut TPtr8).cast();
    // SAFETY: the kernel writes the TDes8 (its length is checked) and reads a2 only as a flag.
    let r = unsafe {
        hal_function(
            GROUP_VARIANT,
            0x4e1e,
            des_ptr,
            usize::from(rescan) as *mut u8,
        )
    };
    if r < 0 {
        return Err(r);
    }
    let n = des.len();
    neoui_proto::wifi::Scan::decode(&buf[..n]).ok_or(-20)
}

/// One Wi-Fi link operation (kernel HAL WlanLink 0x4e1f, docs wifi.md W4): a1 = the op, a2 = a TDes8 the kernel
/// reads (join, send, key) or fills (status, recv). `buf` holds the input, or is the output's room.
fn wifi_link(op: usize, buf: &mut [u8], input: bool) -> Result<usize, i32> {
    let mut des = if input {
        TPtr8::new_full(buf)
    } else {
        TPtr8::new(buf)
    };
    let des_ptr: *mut u8 = (&mut des as *mut TPtr8).cast();
    // SAFETY: the kernel reads or writes the TDes8 within its length/max length; a1 is only a number.
    let r = unsafe { hal_function(GROUP_VARIANT, 0x4e1f, op as *mut u8, des_ptr) };
    if r < 0 { Err(r) } else { Ok(des.len()) }
}

/// Join the scanned network `bssid`, sending our RSN element (`wpa::RSN_IE`) in the Association Request.
pub fn wifi_join(bssid: &[u8; 6], rsn: &[u8]) -> Result<(), i32> {
    let mut b = alloc::vec::Vec::from(&bssid[..]);
    b.extend_from_slice(rsn);
    wifi_link(1, &mut b, true).map(drop)
}

/// Leave the network (or stop a join) and go back to scanning.
pub fn wifi_leave() -> Result<(), i32> {
    wifi_link(2, &mut [], true).map(drop)
}

/// The link's state.
pub fn wifi_link_status() -> Result<neoui_proto::wifi::Link, i32> {
    let mut buf = [0u8; neoui_proto::wifi::LINK_SIZE];
    let n = wifi_link(3, &mut buf, false)?;
    neoui_proto::wifi::Link::decode(&buf[..n]).ok_or(-20)
}

/// The next EAPOL frame from the AP (802.1X header onward) into `buf`: its length, or `None` if none is waiting.
pub fn wifi_eapol_recv(buf: &mut [u8]) -> Result<Option<usize>, i32> {
    match wifi_link(4, buf, false) {
        Ok(n) => Ok(Some(n)),
        Err(-18) => Ok(None), // KErrNotReady
        Err(e) => Err(e),
    }
}

/// Send one EAPOL frame to the AP; returns once the chip reported it sent.
pub fn wifi_eapol_send(frame: &[u8]) -> Result<(), i32> {
    let mut b = alloc::vec::Vec::from(frame);
    wifi_link(5, &mut b, true).map(drop)
}

/// Install a CCMP key: the pairwise key, or the group key with its key index.
pub fn wifi_install_key(group: bool, index: u8, key: &[u8; 16]) -> Result<(), i32> {
    let mut b = [0u8; 18];
    b[0] = u8::from(group);
    b[1] = index;
    b[2..].copy_from_slice(key);
    wifi_link(6, &mut b, true).map(drop)
}

/// The handshake is done: let data through.
pub fn wifi_authorize() -> Result<(), i32> {
    wifi_link(7, &mut [], true).map(drop)
}

/// `buf.len()` (1..=256) bytes from the hardware random generator (kernel HAL Prng 0x4e23).
pub fn random(buf: &mut [u8]) -> Result<(), i32> {
    let mut des = TPtr8::new_full(buf);
    let des_ptr: *mut u8 = (&mut des as *mut TPtr8).cast();
    // SAFETY: the kernel fills the TDes8 to its current length.
    let r = unsafe { hal_function(GROUP_VARIANT, 0x4e23, des_ptr, core::ptr::null_mut()) };
    if r < 0 { Err(r) } else { Ok(()) }
}
