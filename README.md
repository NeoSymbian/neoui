# neoui

neoui is the Rust touch UI server for NeoSymbian. It will run on the 32-bit ARM
Symbian user side (`no_std` + `alloc`) and draw into a raw framebuffer, first
under QEMU Syborg (768x640), later on a Pixel 3a XL (1080x2160).

This first step proves the rendering path: a phone launcher written in Slint,
rendered by Slint's **software renderer** into a caller-owned `&mut [u32]`
XRGB8888 buffer. That is the same path the Symbian framebuffer will use; on the
Mac the buffer is just shown in a window.

| Pixel 3a XL, 1080x2160 | QEMU Syborg, 768x640 |
| --- | --- |
| ![launcher at 1080x2160](snapshots/launcher-pixel.png) | ![launcher at 768x640](snapshots/launcher-syborg.png) |

(`snapshots/launcher-*-tapped.png` show the frame after tapping Messages.)

## Layout

| Path | What |
| --- | --- |
| `crates/neoui-proto` | `#![no_std]` shared types: `Rect`, `Damage`, `InputEvent`, `SurfaceId` |
| `crates/neoui-render` | `#![no_std]` + `alloc`: Slint platform on `MinimalSoftwareWindow`; `Renderer::new(w, h, now_ms)`, `render(buf, stride) -> Damage`, `dispatch(InputEvent)` |
| `ui/launcher.slint` | The launcher: status bar, banner, 8 app tiles; sizes follow the window |
| `ui/fonts/` | Inter (OFL-1.1), bundled so the glyphs baked into the binary do not depend on the build host's fonts |
| `crates/neoui-host` | Mac/Linux desktop host: winit + softbuffer, mouse as touch |
| `snapshots/` | PNGs written by the snapshot tests |
| `symbian/` | The launcher as a Symbian EXE (`SYSSTART.EXE`, GPL-3.0): its own workspace, built for `armv5te-none-eabi` with [e32-rs](https://github.com/NeoSymbian/e32-rs) |

The renderer takes its time from a caller-supplied millisecond clock and owns
no thread, file or OS handle. Glyphs are rendered to signed distance fields at
build time (`slint-build` `sdf-fonts`), so text scales to any size without font
files on the target.

Limits of this step: one `Renderer` per process (Slint's platform is a
process-wide singleton and is built with `unsafe-single-threaded`), a single
pointer (all touch ids drive it), a static clock text, and no app launching yet.

## Run

```sh
cargo run -p neoui-host -- --size pixel    # 1080x2160 framebuffer shown at 50%
cargo run -p neoui-host -- --size syborg   # 768x640
```

Click a tile: the banner shows its name and the tile gets a halo.

## Test

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                     # unit tests + snapshots/launcher-*.png
rustup target add thumbv7em-none-eabihf
cargo build -p neoui-proto -p neoui-render --target thumbv7em-none-eabihf   # no_std check
```

The snapshot tests render both sizes, check the frame is not blank, tap the
Messages tile and check that pixels change and that every changed pixel lies
inside the reported damage.

## On Symbian (QEMU Syborg)

`symbian/` runs the launcher on the 2010 Symbian user side **instead of the window server**: estart starts
`SYSSTART.EXE` when the ROM has one. It reads the frame buffer from the HAL, registers as the window server to take
raw pointer and key events from the kernel, renders into a RAM frame and copies the damaged rectangles to the screen.
On Syborg the first frame takes about 80 ms, and tapping the Phone tile opens the Phone screen.

```sh
cd symbian && cargo arm        # target/armv5te-none-eabi/release/libneoui_symbian.a (nightly pinned here)
# with NeoSymbian/symbian-2010: copy the .a and exes.txt into its rust/, then
./build.sh && MONITOR="mouse_move 3243 27904;mouse_button 1;mouse_button 0" ./boot.sh   # out/fb.png, out/serial.log
```

The `syborg` CI job does exactly that and checks the serial log and the screen. It publishes the screenshot and the
log, never the ROM (see the licence note below).

## Licence

Apache-2.0, see `LICENSE`. Dependencies carry their own licences, notably:

- **Slint** is GPL-3.0-only, or the Slint Royalty-free Desktop/Mobile/Web
  licence, or a commercial licence. The royalty-free licence excludes embedded
  systems and applications that expose Slint's APIs. A launcher/UI server that
  is part of a phone OS may fall outside it, so decide the Slint licence before
  shipping neoui in a NeoSymbian image. For now (2026-11-06) the Symbian EXE in `symbian/` uses Slint under
  GPL-3.0 and is itself GPL-3.0; ROM images that contain it are not distributed.
- **Inter** font (`ui/fonts/`): OFL-1.1-RFN.
