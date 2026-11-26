// SPDX-License-Identifier: GPL-3.0-only
//! D6, SE2c-1 (docs neoui-shell/settings.md "SE2c plan"): Your sounds. "Add from Files" opens Files' picker (filter
//! `wav`); the WAV it answers with (on C: or E:, at most 16 MiB) becomes one ring period (`neoaudio::wav`, 192,000
//! bytes, 48 kHz mono) in `C:\private\e0002007\tones\<id>.pcm` (a private `.new`, flushed, then `Fs::replace_file`),
//! then the shell's reducer registers its name (`ACTION_PROFILE` tag 7, ids 4..=15). Rename is tag 8; Delete is tag 9
//! (every profile using it goes back to the built-in tone) and only then the file goes. AUDIOD rings the file and
//! falls back to the built-in tone when it is missing or the wrong size.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering::SeqCst};
use e32::dprintln;
use e32::fs::{File, Fs, SharedFile, mode};
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
    s.set_import_note("".into());
    PICKING.store(true, SeqCst);
    if let Err(e) = app.open(files::KEY, files::PICK, "wav") {
        PICKING.store(false, SeqCst);
        s.set_import_note(format!("Couldn't open Files ({e})").into());
    }
}

/// The app is shown again: the picked file, if a pick was out (none: cancelled).
pub fn resumed(app: &App, ui: &SettingsApp) {
    if !PICKING.swap(false, SeqCst) {
        return;
    }
    let Some(f) = app.result_file() else {
        return;
    };
    let note = match import(app, &f) {
        Ok(name) => format!("Added {name}."),
        Err(why) => why,
    };
    dprintln!("settings: import: {}", note);
    crate::store::profiles(app, ui);
    ui.global::<SetState>().set_import_note(note.into());
}

/// The file Files gave, read whole (at most `wav::MAX_INPUT`), converted, written and registered. The name added,
/// or why not.
fn import(app: &App, f: &SharedFile) -> Result<String, String> {
    let file = f.file();
    let path = file
        .full_name()
        .map_err(|e| format!("Couldn't read the file ({e})"))?;
    if !matches!(
        path.as_bytes().first().map(u8::to_ascii_uppercase),
        Some(b'C' | b'E')
    ) {
        return Err("Pick a WAV from C: or E:.".into());
    }
    let size = file
        .size()
        .map_err(|e| format!("Couldn't read the file ({e})"))?;
    let size = usize::try_from(size).unwrap_or(usize::MAX);
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
            .map_err(|e| format!("Couldn't read the file ({e})"))?;
        if n == 0 {
            break;
        }
        got += n;
    }
    bytes.truncate(got);
    let sound = wav::convert(&bytes).map_err(|e| String::from(e.reason()))?;
    drop(bytes);
    let profiles = crate::store::read_profiles(app);
    let id = profiles
        .free_tone()
        .ok_or_else(|| String::from("Twelve sounds at most: delete one first."))?;
    let name = sound
        .name
        .clone()
        .or_else(|| stem(&path))
        .unwrap_or_else(|| format!("Sound {}", id - 3));
    write(id, &sound.pcm).map_err(|e| format!("Couldn't save the sound ({e})"))?;
    if let Err(e) = action(app, &Command::AddTone(id, name.clone())) {
        let _ = remove(id);
        return Err(format!("Couldn't add the sound ({e})"));
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

/// `<id>.pcm`, through a flushed private `.new` and one replace.
fn write(id: u8, pcm: &[u8]) -> Result<(), i32> {
    let path = neoaudio::tones::file(id).ok_or(-6)?;
    let fs = Fs::connect()?;
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
    let new = format!("{path}.new");
    {
        let f = File::replace(&fs, &new, mode::WRITE | mode::SHARE_EXCLUSIVE)?;
        f.write(pcm)?;
        f.flush()?;
    }
    fs.replace_file(&new, &path)
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
/// tone), then the file.
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
