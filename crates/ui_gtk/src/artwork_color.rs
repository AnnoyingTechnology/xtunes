use std::collections::BTreeMap;

use gtk::gdk_pixbuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArtworkPalette {
    background: RgbColor,
    foreground: RgbColor,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct QuantizedColor(u16);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ColorBucket {
    red_total: u64,
    green_total: u64,
    blue_total: u64,
    score: u64,
    pixels: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RgbColor {
    red: u8,
    green: u8,
    blue: u8,
}

impl ArtworkPalette {
    pub(crate) fn from_pixbuf(pixbuf: &gdk_pixbuf::Pixbuf) -> Option<Self> {
        dominant_color_from_pixbuf(pixbuf).map(Self::from_background)
    }

    pub(crate) fn background_css(self) -> String {
        self.background.css_hex()
    }

    pub(crate) fn background_rgb(self) -> (f64, f64, f64) {
        self.background.rgb_components()
    }

    pub(crate) fn foreground_css(self) -> String {
        self.foreground.css_hex()
    }

    fn from_background(background: RgbColor) -> Self {
        Self {
            background,
            foreground: readable_foreground(background),
        }
    }
}

fn dominant_color_from_pixbuf(pixbuf: &gdk_pixbuf::Pixbuf) -> Option<RgbColor> {
    let width = usize::try_from(pixbuf.width()).ok()?;
    let height = usize::try_from(pixbuf.height()).ok()?;
    let channels = usize::try_from(pixbuf.n_channels()).ok()?;
    let rowstride = usize::try_from(pixbuf.rowstride()).ok()?;
    if width == 0 || height == 0 || channels < 3 {
        return None;
    }

    let bytes = pixbuf.read_pixel_bytes();
    let pixels = bytes.as_ref();
    let sample_step = sample_step(width, height);
    let mut colors = Vec::new();

    for y in (0..height).step_by(sample_step) {
        let row_offset = y.checked_mul(rowstride)?;
        for x in (0..width).step_by(sample_step) {
            let offset = row_offset.checked_add(x.checked_mul(channels)?)?;
            if offset + channels > pixels.len() {
                continue;
            }

            let alpha = if channels >= 4 {
                pixels[offset + 3]
            } else {
                255
            };
            colors.push((
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                alpha,
            ));
        }
    }

    dominant_color(colors)
}

fn dominant_color<I>(colors: I) -> Option<RgbColor>
where
    I: IntoIterator<Item = (u8, u8, u8, u8)>,
{
    let mut buckets = BTreeMap::<QuantizedColor, ColorBucket>::new();

    for (red, green, blue, alpha) in colors {
        if alpha < 32 {
            continue;
        }

        let weight = color_weight(red, green, blue, alpha);
        let bucket = buckets
            .entry(QuantizedColor::from_rgb(red, green, blue))
            .or_default();
        bucket.red_total += u64::from(red);
        bucket.green_total += u64::from(green);
        bucket.blue_total += u64::from(blue);
        bucket.score += u64::from(weight);
        bucket.pixels += 1;
    }

    buckets
        .into_values()
        .max_by_key(|bucket| bucket.score)
        .and_then(|bucket| bucket.average())
}

fn sample_step(width: usize, height: usize) -> usize {
    let pixels = width.saturating_mul(height);
    let target_samples = 40_000;
    if pixels <= target_samples {
        return 1;
    }

    ((pixels as f64 / target_samples as f64).sqrt().ceil() as usize).max(1)
}

fn color_weight(red: u8, green: u8, blue: u8, alpha: u8) -> u16 {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let chroma = u16::from(max - min);
    let opacity = u16::from(alpha);

    (16 + chroma).saturating_mul(opacity) / 255
}

fn readable_foreground(background: RgbColor) -> RgbColor {
    let white = RgbColor::new(255, 255, 255);
    let black = RgbColor::new(0, 0, 0);
    if contrast_ratio(background, white) >= contrast_ratio(background, black) {
        white
    } else {
        black
    }
}

fn contrast_ratio(left: RgbColor, right: RgbColor) -> f64 {
    let left_luminance = relative_luminance(left);
    let right_luminance = relative_luminance(right);
    let lighter = left_luminance.max(right_luminance);
    let darker = left_luminance.min(right_luminance);

    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(color: RgbColor) -> f64 {
    0.2126 * linear_component(color.red)
        + 0.7152 * linear_component(color.green)
        + 0.0722 * linear_component(color.blue)
}

fn linear_component(value: u8) -> f64 {
    let normalized = f64::from(value) / 255.0;
    if normalized <= 0.04045 {
        normalized / 12.92
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4)
    }
}

impl QuantizedColor {
    fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self((u16::from(red >> 4) << 8) | (u16::from(green >> 4) << 4) | u16::from(blue >> 4))
    }
}

impl ColorBucket {
    fn average(self) -> Option<RgbColor> {
        (self.pixels > 0).then(|| {
            RgbColor::new(
                (self.red_total / self.pixels) as u8,
                (self.green_total / self.pixels) as u8,
                (self.blue_total / self.pixels) as u8,
            )
        })
    }
}

impl RgbColor {
    const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    fn css_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }

    fn rgb_components(self) -> (f64, f64, f64) {
        (
            f64::from(self.red) / 255.0,
            f64::from(self.green) / 255.0,
            f64::from(self.blue) / 255.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtworkPalette, RgbColor, dominant_color, readable_foreground, sample_step};

    #[test]
    fn dominant_color_prefers_the_largest_bucket() {
        let colors = [(20, 40, 200, 255), (21, 41, 201, 255), (220, 20, 20, 255)];

        assert_eq!(dominant_color(colors), Some(RgbColor::new(20, 40, 200)));
    }

    #[test]
    fn transparent_pixels_do_not_drive_the_palette() {
        let colors = [(255, 0, 0, 0), (0, 80, 0, 255), (0, 82, 0, 255)];

        assert_eq!(dominant_color(colors), Some(RgbColor::new(0, 81, 0)));
    }

    #[test]
    fn foreground_uses_the_higher_contrast_neutral() {
        assert_eq!(
            readable_foreground(RgbColor::new(10, 10, 10)),
            RgbColor::new(255, 255, 255)
        );
        assert_eq!(
            readable_foreground(RgbColor::new(240, 240, 240)),
            RgbColor::new(0, 0, 0)
        );
    }

    #[test]
    fn palette_formats_css_colors() {
        let palette = ArtworkPalette::from_background(RgbColor::new(12, 34, 56));

        assert_eq!(palette.background_css(), "#0c2238");
        assert_eq!(palette.foreground_css(), "#ffffff");
    }

    #[test]
    fn sample_step_caps_large_images() {
        assert_eq!(sample_step(100, 100), 1);
        assert!(sample_step(4000, 4000) > 1);
    }
}
