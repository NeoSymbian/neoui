// SPDX-License-Identifier: GPL-3.0-only
//! D5 (docs neoui-shell/settings-data.md): Battery usage's charge cycle and its screen-on time, from the shell's INFO 9
//! (`neoui_proto::battery::cycle`'s recorder in SYSSTART). A shell that doesn't advertise INFO 9 in FEATURES gets
//! SE3a's view of its INFO 5 history, without screen time. The same texts feed the page, the Battery page's row and
//! search (`battery.screen_on`).
use core::sync::atomic::{AtomicU8, Ordering};
use neoapp::App;
use neoui_proto::battery::cycle::{self, CycleText};
use neoui_proto::ipc as proto;
use neoui_proto::settings::kerr;
use neoui_proto::settings_app::info::decode_usage;
use neoui_proto::settings_app::wire::{self, BatteryCycle, Features};
use settings_ui::{SetState, SettingsApp};
use slint::ComponentHandle;

/// Whether the shell answers INFO 9: 0 not asked yet, 1 no, 2 yes (FEATURES, once per connection).
static ADVERTISED: AtomicU8 = AtomicU8::new(0);

fn advertised(app: &App) -> bool {
    match ADVERTISED.load(Ordering::Relaxed) {
        0 => {}
        n => return n == 2,
    }
    let yes = match app.settings_records(proto::INFO_FEATURES) {
        Ok((_, records)) => wire::decode_singleton(&records, Features::decode)
            .flatten()
            .is_some_and(|f| f.has_info(proto::INFO_BATTERY_CYCLE)),
        // a 1.19 shell: it knows no paged kinds
        Err(kerr::ARGUMENT | kerr::NOT_SUPPORTED) => false,
        Err(_) => return false, // ask again next time
    };
    ADVERTISED.store(if yes { 2 } else { 1 }, Ordering::Relaxed);
    yes
}

/// The cycle as the shell has it: `Some(None)` before its first reading, `None` when it can't say now (keep what
/// is shown).
fn read(app: &App) -> Option<Option<CycleText>> {
    if advertised(app) {
        let (_, records) = app.settings_records(proto::INFO_BATTERY_CYCLE).ok()?;
        let c = wire::decode_singleton(&records, BatteryCycle::decode)?;
        return Some(c.map(|c| cycle::text(&c, true)));
    }
    let u = decode_usage(&app.settings_info(proto::INFO_BATTERY_USAGE).ok()?)?;
    Some(u.map(|u| cycle::text(&cycle::from_usage(&u), false)))
}

/// Read and show (Battery, Battery usage and search, every five seconds while one is shown).
pub fn refresh(app: &App, ui: &SettingsApp) {
    let Some(t) = read(app) else { return };
    let s = ui.global::<SetState>();
    s.set_usage_known(t.is_some());
    let t = t.unwrap_or_default();
    s.set_usage_title(t.title.into());
    s.set_usage_from(t.from.into());
    s.set_usage_to(t.to.into());
    s.set_usage_span(t.span.into());
    s.set_usage_on_battery(t.on_battery.into());
    s.set_usage_gaps(t.gaps.into());
    s.set_screen_on(t.screen.into());
}
