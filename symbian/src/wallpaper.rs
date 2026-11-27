// SPDX-License-Identifier: GPL-3.0-only
//! (D7) Wallpaper pictures (protocol.md `SETTINGS_ACTION` 8 `INSTALL_WALLPAPER`): Settings sends a 540×1080 RGB888
//! raster in a file it opened (slots 2/3); the shell checks the caller and the request (`contract::wallpaper_request`),
//! copies the file into its own private folder (staged, then `RFs::Replace` over the installed one, so a failure
//! leaves the old picture), then sets `display.wallpaper` to `picture` and hands the pixels to home. At boot the
//! installed one is read back.
// shortcut: the copy (1.7 MB) runs in the IPC handler, a few hundred ms once per install, and completes 0 when it is
// in place rather than with protocol.md's ticket: a worker and INFO 13's ticket if an install ever shows in a frame.

use super::{Host, read_bytes};
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, SharedFile, mode};
use e32::server::Message;
use e32::sys::err;
use neoui_proto::contract;
use neoui_proto::settings::{Caller, Value, access, kerr, keys};
use neoui_proto::settings_app::wallpaper::{self, BYTES};

const DIR: &str = "C:\\private\\e0002001\\";
const PATH: &str = "C:\\private\\e0002001\\wallpaper.rgb";
const STAGING: &str = "C:\\private\\e0002001\\wallpaper.new";

/// The installed picture on its way to home: the frame loop takes it (`lib.rs`).
#[derive(Default)]
pub struct Wallpaper {
    pub pending: Option<Vec<u8>>,
}

impl Wallpaper {
    /// The installed picture, if there is a whole one.
    pub fn boot() -> Self {
        let read = || -> Result<Vec<u8>, i32> {
            let fs = Fs::connect()?;
            let f = File::open(&fs, PATH, mode::READ | mode::SHARE_READERS_ONLY)?;
            read_all(&f)
        };
        let pending = match read() {
            Ok(px) => {
                dprintln!("neoui: wallpaper picture loaded");
                Some(px)
            }
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => None,
            Err(e) => {
                dprintln!(
                    "neoui: wallpaper picture unreadable ({}), the colour stays",
                    e
                );
                None
            }
        };
        Wallpaper { pending }
    }
}

/// All of `f`, which must be exactly one raster ([`BYTES`]); a short read is a failed one.
fn read_all(f: &File<'_>) -> Result<Vec<u8>, i32> {
    if f.size()? != BYTES as i64 {
        return Err(kerr::ARGUMENT);
    }
    let mut px = Vec::new();
    px.try_reserve_exact(BYTES)
        .map_err(|_| err::KERR_NO_MEMORY)?;
    px.resize(BYTES, 0);
    let mut got = 0;
    while got < BYTES {
        let n = f.read_at(got as i32, &mut px[got..BYTES.min(got + 64 * 1024)])?;
        if n == 0 {
            return Err(kerr::CORRUPT);
        }
        got += n;
    }
    Ok(px)
}

/// The raster staged and put in place of the installed one in one file server operation.
fn store(px: &[u8]) -> Result<(), i32> {
    let fs = Fs::connect()?;
    match fs.mkdir(DIR) {
        Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
        Err(e) => return Err(e),
    }
    {
        let f = File::replace(&fs, STAGING, mode::SHARE_EXCLUSIVE)?;
        f.write(px)?;
        f.flush()?;
    } // closed before the replace
    let r = fs.replace_file(STAGING, PATH);
    if r.is_err() {
        let _ = fs.delete(STAGING);
    }
    r
}

impl Host {
    /// `SETTINGS_ACTION` 8 from app `key`: 0 once the picture is in place and `display.wallpaper` says so.
    pub(super) fn install_wallpaper(&mut self, key: &str, m: &Message) -> i32 {
        let caller = access::caller(key, m.client_sid());
        if let Err(e) = contract::wallpaper_request(caller, |max| {
            read_bytes(m, 1, max).map_err(|_| kerr::ARGUMENT)
        }) {
            dprintln!("neoui: wallpaper from {} refused ({})", key, e);
            return e;
        }
        let file = match SharedFile::adopt_from_client(m, 2, 3) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let r = read_all(&file.file()).and_then(|px| store(&px).map(|()| px));
        let px = match r {
            Ok(px) => px,
            Err(e) => {
                dprintln!("neoui: wallpaper not installed ({}), the old one stays", e);
                return e;
            }
        };
        let now = e32::time::millis();
        let set = crate::settings::set(
            &mut self.settings,
            Caller::Shell,
            keys::DISPLAY_WALLPAPER.name(),
            Value::Str(wallpaper::KEY.into()),
            now,
        );
        if let Err(e) = set {
            return e.to_symbian();
        }
        dprintln!("neoui: wallpaper picture installed ({} bytes)", px.len());
        self.wallpaper.pending = Some(px);
        0
    }
}
