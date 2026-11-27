// SPDX-License-Identifier: GPL-3.0-only
//! SE4 / D8: an app's capabilities as the kernel holds them (`RProcess::Open(TProcessId)` +
//! `Exec::ProcessSecurityInfo`, us_secure.cpp), read once its session binds (`host::bind`), for INFO 8's
//! `cap_source` 2 (`settings_info::Shelf::saw_caps`).

use e32::sys::execs::slow;
use e32::sys::swi::exec4;

/// `EOwnerThread`: the handle is this thread's, closed below.
const OWNER_THREAD: u32 = 1;

/// The capability set (`SSecurityInfo::iCaps`, 64 bits) of process `pid`, or `None` when it can't be opened (gone).
pub fn of_process(pid: u32) -> Option<u64> {
    // SAFETY: plain integers; the kernel returns a new handle or an error.
    let h = unsafe { exec4::<{ slow::PROCESS_OPEN_BY_ID }>(pid, OWNER_THREAD, 0, 0) } as i32;
    if h < 0 {
        return None;
    }
    let mut info = [0u32; 4];
    // SAFETY: `info` is 16 writable bytes, the size of SSecurityInfo (secure ID, vendor ID, two capability words).
    unsafe {
        exec4::<{ slow::PROCESS_SECURITY_INFO }>(h as u32, info.as_mut_ptr() as u32, 0, 0);
    }
    e32::sys::handle_close(h);
    Some(u64::from(info[2]) | u64::from(info[3]) << 32)
}
