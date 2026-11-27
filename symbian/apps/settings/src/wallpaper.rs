// SPDX-License-Identifier: GPL-3.0-only
//! (D7) Wallpaper pictures (set2-wallpaper "From Files"): Files' picker for an image; the picked file (C: or E:, at
//! most 16 MiB) is decoded on a worker (files' `neo-format-image`: JPEG, PNG, GIF, WebP, BMP), turned by its EXIF
//! orientation, and fitted to cover 540×1080 (`settings_app::wallpaper::cover`) into this app's private folder; then
//! the shell takes that file (`SETTINGS_ACTION` 8, `App::settings_action_file`), installs it and sets
//! `display.wallpaper` to `picture`. Anything that fails leaves the old wallpaper.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering::SeqCst};
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use neo_format_image::{self as img, shrink};
use neoapp::App;
use neoui_proto::ipc as proto;
use neoui_proto::open::files;
use neoui_proto::settings_app::wallpaper::{self as wp, BYTES, H, W};
use settings_ui::{SetState, SettingsApp};
use slint::ComponentHandle;

/// The biggest file read: a 12 MP JPEG is about 5 MiB.
const FILE_MAX: usize = 16 << 20;
/// The most a decode may hold (`neo_format_image::cost`): about 7 MP of RGB. With the file and the fitted copy that
/// stays well inside this app's 64 MB heap, which never shrinks.
// shortcut: bigger photos are refused ("too large"); decode_into + a row-wise fit if people pick 12 MP photos.
const DECODE_MAX: usize = 20 << 20;
/// The fitted raster before the shell takes it, in this app's private folder (NEOSET's SID).
const STAGED: &str = "C:\\private\\e0002007\\wallpaper.rgb";

/// A pick is out with Files: its answer comes when this app is shown again (tone_import's flag is the other pick).
static PICKING: AtomicBool = AtomicBool::new(false);
/// A picture is being decoded on its worker.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The worker's outcome (the raster staged, or why not), boxed; the UI thread takes it ([`poll`]).
static OUTCOME: AtomicPtr<Result<(), String>> = AtomicPtr::new(core::ptr::null_mut());

fn note(ui: &SettingsApp, text: &str) {
    ui.global::<SetState>().set_wallpaper_note(text.into());
}

/// "wallpaper-pick": Files' picker for an image.
pub fn pick(app: &App, ui: &SettingsApp) {
    if RUNNING.load(SeqCst) {
        return note(ui, "Still setting the last one…");
    }
    note(ui, "");
    PICKING.store(true, SeqCst);
    if let Err(e) = app.open(files::KEY, files::PICK, "image") {
        PICKING.store(false, SeqCst);
        note(ui, &format!("Couldn't open Files ({e})"));
    }
}

/// The app is shown again: the picked file, if a pick was out (none: cancelled), goes to the worker.
pub fn resumed(app: &App, ui: &SettingsApp) {
    if !PICKING.swap(false, SeqCst) {
        return;
    }
    let Some(f) = app.result_file() else {
        return;
    };
    let path = match f.file().full_name() {
        Ok(p) => p,
        Err(e) => return note(ui, &format!("Couldn't read the picture ({e})")),
    };
    if !matches!(
        path.as_bytes().first().map(u8::to_ascii_uppercase),
        Some(b'C' | b'E')
    ) {
        return note(ui, "Pick a picture on C: or E:.");
    }
    if RUNNING.swap(true, SeqCst) {
        return note(ui, "Still setting the last one…");
    }
    // the worker opens the file by its name with a file-server session of its own (Files' handle is this thread's)
    if let Err(e) = e32::thread::spawn(move || {
        let started = e32::time::millis();
        let outcome = Box::new(fit(&path));
        dprintln!(
            "settings: wallpaper picture fitted in {} ms ({:?})",
            e32::time::millis() - started,
            (*outcome).as_ref().err()
        );
        let old = OUTCOME.swap(Box::into_raw(outcome), SeqCst);
        if !old.is_null() {
            // SAFETY: a pointer only `Box::into_raw` put there, taken once (swap)
            drop(unsafe { Box::from_raw(old) });
        }
        RUNNING.store(false, SeqCst);
        0
    }) {
        RUNNING.store(false, SeqCst);
        return note(ui, &format!("Couldn't start ({e})"));
    }
    note(ui, "Setting the picture…");
}

/// The worker: the file read, decoded, turned, fitted and staged.
fn fit(path: &str) -> Result<(), String> {
    let fs = Fs::connect().map_err(|e| format!("Couldn't read the picture ({e})"))?;
    let read = |e: i32| format!("Couldn't read the picture ({e})");
    let bytes = {
        let file = File::open(&fs, path, mode::READ | mode::SHARE_READERS_ONLY).map_err(read)?;
        let size = usize::try_from(file.size().map_err(read)?).unwrap_or(usize::MAX);
        if size > FILE_MAX {
            return Err("This picture is too large".into());
        }
        let mut b = Vec::new();
        b.try_reserve_exact(size)
            .map_err(|_| String::from("Not enough memory for this picture"))?;
        b.resize(size, 0);
        let mut got = 0;
        while got < size {
            let n = file
                .read_at(got as i32, &mut b[got..size.min(got + 64 * 1024)])
                .map_err(read)?;
            if n == 0 {
                break; // it shrank meanwhile: what is there is decoded (or fails)
            }
            got += n;
        }
        b.truncate(got);
        b
    };
    let decoded = img::decode(&bytes, DECODE_MAX).map_err(|e| {
        String::from(match e {
            img::Error::NotImage => "That isn't a picture",
            img::Error::Corrupt => "This picture is damaged",
            img::Error::TooBig => "This picture is too large",
        })
    })?;
    drop(bytes);
    // a JPEG too big to decode comes back as its small EXIF preview: too blurry for a wallpaper
    if decoded.preview {
        return Err("This picture is too large".into());
    }
    // turned as the camera held it, and box-shrunk to at most twice the screen (the fit then averages the rest)
    let turned = shrink::fit(
        &decoded.px,
        shrink::Turn::exif(decoded.info.orientation),
        u32::from(H) * 2,
    );
    drop(decoded);
    let raster = wp::cover(
        &turned.px,
        turned.w,
        turned.h,
        turned.ch,
        u32::from(W),
        u32::from(H),
    )
    .ok_or_else(|| String::from("This kind of picture isn't supported"))?;
    debug_assert_eq!(raster.len(), BYTES);
    let write = |e: i32| format!("Couldn't save the picture ({e})");
    match fs.mkdir("C:\\private\\e0002007\\") {
        Ok(()) | Err(e32::sys::err::KERR_ALREADY_EXISTS) => {}
        Err(e) => return Err(write(e)),
    }
    let f = File::replace(&fs, STAGED, mode::SHARE_EXCLUSIVE).map_err(write)?;
    f.write(&raster).map_err(write)?;
    f.flush().map_err(write)
}

/// From the tick: the worker's outcome, and the staged raster handed to the shell.
pub fn poll(app: &App, ui: &SettingsApp) {
    let p = OUTCOME.swap(core::ptr::null_mut(), SeqCst);
    if p.is_null() {
        return;
    }
    // SAFETY: a pointer only `Box::into_raw` put there, taken once (swap)
    let outcome = *unsafe { Box::from_raw(p) };
    if let Err(why) = outcome {
        return note(ui, &why);
    }
    let r = install(app);
    dprintln!("settings: wallpaper picture install ({:?})", r);
    note(
        ui,
        match r {
            Ok(()) => "",
            Err(-5) => "This phone can't show pictures yet", // KErrNotSupported: an older shell
            Err(_) => "Couldn't set the picture; the old wallpaper stays",
        },
    );
    if let Ok(fs) = Fs::connect() {
        let _ = fs.delete(STAGED);
    }
}

/// The staged raster to the shell (it copies it into its own folder before it answers).
fn install(app: &App) -> Result<(), i32> {
    let mut fs = Fs::connect()?;
    fs.share_protected()?;
    let f = File::open(&fs, STAGED, mode::READ | mode::SHARE_READERS_ONLY)?;
    let req = wp::REQUEST.encode().ok_or(-6)?; // KErrArgument: never, the request is a constant
    app.settings_action_file(proto::ACTION_INSTALL_WALLPAPER, &req, &f)
}
