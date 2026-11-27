// SPDX-License-Identifier: Apache-2.0
//! (D7) Home's wallpaper picture (`SETTINGS_ACTION` 8 `INSTALL_WALLPAPER`): the shell hands the installed raster
//! here; home draws it, covering the screen, while `display.wallpaper` is `picture`
//! (`ui/components/wallpaper.slint`'s `WallpaperPicture`).

use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer};

use crate::{Renderer, WallpaperPicture};

impl Renderer {
    /// The picture home draws: tightly packed RGB888, `w`×`h`, top-down; `None` (or a buffer of the wrong length)
    /// takes it away, and home draws the colour behind it.
    pub fn set_wallpaper_picture(&mut self, picture: Option<(&[u8], u32, u32)>) {
        let g = self.ui.global::<WallpaperPicture>();
        match picture {
            Some((px, w, h)) if w > 0 && h > 0 && px.len() == w as usize * h as usize * 3 => {
                let buf = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(px, w, h);
                g.set_image(Image::from_rgb8(buf));
                g.set_shown(true);
            }
            _ => {
                g.set_shown(false);
                g.set_image(Image::default());
            }
        }
    }

    /// Whether home has a picture to draw (the store's key decides whether it does).
    #[must_use]
    pub fn has_wallpaper_picture(&self) -> bool {
        self.ui.global::<WallpaperPicture>().get_shown()
    }
}
