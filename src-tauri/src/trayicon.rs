#![allow(unsafe_op_in_unsafe_fn)]
//! The live tray icon: the current CPU, GPU or RAM percentage drawn as a number on a coloured square.
//!
//! The digits are rendered by GDI (Segoe UI Bold, grey-scale anti-aliasing) white on black into a scratch
//! bitmap, whose brightness then serves as the text's coverage; that ink is centred on the square. Three
//! digits ("100") are drawn condensed so they fit.

/// What the tray icon shows (Settings > Appearance).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Logo,
    Cpu,
    Gpu,
    Ram,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "cpu" => Mode::Cpu,
            "gpu" => Mode::Gpu,
            "ram" => Mode::Ram,
            _ => Mode::Logo,
        }
    }

    pub fn from_u8(v: u8) -> Mode {
        [Mode::Logo, Mode::Cpu, Mode::Gpu, Mode::Ram].get(v as usize).copied().unwrap_or(Mode::Logo)
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Square colour: the module colours of the popup, darkened so white digits stay readable on it.
    fn color(self) -> [u8; 3] {
        match self {
            Mode::Cpu => [0x2f, 0x62, 0xd8],
            Mode::Gpu => [0xc8, 0x2a, 0x62],
            Mode::Ram => [0x76, 0x45, 0xd0],
            Mode::Logo => [0x40, 0x40, 0x40],
        }
    }
}

/// The small-icon size the taskbar uses (16 px at 100 % scaling, 24 px at 150 %).
#[cfg(windows)]
pub fn icon_size() -> u32 {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    let px = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    if px <= 0 { 16 } else { (px as u32).clamp(16, 64) }
}

#[cfg(not(windows))]
pub fn icon_size() -> u32 {
    16
}

/// `value` (0..100) as an RGBA icon of `size` x `size` pixels.
pub fn render(mode: Mode, value: u32, size: u32) -> Option<Vec<u8>> {
    let text = value.min(100).to_string();
    let ink = text_coverage(&text, size)?;
    Some(compose(&ink, size, mode.color()))
}

/// Text coverage (0..255) cropped to its ink, with its width and height.
struct Ink {
    cov: Vec<u8>,
    w: usize,
    h: usize,
}

/// Rounded square in `bg`, the ink centred on it in white.
fn compose(ink: &Ink, size: u32, bg: [u8; 3]) -> Vec<u8> {
    let s = size as usize;
    let mut out = vec![0u8; s * s * 4];
    let radius = size as f32 * 0.22;
    let (ox, oy) = ((s as isize - ink.w as isize) / 2, (s as isize - ink.h as isize + 1) / 2);
    for y in 0..s {
        for x in 0..s {
            let shape = rounded_square_coverage(x as f32 + 0.5, y as f32 + 0.5, size as f32, radius);
            if shape <= 0.0 {
                continue;
            }
            let (ix, iy) = (x as isize - ox, y as isize - oy);
            let text = if ix >= 0 && iy >= 0 && (ix as usize) < ink.w && (iy as usize) < ink.h {
                ink.cov[iy as usize * ink.w + ix as usize] as f32 / 255.0
            } else {
                0.0
            };
            let i = (y * s + x) * 4;
            for c in 0..3 {
                out[i + c] = (bg[c] as f32 * (1.0 - text) + 255.0 * text).round() as u8;
            }
            out[i + 3] = (shape * 255.0).round() as u8;
        }
    }
    out
}

/// How much of the pixel centred at (px, py) lies inside a `size` square with corners rounded by `r` (0..1).
fn rounded_square_coverage(px: f32, py: f32, size: f32, r: f32) -> f32 {
    let half = size / 2.0;
    let (dx, dy) = ((px - half).abs() - (half - r), (py - half).abs() - (half - r));
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r;
    (0.5 - outside).clamp(0.0, 1.0)
}

#[cfg(windows)]
fn text_coverage(text: &str, size: u32) -> Option<Ink> {
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, SIZE};
    use windows::Win32::Graphics::Gdi::*;

    let s = size as i32;
    // Room around the text so nothing is clipped before it is cropped to its ink.
    let (cw, ch) = (s * 3, s * 2);
    let room = s - 2 - (s / 12); // ink may use the square minus a thin margin
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: cw,
                biHeight: -ch,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old_bmp = SelectObject(dc, HGDIOBJ(bmp.0));
        let _ = SetBkMode(dc, TRANSPARENT);
        let _ = SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let px = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (cw * ch * 4) as usize);

        // Digit height about 62 % of the icon; condensed (narrower average width) when the text is too wide.
        let em = (s as f32 * 0.86).round() as i32;
        let mut avg_width = 0;
        let mut result = None;
        for _ in 0..4 {
            let font = CreateFontW(
                -em,
                avg_width,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
                w!("Segoe UI"),
            );
            let old_font = SelectObject(dc, HGDIOBJ(font.0));
            px.fill(0);
            let _ = TextOutW(dc, s / 2, s / 4, &wide);
            let _ = GdiFlush();
            let ink = crop(px, cw as usize, ch as usize);
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut tm);
            let mut extent = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &wide, &mut extent);
            SelectObject(dc, old_font);
            let _ = DeleteObject(HGDIOBJ(font.0));
            match ink {
                Some(ink) if ink.w as i32 > room && tm.tmAveCharWidth > 1 => {
                    // Narrower glyphs, in proportion to how much too wide the ink is.
                    let current = if avg_width > 0 { avg_width } else { tm.tmAveCharWidth };
                    avg_width = ((current as f32 * room as f32 / ink.w as f32).floor() as i32).max(1);
                    result = Some(ink);
                }
                other => {
                    result = other;
                    break;
                }
            }
        }
        SelectObject(dc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(dc);
        result
    }
}

#[cfg(not(windows))]
fn text_coverage(_text: &str, _size: u32) -> Option<Ink> {
    None
}

/// The drawn text's bounding box (BGRA scratch bitmap, white text on black) as coverage values.
fn crop(px: &[u8], w: usize, h: usize) -> Option<Ink> {
    let cov = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        px[i].max(px[i + 1]).max(px[i + 2])
    };
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if cov(x, y) > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    let (iw, ih) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut out = Vec::with_capacity(iw * ih);
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(cov(x, y));
        }
    }
    Some(Ink { cov: out, w: iw, h: ih })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip() {
        for m in [Mode::Logo, Mode::Cpu, Mode::Gpu, Mode::Ram] {
            assert_eq!(Mode::from_u8(m.as_u8()), m);
        }
        assert_eq!(Mode::parse("gpu"), Mode::Gpu);
        assert_eq!(Mode::parse("anything"), Mode::Logo);
    }

    #[test]
    fn the_square_has_soft_corners_and_a_solid_middle() {
        assert_eq!(rounded_square_coverage(12.0, 12.0, 24.0, 5.0), 1.0);
        assert_eq!(rounded_square_coverage(0.5, 0.5, 24.0, 5.0), 0.0, "corner pixel is outside the rounding");
        assert_eq!(rounded_square_coverage(12.0, 0.5, 24.0, 5.0), 1.0, "edge middle is inside");
    }

    #[test]
    #[cfg(windows)]
    fn every_value_fits_inside_the_icon_with_a_margin() {
        for size in [16u32, 20, 24, 32] {
            for v in [0u32, 7, 42, 88, 100] {
                let ink = text_coverage(&v.to_string(), size).expect("text is drawn");
                assert!(ink.w as u32 <= size - 2 && ink.h as u32 <= size - 2, "{v} at {size}px: {}x{}", ink.w, ink.h);
                assert!(ink.h as u32 >= size / 2, "{v} at {size}px is too small to read: {} px tall", ink.h);
                let icon = render(Mode::Cpu, v, size).unwrap();
                assert_eq!(icon.len(), (size * size * 4) as usize);
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn drawing_many_icons_leaks_no_gdi_objects() {
        use windows::Win32::System::Threading::{GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS};
        let count = || unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        render(Mode::Cpu, 1, 24); // first call may set up GDI's own caches
        let before = count();
        for v in 0..300 {
            render(Mode::Gpu, v % 101, 24).unwrap();
        }
        assert!(count() <= before + 2, "GDI objects grew from {before} to {}", count());
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "writes sample icons to the temp folder for a look: run with --ignored"]
    fn write_samples() {
        let dir = std::env::temp_dir().join("pulsetray-tray-icons");
        std::fs::create_dir_all(&dir).unwrap();
        for size in [16u32, 24, 32] {
            for (mode, v) in [(Mode::Cpu, 100u32), (Mode::Cpu, 7), (Mode::Gpu, 85), (Mode::Ram, 71)] {
                let rgba = render(mode, v, size).unwrap();
                let png = crate::apps::encode_png(&rgba, size, size).unwrap();
                std::fs::write(dir.join(format!("{mode:?}-{v}-{size}.png")), png).unwrap();
            }
        }
        println!("{}", dir.display());
    }
}
