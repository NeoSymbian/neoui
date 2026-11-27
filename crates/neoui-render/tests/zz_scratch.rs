#[allow(dead_code)]
mod common;
use neoui_proto::{InputEvent, TouchPhase};

#[test]
fn scratch() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    let calm = buf.clone();
    let (x, y) = (540, 1680);
    let touch = |r: &mut neoui_render::Renderer, y: i32, phase| r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
    let differ = |a: &[u32], b: &[u32]| a.iter().zip(b).filter(|(p, q)| p != q).count();
    let mode = std::env::var("M").unwrap_or_default();
    if mode.contains('a') {
        touch(&mut r, y, TouchPhase::Down);
        r.cancel_touch();
        common::frame(&mut r, &mut buf, w, "taken");
        eprintln!("taken: {}", differ(&buf, &calm));
        if mode.contains('u') { touch(&mut r, y, TouchPhase::Up); }
        common::frame(&mut r, &mut buf, w, "rel");
    }
    touch(&mut r, y, TouchPhase::Down);
    common::frame(&mut r, &mut buf, w, "held");
    eprintln!("held: {}", differ(&buf, &calm));
}
