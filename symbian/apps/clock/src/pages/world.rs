// SPDX-License-Identifier: GPL-3.0-only
//! The World tab and its city catalogue: the time in each chosen city against local time (`clock::world`); a tap on
//! a city removes it, a tap in the catalogue adds one. Saved in `clock.cities`.

use super::{Ctx, Now, rows};
use crate::state::{Model, Part, View};
use crate::{CatalogueRow, CityRow, ClockState};
use alloc::format;
use alloc::rc::Rc;
use e32::dprintln;
use neoui_proto::clock::world::{AddError, CATALOGUE, CITIES_MAX};

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_add_city(move || c.open(View::Cities));
    let c = ctx.clone();
    s.on_remove_city(move |position| {
        c.update(|m, _| {
            let position = usize::try_from(position).ok()?;
            let name = m.cities.as_slice().get(position)?.name;
            m.cities.remove(position).then(|| {
                dprintln!("clock: city {} removed", name);
                Part::Cities
            })
        });
    });
    let c = ctx.clone();
    s.on_pick_city(move |index| {
        let mut added = false;
        c.update(|m, _| {
            let index = usize::try_from(index).ok()?;
            match m.cities.add(index) {
                Ok(()) => {
                    dprintln!("clock: city {} added", CATALOGUE.get(index)?.name);
                    added = true;
                    Some(Part::Cities)
                }
                Err(AddError::Duplicate | AddError::Full) => None,
            }
        });
        if added {
            c.open(View::None);
        }
    });
}

pub fn show(s: &ClockState<'_>, m: &Model, now: &Now) {
    s.set_cities(rows(m.cities.as_slice().iter().map(|city| {
        let shown = city.show(now.utc, now.tz, m.h24);
        CityRow {
            name: city.name.into(),
            time: shown.time.as_str().into(),
            day: shown.day.into(),
            difference: shown.difference.as_str().into(),
        }
    })));
    s.set_cities_full(m.cities.as_slice().len() >= CITIES_MAX);
    if m.view == View::Cities {
        s.set_catalogue(rows(CATALOGUE.iter().map(|city| {
            let (sign, mins) = (
                if city.offset_min < 0 { '−' } else { '+' },
                city.offset_min.unsigned_abs(),
            );
            CatalogueRow {
                name: city.name.into(),
                offset: format!("UTC{sign}{}:{:02}", mins / 60, mins % 60)
                    .as_str()
                    .into(),
                added: m.cities.as_slice().iter().any(|c| c.name == city.name),
            }
        })));
    }
}
