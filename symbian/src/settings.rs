// SPDX-License-Identifier: GPL-3.0-only
//! The UI server's settings (docs/neoui-shell.md, Part 1): `neoui_proto::settings::Service` over a file on `C:`.
//! The logic (validation, save delay, access, subscriptions) is the host-tested service; this module only adds the
//! file and the log lines CI checks.

use alloc::vec;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::sys::err;
use neoui_proto::clock;
use neoui_proto::settings::service::{LoadReport, SaveOutcome, StorageError};
use neoui_proto::settings::{Backend, Caller, Changed, Error, Service, Value, file, kerr};
use neoui_render::Renderer;

/// The settings file, in the UI server's private directory (SID 0xE0002001).
const PATH: &str = "C:\\private\\e0002001\\settings.dat";
/// A save is written here first, then renamed over [`PATH`].
const NEW_PATH: &str = "C:\\private\\e0002001\\settings.new";

/// The UI server's settings service.
pub type Settings = Service<FileBackend>;

/// Stores the settings in [`PATH`], replaced atomically: write [`NEW_PATH`], delete the old file, rename. A power
/// cut between the delete and the rename leaves only the new file, which [`Backend::load`] falls back to.
#[derive(Debug, Default)]
pub struct FileBackend;

impl Backend for FileBackend {
    fn load(&mut self) -> Result<Option<Vec<u8>>, i32> {
        let fs = Fs::connect()?;
        for path in [PATH, NEW_PATH] {
            match read(&fs, path) {
                Ok(bytes) => return Ok(Some(bytes)),
                Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    fn save(&mut self, bytes: &[u8]) -> Result<(), i32> {
        let fs = Fs::connect()?;
        for dir in ["C:\\private\\", clock::DIR] {
            match fs.mkdir(dir) {
                Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
                Err(e) => return Err(e),
            }
        }
        // the file closes (drop) before the rename
        File::replace(&fs, NEW_PATH, mode::SHARE_EXCLUSIVE)?.write(bytes)?;
        match fs.delete(PATH) {
            Ok(()) | Err(err::KERR_NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
        fs.rename(NEW_PATH, PATH)
    }
}

/// The whole file at `path`, refusing one larger than a settings file can be.
fn read(fs: &Fs, path: &str) -> Result<Vec<u8>, i32> {
    let f = File::open(fs, path, mode::READ | mode::SHARE_READERS_ONLY)?;
    let size = usize::try_from(f.size()?).map_err(|_| kerr::CORRUPT)?;
    if size > file::FILE_MAX {
        return Err(kerr::CORRUPT);
    }
    let mut bytes = vec![0u8; size];
    let n = f.read_at(0, &mut bytes)?;
    bytes.truncate(n);
    Ok(bytes)
}

/// Loads the settings and logs what was found (`neoui: settings loaded N keys (vV)`).
pub fn open() -> Settings {
    let (settings, report) = Service::open(FileBackend);
    let LoadReport {
        keys,
        version,
        dropped,
        error,
    } = report;
    match (error, version) {
        (Some(e), _) => dprintln!("neoui: settings unreadable ({:?}), using defaults", e),
        (None, Some(v)) => dprintln!(
            "neoui: settings loaded {} keys (v{}, {} dropped)",
            keys,
            v,
            dropped
        ),
        (None, None) => dprintln!("neoui: settings: none saved yet, using defaults"),
    }
    settings
}

/// Logs a save's outcome (`neoui: settings saved (N bytes)` / `neoui: settings save failed (E)`).
pub fn log_save(outcome: Option<SaveOutcome>) {
    match outcome {
        None => {}
        Some(Ok(n)) => dprintln!("neoui: settings saved ({} bytes)", n),
        Some(Err(StorageError::Backend(e))) => dprintln!("neoui: settings save failed ({})", e),
        Some(Err(StorageError::Format(e))) => dprintln!("neoui: settings save failed ({})", e),
    }
}

/// Writes a setting for `caller` and logs a change (`neoui: setting KEY = VALUE (by WHO)`) or a refusal.
///
/// # Errors
/// As [`Service::set_raw`].
pub fn set(
    settings: &mut Settings,
    caller: Caller<'_>,
    key: &str,
    value: Value,
    now_ms: u64,
) -> Result<Changed, Error> {
    let shown = value.clone();
    let result = settings.set_raw(caller, key, value, now_ms);
    match result {
        Ok(Changed::Yes) => dprintln!("neoui: setting {} = {} (by {})", key, shown, caller),
        Ok(Changed::No) => {}
        Err(e) => dprintln!("neoui: setting {} refused for {} ({})", key, caller, e),
    }
    result
}

/// Keeps the shell's view of the settings in step with the store: edits made in the UI go into the store, and
/// changes to the store (from the UI, an app, or a load) show in the UI.
pub struct UiMirror {
    /// The store's change counter the UI shows.
    seq: u32,
}

impl UiMirror {
    /// Shows every setting the shell displays, as stored.
    pub fn new(ui: &mut Renderer, settings: &Settings) -> Self {
        for key in Renderer::shown_settings() {
            show(ui, settings, key);
        }
        Self {
            seq: settings.store().seq(),
        }
    }

    /// Once per frame: stores the UI's edits, then shows what changed in the store.
    pub fn sync(&mut self, ui: &mut Renderer, settings: &mut Settings, now_ms: u64) {
        for (key, value) in ui.take_setting_edits() {
            // a refused edit (logged by `set`) must not stay on screen: show the stored value again
            if set(settings, Caller::Shell, key, value, now_ms).is_err() {
                show(ui, settings, key);
            }
        }
        let seq = settings.store().seq();
        if seq == self.seq {
            return;
        }
        let stale: Vec<&'static str> = match settings.store().changes_since(self.seq) {
            Some(changed) => {
                let changed: Vec<&str> = changed.collect();
                Renderer::shown_settings()
                    .filter(|k| changed.contains(k))
                    .collect()
            }
            None => Renderer::shown_settings().collect(),
        };
        for key in stale {
            show(ui, settings, key);
        }
        self.seq = seq;
    }
}

fn show(ui: &mut Renderer, settings: &Settings, key: &str) {
    match settings.get_raw(key) {
        Ok(v) if ui.apply_setting(key, &v) => {}
        other => dprintln!("neoui: setting {} not shown ({:?})", key, other),
    }
}
