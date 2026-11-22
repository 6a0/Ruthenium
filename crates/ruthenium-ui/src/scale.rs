//! The GUI scale (`com/mojang/blaze3d/platform/Window.java` `calculateScale`,
//! `setGuiScale`).

/// Smallest GUI, in GUI units, that a larger scale must still leave.
const MIN_WIDTH: u32 = 320;
const MIN_HEIGHT: u32 = 240;

/// The integer GUI scale and the GUI's size in GUI units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiScale {
    pub scale: u32,
    pub width: u32,
    pub height: u32,
}

impl GuiScale {
    /// The largest scale up to `max` (the `guiScale` option; 0 = auto, as large as
    /// fits) that keeps the GUI at least 320 x 240 units. With `force_unicode` an odd
    /// scale is raised by one.
    pub fn new(pixel_width: u32, pixel_height: u32, max: u32, force_unicode: bool) -> GuiScale {
        let mut scale = 1;
        while scale != max
            && scale < pixel_width
            && scale < pixel_height
            && pixel_width / (scale + 1) >= MIN_WIDTH
            && pixel_height / (scale + 1) >= MIN_HEIGHT
        {
            scale += 1;
        }
        if force_unicode && scale % 2 != 0 {
            scale += 1;
        }
        GuiScale {
            scale,
            width: pixel_width.div_ceil(scale),
            height: pixel_height.div_ceil(scale),
        }
    }

    /// A position in window coordinates (points, as mouse events report them) in GUI
    /// units, for a window `window_size` points large.
    pub fn to_gui(&self, x: f64, y: f64, window_size: (u32, u32)) -> (f64, f64) {
        let (w, h) = (
            f64::from(window_size.0.max(1)),
            f64::from(window_size.1.max(1)),
        );
        (
            x * f64::from(self.width) / w,
            y * f64::from(self.height) / h,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_scale_keeps_320_by_240() {
        assert_eq!(
            GuiScale::new(854, 480, 0, false),
            GuiScale {
                scale: 2,
                width: 427,
                height: 240
            }
        );
        assert_eq!(
            GuiScale::new(1920, 1080, 0, false),
            GuiScale {
                scale: 4,
                width: 480,
                height: 270
            }
        );
        assert_eq!(
            GuiScale::new(2560, 1440, 0, false),
            GuiScale {
                scale: 6,
                width: 427,
                height: 240
            }
        );
        assert_eq!(GuiScale::new(300, 200, 0, false).scale, 1);
        // A fixed scale, and the unicode rounding.
        assert_eq!(GuiScale::new(1920, 1080, 2, false).scale, 2);
        assert_eq!(GuiScale::new(1920, 1080, 3, true).scale, 4);
        assert_eq!(
            GuiScale::new(1921, 1081, 2, false),
            GuiScale {
                scale: 2,
                width: 961,
                height: 541
            }
        );
    }

    #[test]
    fn mouse_to_gui_units() {
        let s = GuiScale::new(1920, 1080, 0, false);
        // A HiDPI window of 960 x 540 points over 1920 x 1080 pixels.
        assert_eq!(s.to_gui(480.0, 270.0, (960, 540)), (240.0, 135.0));
    }
}
