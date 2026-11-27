// SPDX-License-Identifier: GPL-3.0-only
//! (TP7) The control centre's shell side (docs neoui-shell/top-panel.md "Tiles and the control centre", protocol.md
//! "TP7 tiles"): a child of `host`. It keeps the persistent slot table (`neoui_proto::tile_table::Slots`, saved to
//! [`PATH`] as the jobs table is: staged, flushed, then replaced in one operation), takes the apps' `TILE` offers
//! (checked by `contract::tile`), sends a tap or hold on an app's tile as event 26 (held for an app that isn't running
//! until it binds with minor ≥ 22), answers INFO 12 and `ACTION_EDIT_TILES`, and at load takes app tiles of
//! unassigned slots out of `shell.tiles`. The control centre itself is the renderer's (neoui-render `tiles.rs`).

use super::{Host, read_bytes};
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::server::Message;
use e32::sys::err;
use neoui_proto::apps;
use neoui_proto::contract;
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::settings::{Caller, Value, access, kerr, keys};
use neoui_proto::tile_table::{self, Held, Slots};
use neoui_proto::tiles::{self, builtin};
use neoui_render::Renderer;

/// The slot table across restarts, and the file a save writes first.
const PATH: &str = "C:\\private\\e0002001\\tiles.dat";
const STAGING: &str = "C:\\private\\e0002001\\tiles.tmp";
/// The most a saved table can be: far more slots than apps.
const FILE_MAX: usize = 64 * 1024;
/// A failed save is tried again this often.
const RETRY_MS: u64 = 1000;

/// The host's TP7 state.
#[derive(Default)]
pub struct Tiles {
    slots: Slots,
    held: Held,
    /// apps bound before 1.22: their tiles can't take a tap
    stale: Vec<&'static str>,
    /// the renderer hasn't the table as it is
    changed: bool,
    /// Settings asked for the editor
    edit: bool,
    /// the table changed and isn't saved yet; when the last save failed
    dirty: bool,
    failed_at: Option<u64>,
    /// the table was read (or there is none): until then nothing overwrites it
    loaded: bool,
    /// `shell.tiles` was checked against the table since it was read
    pruned: bool,
    /// INFO 12 as last published, and the store's change counter it was built at
    published: Vec<Vec<u8>>,
    seq: Option<u32>,
    /// the control centre and the editor as last logged
    shown: (bool, bool),
}

impl Tiles {
    /// The saved table (an unreadable one: an empty table, so no slot is ever handed out twice from a stale copy, and
    /// the placed app tiles of the lost slots leave `shell.tiles`).
    pub fn boot() -> Self {
        let read = || -> Result<Vec<u8>, i32> {
            let fs = Fs::connect()?;
            let f = File::open(&fs, PATH, mode::READ | mode::SHARE_READERS_ONLY)?;
            let size = usize::try_from(f.size()?).map_err(|_| kerr::CORRUPT)?;
            if size > FILE_MAX {
                return Err(kerr::CORRUPT);
            }
            let mut b = alloc::vec![0; size];
            if f.read_at(0, &mut b)? != size {
                return Err(kerr::CORRUPT);
            }
            Ok(b)
        };
        let slots = match read() {
            Ok(b) => Slots::decode(&b).unwrap_or_else(|| {
                dprintln!("neoui: tiles table damaged, app tiles dropped");
                Slots::default()
            }),
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => Slots::default(),
            Err(e) => {
                // shortcut: not read again later (unlike the jobs table); a slot taken now may be one of the unread
                // table's, so its placed tiles are dropped at load like a lost table's
                dprintln!("neoui: tiles table unreadable ({}), app tiles dropped", e);
                Slots::default()
            }
        };
        Self {
            slots,
            changed: true,
            loaded: true,
            ..Self::default()
        }
    }

    fn save(&mut self) -> Result<(), i32> {
        let fs = Fs::connect()?;
        for dir in ["C:\\private\\", "C:\\private\\e0002001\\"] {
            match fs.mkdir(dir) {
                Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
                Err(e) => return Err(e),
            }
        }
        {
            let f = File::replace(&fs, STAGING, mode::SHARE_EXCLUSIVE)?;
            f.write(&self.slots.encode())?;
            f.flush()?;
        }
        fs.replace_file(STAGING, PATH)
    }
}

/// A built-in tile's state as the store has it.
fn builtin_on(host: &Host, id: u16) -> bool {
    let s = &host.settings;
    match id {
        builtin::WIFI => s.get(keys::RADIO_WIFI).unwrap_or(true),
        builtin::MOBILE => s.get(keys::RADIO_DATA).unwrap_or(true),
        builtin::SILENT => s.get(keys::SOUND_SILENT).unwrap_or(false),
        builtin::DND => host.dnd(),
        builtin::AIRPLANE => s.get(keys::RADIO_AIRPLANE).unwrap_or(false),
        builtin::BATTERY_SAVER => s.get(keys::BATTERY_SAVER).unwrap_or(false),
        builtin::DARK => s.get(keys::DISPLAY_DARK).unwrap_or(true),
        _ => false,
    }
}

impl Host {
    /// `shell.tiles` as stored (the default order when unreadable).
    fn tile_order(&self) -> (Vec<u8>, Vec<(u16, bool)>) {
        let raw = match self.settings.get_raw(keys::SHELL_TILES.name()) {
            Ok(Value::Bytes(b)) => b,
            _ => Vec::new(),
        };
        let order = tiles::decode_order(&raw).unwrap_or_else(|| tiles::DEFAULT_ORDER.to_vec());
        (raw, order)
    }

    /// `TILE` from app `key` (TP7): checked (its catalogue SID, the offer), then kept in its slot.
    pub(super) fn tile_offer(&mut self, key: &'static str, m: &Message) -> i32 {
        let offer = match contract::tile(key, m.client_sid(), |max| read_bytes(m, 0, max)) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let (local, withdraw) = (offer.local_id, offer.kind == tiles::kind::WITHDRAW);
        if withdraw && self.tiles.slots.slot(key).is_none() {
            return 0; // nothing to withdraw: no slot taken for it
        }
        let Some(id) = self.tiles.slots.offer(key, offer) else {
            dprintln!("neoui: no tile slot left for {}", key);
            return kerr::OVERFLOW;
        };
        dprintln!(
            "neoui: tile {} {} by {} ({})",
            id,
            if withdraw { "withdrawn" } else { "offered" },
            key,
            local
        );
        self.tiles.dirty = true;
        self.tiles.changed = true;
        0
    }

    /// `SETTINGS_ACTION` `ACTION_EDIT_TILES`: Settings only, an empty request; the editor comes up over the control
    /// centre at the next frame.
    pub(super) fn edit_tiles_action(&mut self, key: &str, m: &Message) -> i32 {
        if access::caller(key, m.client_sid()) != Caller::Settings {
            return kerr::PERMISSION_DENIED;
        }
        match m.des_len(1) {
            Ok(0) => {}
            Ok(_) | Err(_) => return kerr::ARGUMENT,
        }
        dprintln!("neoui: settings opens the tile editor");
        self.tiles.edit = true;
        0
    }

    /// App `key` bound with protocol minor `minor`: the taps held for it, and whether its tiles can take them.
    pub(super) fn tiles_bound(&mut self, key: &'static str, minor: u8) {
        let was = self.tiles.stale.contains(&key);
        if minor < 22 {
            if !was {
                self.tiles.stale.push(key);
                self.tiles.changed = true;
            }
        } else if was {
            self.tiles.stale.retain(|k| *k != key);
            self.tiles.changed = true;
        }
        for e in self.tiles.held.bound(key, minor) {
            dprintln!("neoui: held tile event for {}", key);
            self.post(key, e);
        }
    }

    /// A tap or hold on app tile `id`: event 26 to its app; an app that isn't running is started hidden and gets it
    /// once it binds (1.22 or later). A Detail tile's tap opens its app on the tile's page too.
    fn tile_tapped(&mut self, id: u16, hold: bool, now_ms: u64) {
        let Some((owner, offer)) = self.tiles.slots.owner_of(id) else {
            return;
        };
        let (Some(key), local, kind, page) = (
            apps::find(owner).map(|a| a.key),
            offer.local_id,
            offer.kind,
            offer.page,
        ) else {
            return;
        };
        dprintln!(
            "neoui: tile {} of {} {}",
            id,
            key,
            if hold { "held" } else { "tapped" }
        );
        if kind == tiles::kind::DETAIL && !hold {
            self.limit_running(key, now_ms);
            self.start(key);
            self.post(key, AppEvent::Page { page });
            self.requested_screen = Some(key);
        }
        let e = AppEvent::Tile { id: local, hold };
        match self
            .apps
            .find(|a| a.key == key && a.quit_at.is_none())
            .map(|a| (a.session.is_some(), a.minor))
        {
            Some((true, minor)) if minor >= e.needs_minor() => {
                self.post(key, e);
            }
            Some((true, _)) => dprintln!("neoui: {} is before 1.22: no tile event", key),
            Some((false, _)) => self.tiles.held.hold(key, local, hold, now_ms),
            None => {
                // `connect` binds the newest app without a session: no hidden start while another app connects
                let connecting = self
                    .apps
                    .find(|a| a.session.is_none() && a.quit_at.is_none())
                    .is_some();
                if !connecting && self.launch(key) {
                    self.tiles.held.hold(key, local, hold, now_ms);
                }
            }
        }
    }

    /// INFO 12 as last published.
    pub(super) fn tile_records(&self) -> Vec<Vec<u8>> {
        self.tiles.published.clone()
    }

    /// Once per frame: `shell.tiles` checked against the table after it was read, the table saved, the renderer and
    /// INFO 12 kept up, the taps on app tiles sent, Settings' editor opened.
    pub fn tiles_frame(&mut self, ui: &mut Renderer, now_ms: u64) {
        if !self.tiles.pruned && self.tiles.loaded {
            self.tiles.pruned = true;
            let (_, order) = self.tile_order();
            if let Some(kept) = tile_table::prune(&order, &self.tiles.slots) {
                let b = tiles::encode_order(&kept).unwrap_or_default();
                dprintln!("neoui: tiles of unassigned slots left shell.tiles");
                let _ = crate::settings::set(
                    &mut self.settings,
                    Caller::Shell,
                    keys::SHELL_TILES.name(),
                    Value::Bytes(b),
                    now_ms,
                );
            }
        }
        if self.tiles.dirty
            && self
                .tiles
                .failed_at
                .is_none_or(|t| now_ms.saturating_sub(t) >= RETRY_MS)
        {
            match self.tiles.save() {
                Ok(()) => (self.tiles.dirty, self.tiles.failed_at) = (false, None),
                Err(e) => {
                    if self.tiles.failed_at.is_none() {
                        dprintln!("neoui: tiles table save failed ({})", e);
                    }
                    self.tiles.failed_at = Some(now_ms);
                }
            }
        }
        self.tiles.held.expire(now_ms);
        let seq = self.settings.store().seq();
        let changed = core::mem::take(&mut self.tiles.changed);
        if changed {
            // only then: a new model would drop a press or drag on the tiles
            ui.set_tile_slots(&self.tiles.slots, &self.tiles.stale);
        }
        if changed || self.tiles.seq != Some(seq) {
            self.tiles.seq = Some(seq);
            let (_, order) = self.tile_order();
            let drawn = tile_table::drawn(&order, &self.tiles.slots);
            let records: Vec<Vec<u8>> =
                tile_table::records(&drawn, &self.tiles.slots, |b| builtin_on(self, b))
                    .iter()
                    .map(tiles::Record::encode)
                    .collect();
            if records != self.tiles.published {
                self.tiles.published = records;
                self.info_changed(proto::INFO_TILES);
            }
        }
        for (id, hold) in ui.take_tile_taps() {
            self.tile_tapped(id, hold, now_ms);
        }
        if core::mem::take(&mut self.tiles.edit) {
            ui.edit_tiles();
        }
        let shown = (ui.control_open(), ui.tiles_editing());
        if shown != self.tiles.shown {
            if shown.0 != self.tiles.shown.0 {
                dprintln!(
                    "neoui: control centre {}",
                    if shown.0 { "shown" } else { "closed" }
                );
            }
            if shown.1 != self.tiles.shown.1 {
                dprintln!(
                    "neoui: tile editor {}",
                    if shown.1 { "shown" } else { "closed" }
                );
            }
            self.tiles.shown = shown;
        }
    }
}
