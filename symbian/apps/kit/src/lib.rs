// SPDX-License-Identifier: GPL-3.0-only
//! NEOKIT.EXE: the neoui kit gallery on the device, and the smallest complete neoui app (docs `neoui-kit.md`):
//! connect, show the UI, describe the page, then [`neoapp::App::run`]: input, resizes, Slint timers and drawing are
//! neoapp's; the app handles Back (through the kit's layers first) and what is its own.
#![no_std]

extern crate alloc;

use e32::dprintln;
use neoapp::{App, AppEvent, Page};
use slint::ComponentHandle;

slint::include_modules!();

e32::main!(main);

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("kit: failed {}", e);
            e
        }
    }
}

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    let ui = KitApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    ui.show().map_err(|_| -2)?;
    let mut shown = Page::default();
    describe(&app, &ui, &mut shown)?;
    app.draw()?;
    app.run(|e| {
        match e {
            // Back closes the top kit layer (the ⋮ menu, a sheet); with none open the page's can_back is false and
            // Back leaves the app (the shell's)
            AppEvent::Back => {
                neoapp::kit_back!(ui);
            }
            AppEvent::Page { page } => ui.global::<GalleryState>().set_page(i32::from(page)),
            _ => {}
        }
        describe(&app, &ui, &mut shown)?;
        Ok(true)
    })
}

/// The page: Back stays in the app while a kit layer is open.
fn describe(app: &App, ui: &KitApp, shown: &mut Page) -> Result<(), i32> {
    let page = Page {
        title: "UI kit".into(),
        can_back: ui.global::<Kit>().get_layers() > 0,
        ..Page::default()
    };
    if page != *shown {
        app.page(&page)?;
        *shown = page;
    }
    Ok(())
}
