// SPDX-License-Identifier: GPL-3.0-only
//! What the system is running and how much memory it has: the kernel HAL's memory and CPU facts, and the processes
//! and threads found by name (`TFindProcess` / `TFindThread` underneath), each opened just long enough to read it.
//!
//! shortcut: raw executive calls here, because e32 has no wrappers yet; move them to e32-rs (`e32::sysinfo`) with its
//! next release.

use alloc::string::String;
use alloc::vec::Vec;
use e32::sys::execs::slow;
use e32::sys::swi::exec4;
use e32::sys::ui::hal_function;
use e32::sys::{TPtr8, handle_close};

const GROUP_KERNEL: i32 = 0;
const GROUP_VARIANT: i32 = 1;
const KERNEL_MEMORY_INFO: i32 = 0;
const KERNEL_NUM_CPUS: i32 = 16;
const VARIANT_INFO: i32 = 0;
/// `TObjectType` (u32std.h).
const OBJ_THREAD: u32 = 0;
const OBJ_PROCESS: u32 = 1;
/// `TOwnerType::EOwnerThread`: the handle dies with this thread.
const OWNER_THREAD: u32 = 1;
const KMAX_FULL_NAME: usize = 256;
/// `TDesC8` type `EBuf` (a `TBuf8`: the data follows the header).
const EBUF: u32 = 3;

/// RAM and ROM, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub total_ram: u64,
    pub free_ram: u64,
    pub rom: u64,
}

/// `TMemoryInfoV1` (e32hal.h): six words.
#[repr(C)]
#[derive(Default)]
struct MemoryInfoV1 {
    total_ram: u32,
    total_rom: u32,
    max_free_ram: u32,
    free_ram: u32,
    internal_disk_ram: u32,
    rom_reprogrammable: i32,
}

/// The kernel's memory figures (`EKernelHalMemoryInfo`).
///
/// # Errors
/// The HAL's error.
pub fn memory() -> Result<Memory, i32> {
    let mut info = MemoryInfoV1::default();
    // SAFETY: a TPckg-style descriptor over `info`, which lives across the call; the kernel copies at most its size.
    let r = unsafe {
        let bytes = core::slice::from_raw_parts_mut(
            (&mut info as *mut MemoryInfoV1).cast::<u8>(),
            core::mem::size_of::<MemoryInfoV1>(),
        );
        let mut des = TPtr8::new_full(bytes);
        hal_function(
            GROUP_KERNEL,
            KERNEL_MEMORY_INFO,
            (&mut des as *mut TPtr8).cast(),
            core::ptr::null_mut(),
        )
    };
    if r < 0 {
        return Err(r);
    }
    // TInt fields: read as unsigned so a 4 GB phone is not negative
    Ok(Memory {
        total_ram: u64::from(info.total_ram),
        free_ram: u64::from(info.free_ram),
        rom: u64::from(info.total_rom),
    })
}

/// CPUs the kernel runs on (1 when it can't say).
#[must_use]
pub fn cpus() -> u32 {
    // SAFETY: this function takes no arguments and returns the count.
    let r = unsafe {
        hal_function(
            GROUP_KERNEL,
            KERNEL_NUM_CPUS,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    u32::try_from(r).ok().filter(|&n| n > 0).unwrap_or(1)
}

/// The fastest core's top clock (`EVariantHalVariantInfo`), in kHz.
#[must_use]
pub fn cpu_max_khz() -> Option<u32> {
    // TVariantInfoV01: TVersion (4), pad (4), TInt64 machine id (8), LED caps (4), clock kHz (4), speed factor (4)
    let mut info = [0u32; 8];
    // SAFETY: a descriptor over `info`, which lives across the call; the kernel copies at most 32 bytes.
    let r = unsafe {
        let bytes = core::slice::from_raw_parts_mut(info.as_mut_ptr().cast::<u8>(), 32);
        let mut des = TPtr8::new_full(bytes);
        hal_function(
            GROUP_VARIANT,
            VARIANT_INFO,
            (&mut des as *mut TPtr8).cast(),
            core::ptr::null_mut(),
        )
    };
    (r >= 0 && info[5] > 0).then_some(info[5])
}

/// A `TBuf8<KMaxFullName>`, what `Exec::ObjectNext` reads the pattern from and writes the name into.
#[repr(C)]
struct NameBuf {
    len_type: u32,
    max_len: i32,
    data: [u8; KMAX_FULL_NAME],
}

impl NameBuf {
    fn new(text: &[u8]) -> Self {
        let mut b = Self {
            len_type: EBUF << 28 | text.len() as u32,
            max_len: KMAX_FULL_NAME as i32,
            data: [0; KMAX_FULL_NAME],
        };
        b.data[..text.len()].copy_from_slice(text);
        b
    }

    fn text(&self) -> String {
        // the kernel never writes more than max_len
        let n = ((self.len_type & 0x0fff_ffff) as usize).min(KMAX_FULL_NAME);
        String::from_utf8_lossy(&self.data[..n]).into_owned()
    }
}

/// Each object of `kind` whose full name matches "*", opened: `f(full name, handle)`. The handle is closed after.
fn for_each(kind: u32, mut f: impl FnMut(String, i32)) {
    // TFindHandle: four words, zero to start; the kernel keeps its place in it (16 more bytes of slack)
    let mut find = [0u32; 8];
    loop {
        let mut name = NameBuf::new(b"*");
        // SAFETY: ObjectNext(TObjectType, TBuf8<KMaxFullName>&, TFindHandle&): both buffers live across the call.
        let r = unsafe {
            exec4::<{ slow::OBJECT_NEXT }>(
                kind,
                (&mut name as *mut NameBuf) as u32,
                find.as_mut_ptr() as u32,
                0,
            ) as i32
        };
        if r < 0 {
            return; // KErrNotFound: the end
        }
        // SAFETY: FindHandleOpen(TOwnerType, const TFindHandle&): the find handle the last call filled.
        let h = unsafe {
            exec4::<{ slow::FIND_HANDLE_OPEN }>(OWNER_THREAD, find.as_ptr() as u32, 0, 0) as i32
        };
        if h > 0 {
            f(name.text(), h);
            handle_close(h);
        }
    }
}

/// A thread, and the process that owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadInfo {
    /// The owning process's full name ("SYSSTART.EXE[10005901]0001").
    pub process: String,
    pub name: String,
    /// `TThreadPriority` (-30 null … 30 real time; 100+ absolute).
    pub priority: i32,
    /// CPU time used, in microseconds; None when the kernel does not measure it (`MONITOR_THREAD_CPU_TIME`).
    pub cpu_us: Option<u64>,
}

/// Every thread.
#[must_use]
pub fn threads() -> Vec<ThreadInfo> {
    let mut out = Vec::new();
    for_each(OBJ_THREAD, |full, h| {
        let (process, name) = match full.split_once("::") {
            Some((p, n)) => (String::from(p), String::from(n)),
            None => (String::new(), full),
        };
        // SAFETY: ThreadPriority takes the thread's handle.
        let priority = unsafe { exec4::<{ slow::THREAD_PRIORITY }>(h as u32, 0, 0, 0) as i32 };
        let mut t: i64 = 0;
        // SAFETY: ThreadGetCpuTime(TInt handle, Int64&): `t` lives across the call.
        let r = unsafe {
            exec4::<{ slow::THREAD_GET_CPU_TIME }>(h as u32, (&mut t as *mut i64) as u32, 0, 0)
                as i32
        };
        // KErrNotSupported (-5) unless the kernel is built with MONITOR_THREAD_CPU_TIME
        let cpu_us = if r == 0 { u64::try_from(t).ok() } else { None };
        out.push(ThreadInfo {
            process,
            name,
            priority,
            cpu_us,
        });
    });
    out
}

/// A process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    /// "SYSSTART.EXE[10005901]0001".
    pub full: String,
    pub id: u32,
    /// Code bytes, and data bytes (constant + initialised + zeroed), of its EXE (0 for the kernel).
    pub code: u32,
    pub data: u32,
}

impl ProcessInfo {
    /// "SYSSTART.EXE": the full name up to the UID.
    #[must_use]
    pub fn name(&self) -> &str {
        self.full.split('[').next().unwrap_or(&self.full)
    }
}

/// `TModuleMemoryInfo`: base and size of code, constant data, initialised and zeroed data.
#[repr(C)]
#[derive(Default)]
struct ModuleMemoryInfo {
    code_base: u32,
    code_size: u32,
    const_base: u32,
    const_size: u32,
    init_base: u32,
    init_size: u32,
    zero_base: u32,
    zero_size: u32,
}

/// Every process.
#[must_use]
pub fn processes() -> Vec<ProcessInfo> {
    let mut out = Vec::new();
    for_each(OBJ_PROCESS, |full, h| {
        // SAFETY: ProcessId takes the process's handle.
        let id = unsafe { exec4::<{ slow::PROCESS_ID }>(h as u32, 0, 0, 0) };
        let mut m = ModuleMemoryInfo::default();
        // SAFETY: ProcessGetMemoryInfo(TInt handle, TModuleMemoryInfo&): `m` lives across the call.
        let r = unsafe {
            exec4::<{ slow::PROCESS_GET_MEMORY_INFO }>(
                h as u32,
                (&mut m as *mut ModuleMemoryInfo) as u32,
                0,
                0,
            ) as i32
        };
        let ok = r >= 0;
        out.push(ProcessInfo {
            full,
            id,
            code: if ok { m.code_size } else { 0 },
            data: if ok {
                m.const_size
                    .saturating_add(m.init_size)
                    .saturating_add(m.zero_size)
            } else {
                0
            },
        });
    });
    out
}
