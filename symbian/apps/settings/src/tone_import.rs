// SPDX-License-Identifier: GPL-3.0-only
//! D6, SE2c-1 (docs neoui-shell/settings.md "SE2c plan"): Your sounds. "Add from Files" opens Files' picker (filter
//! `wav`); the WAV it answers with (on C: or E:, at most 16 MiB) becomes one ring period (`neoaudio::wav`, 192,000
//! bytes, 48 kHz mono), read and converted on a worker into a flushed `tones\import-<ms>.new` of its own; then the
//! shell's reducer reserves a free id with its name (`ACTION_PROFILE` tag 7, ids 4..=15; a taken one is refused) and
//! only then the file becomes `C:\private\e0002007\tones\<id>.pcm` (`Fs::rename`: never over a file already
//! there). Rename is tag 8; Delete is tag 9
//! (every profile using it goes back to the built-in tone) and only then the file goes. AUDIOD rings the file and
//! falls back to the built-in tone when it is missing or the wrong size.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering::SeqCst};
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use neoapp::App;
use neoaudio::wav;
use neoui_proto::ipc as proto;
use neoui_proto::open::files;
use neoui_proto::sound_profile::{Command, Profiles};
use settings_ui::{SetState, SettingsApp};
use slint::{ComponentHandle, ModelRc, VecModel};

/// A pick is out with Files: its answer comes when this app is shown again.
// shortcut: one pick at a time and no binding to the Files session (the integrator's picker.rs closes that race:
// a pick interrupted by a call, Files exiting, or another app's pick), upgrade when picker.rs lands
static PICKING: AtomicBool = AtomicBool::new(false);

/// The sounds into the Ringtone page (from the profiles the store just gave).
pub fn show(ui: &SettingsApp, p: &Profiles) {
    let s = ui.global::<SetState>();
    let rows: Vec<settings_ui::YourTone> = p
        .yours
        .iter()
        .map(|t| settings_ui::YourTone {
            id: i32::from(t.id),
            name: t.name.as_str().into(),
        })
        .collect();
    s.set_your_tones(ModelRc::new(VecModel::from(rows)));
    s.set_your_tones_full(p.free_tone().is_none());
}

/// "tone-add": Files' picker for a WAV.
pub fn add(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    if s.get_your_tones_full() {
        s.set_import_note("Twelve sounds at most: delete one first.".into());
        return;
    }
    if RUNNING.load(SeqCst) {
        s.set_import_note("Still adding the last one…".into());
        return;
    }
    s.set_import_note("".into());
    PICKING.store(true, SeqCst);
    if let Err(e) = app.open(files::KEY, files::PICK, "wav") {
        PICKING.store(false, SeqCst);
        s.set_import_note(format!("Couldn't open Files ({e})").into());
    }
}

/// The worker's answer (handed over once through [`OUTCOME`]): the import's own staging file ([`staged`]) and the
/// name the file gives (INAM, else its file name); or why it can't be a sound.
type Outcome = Result<(String, Option<String>), String>;

/// A conversion is running on its worker.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The worker's [`Outcome`], boxed; the UI thread takes it ([`poll`]).
static OUTCOME: AtomicPtr<Outcome> = AtomicPtr::new(core::ptr::null_mut());

/// The converted sound before it has an id, one file per import (`import-<ms>.new`), so a late clean-up of one never
/// takes another's: renamed onto `<id>.pcm` once the reducer reserved that id.
fn staged() -> String {
    format!("{}import-{}.new", neoaudio::tones::DIR, e32::time::millis())
}

/// The app is shown again: the picked file, if a pick was out (none: cancelled). It is read and converted on a
/// worker (up to 16 MiB: the UI keeps running); [`poll`] finishes it.
pub fn resumed(app: &App, ui: &SettingsApp) {
    if !PICKING.swap(false, SeqCst) {
        return;
    }
    let Some(f) = app.result_file() else {
        return;
    };
    let s = ui.global::<SetState>();
    let path = match f.file().full_name() {
        Ok(p) => p,
        Err(e) => return s.set_import_note(format!("Couldn't read the file ({e})").into()),
    };
    if !matches!(
        path.as_bytes().first().map(u8::to_ascii_uppercase),
        Some(b'C' | b'E')
    ) {
        return s.set_import_note("Pick a WAV from C: or E:.".into());
    }
    if RUNNING.swap(true, SeqCst) {
        return s.set_import_note("Still adding the last one…".into());
    }
    // the worker opens the file by its name with a file-server session of its own (Files' handle is this thread's)
    if let Err(e) = e32::thread::spawn(move || {
        let outcome = Box::new(convert(&path, staged()));
        let old = OUTCOME.swap(Box::into_raw(outcome), SeqCst);
        if !old.is_null() {
            // SAFETY: a pointer only `Box::into_raw` put there, taken once (swap)
            drop(unsafe { Box::from_raw(old) });
        }
        RUNNING.store(false, SeqCst);
        0
    }) {
        RUNNING.store(false, SeqCst);
        return s.set_import_note(format!("Couldn't start the import ({e})").into());
    }
    s.set_import_note("Adding the sound…".into());
}

/// The worker: the file read whole (at most `wav::MAX_INPUT`), converted, staged.
fn convert(path: &str, staging: String) -> Outcome {
    let fs = Fs::connect().map_err(|e| format!("Couldn't read the file ({e})"))?;
    let read = |e: i32| format!("Couldn't read the file ({e})");
    let file = File::open(&fs, path, mode::READ | mode::SHARE_READERS_ONLY).map_err(read)?;
    let size = usize::try_from(file.size().map_err(read)?).unwrap_or(usize::MAX);
    if size > wav::MAX_INPUT {
        return Err(wav::WavError::TooLarge.reason().into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| String::from(wav::WavError::NoMemory.reason()))?;
    bytes.resize(size, 0);
    let mut got = 0;
    while got < size {
        // the read's own count: a file that shrank meanwhile ends early
        let n = file
            .read_at(got as i32, &mut bytes[got..size.min(got + 64 * 1024)])
            .map_err(read)?;
        if n == 0 {
            break;
        }
        got += n;
    }
    bytes.truncate(got);
    drop(file);
    let sound = wav::convert(&bytes).map_err(|e| String::from(e.reason()))?;
    drop(bytes);
    stage(&fs, &staging, &sound.pcm).map_err(|e| format!("Couldn't save the sound ({e})"))?;
    Ok((staging, sound.name.or_else(|| stem(path))))
}

/// Once a tick: the worker's answer, if it came. The reducer reserves a free id first (it refuses one that is
/// taken), and only then the staged file becomes `<id>.pcm`; a failed move gives the id back.
pub fn poll(app: &App, ui: &SettingsApp) {
    let p = OUTCOME.swap(core::ptr::null_mut(), SeqCst);
    if p.is_null() {
        return;
    }
    // SAFETY: a pointer only `Box::into_raw` put there, taken once (swap)
    let outcome = *unsafe { Box::from_raw(p) };
    let staging = outcome.as_ref().ok().map(|(p, _)| p.clone());
    let note = match outcome.and_then(|(p, name)| install(app, &p, name)) {
        Ok(name) => format!("Added {name}."),
        Err(why) => {
            if let (Some(p), Ok(fs)) = (staging, Fs::connect()) {
                let _ = fs.delete(&p);
            }
            why
        }
    };
    dprintln!("settings: import: {}", note);
    crate::store::profiles(app, ui);
    ui.global::<SetState>().set_import_note(note.into());
}

fn install(app: &App, staging: &str, name: Option<String>) -> Result<String, String> {
    // an unreadable catalogue is an error, never an empty one: a taken id must not be written over
    let profiles = crate::store::read_profiles_checked(app)
        .map_err(|e| format!("Couldn't read your sounds ({e})"))?;
    let id = profiles
        .free_tone()
        .ok_or_else(|| String::from("Twelve sounds at most: delete one first."))?;
    let name = name.unwrap_or_else(|| format!("Sound {}", id - 3));
    action(app, &Command::AddTone(id, name.clone()))
        .map_err(|e| format!("Couldn't add the sound ({e})"))?;
    let path = neoaudio::tones::file(id).ok_or_else(|| String::from("No such sound"))?;
    // the id is ours now, but a file already under it is never written over (a rename fails on it): the id goes
    // back and the import says so
    let moved = Fs::connect().and_then(|fs| fs.rename(staging, &path));
    if let Err(e) = moved {
        let _ = action(app, &Command::DeleteTone(id));
        return Err(if e == -11 {
            String::from("A file is already in that sound's place; nothing was replaced") // KErrAlreadyExists
        } else {
            format!("Couldn't save the sound ({e})")
        });
    }
    Ok(name)
}

/// The file's name without its folder and extension, cut to a sound name's 24 bytes; `None` when blank.
fn stem(path: &str) -> Option<String> {
    let base = path.rsplit(['\\', '/']).next()?;
    let base = base.rsplit_once('.').map_or(base, |(b, _)| b);
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let mut end = clean.len().min(wav::NAME_MAX);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    let name = clean[..end].trim();
    (!name.is_empty()).then(|| String::from(name))
}

fn action(app: &App, cmd: &Command) -> Result<(), i32> {
    let r = app.settings_action_with(proto::ACTION_PROFILE, &cmd.encode());
    dprintln!("settings: tone {:?} ({:?})", cmd, r);
    r
}

/// The converted sound into this import's `staging` file, flushed.
fn stage(fs: &Fs, staging: &str, pcm: &[u8]) -> Result<(), i32> {
    for dir in [
        "C:\\private\\",
        "C:\\private\\e0002007\\",
        neoaudio::tones::DIR,
    ] {
        match fs.mkdir(dir) {
            Ok(()) | Err(-11) => {} // KErrAlreadyExists
            Err(e) => return Err(e),
        }
    }
    let f = File::replace(fs, staging, mode::WRITE | mode::SHARE_EXCLUSIVE)?;
    f.write(pcm)?;
    f.flush()
}

/// Delete `<id>.pcm` (and a `.new` a cut write left).
fn remove(id: u8) -> Result<(), i32> {
    let path = neoaudio::tones::file(id).ok_or(-6)?;
    let fs = Fs::connect()?;
    let _ = fs.delete(&format!("{path}.new"));
    match fs.delete(&path) {
        Ok(()) | Err(-1) => Ok(()), // KErrNotFound
        Err(e) => Err(e),
    }
}

/// "tone-delete" (the sheet's Delete for your-current): the reducer first (profiles using it go back to the built-in
/// tone; the shell answers only once that is saved), then the file. An error keeps the file.
pub fn delete(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let Ok(id) = u8::try_from(s.get_your_current().id) else {
        return;
    };
    let note = match action(app, &Command::DeleteTone(id)) {
        Ok(()) => match remove(id) {
            Ok(()) => format!("Deleted {}.", s.get_your_current().name),
            Err(e) => format!("Deleted; its file stays ({e})"),
        },
        Err(e) => format!("Couldn't delete the sound ({e})"),
    };
    crate::store::profiles(app, ui);
    s.set_import_note(note.into());
}

/// "tone-rename" + the new name, for your-current.
pub fn rename(app: &App, ui: &SettingsApp, name: &str) {
    let s = ui.global::<SetState>();
    let Ok(id) = u8::try_from(s.get_your_current().id) else {
        return;
    };
    if action(app, &Command::RenameTone(id, String::from(name.trim()))).is_err() {
        s.set_import_note("Couldn't rename the sound".into());
    }
    crate::store::profiles(app, ui);
}

/// The PCM a preview plays for a sound of the user's at `volume`; `None` when its file is missing or the wrong size
/// (AUDIOD would ring the built-in tone).
pub fn pcm(id: u8, volume: u8) -> Option<Vec<u8>> {
    let path = neoaudio::tones::file(id)?;
    let fs = Fs::connect().ok()?;
    let f = File::open(&fs, &path, mode::READ | mode::SHARE_READERS_ONLY).ok()?;
    let mut b = alloc::vec![0u8; neoaudio::tones::BYTES + 1];
    let n = f.read_at(0, &mut b).ok()?;
    if n != neoaudio::tones::BYTES {
        return None;
    }
    b.truncate(n);
    neoaudio::tones::scale(&mut b, volume);
    Some(b)
}
