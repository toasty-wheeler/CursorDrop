//! A thin layer over GDI+ (the same renderer DS4-Native and MagicMouseNative paint with), plus
//! the off-screen canvas the window is painted into and cursor images drawn with GDI.

use std::collections::HashMap;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::cursors::wide;

pub type Color = u32; // 0xAARRGGBB

pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}
pub fn alpha(c: Color) -> u8 {
    (c >> 24) as u8
}
pub fn with_alpha(c: Color, a: i32) -> Color {
    (c & 0x00FF_FFFF) | ((a.clamp(0, 255) as u32) << 24)
}
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let ch = |s: u32| {
        let x = ((a >> s) & 0xFF) as f32;
        let y = ((b >> s) & 0xFF) as f32;
        ((x + (y - x) * t + 0.5) as u32) << s
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}
pub const WHITE: Color = argb(255, 255, 255, 255);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn inset(&self, dx: f32, dy: f32) -> R {
        R::new(self.x + dx, self.y + dy, self.w - 2.0 * dx, self.h - 2.0 * dy)
    }
    pub fn intersects(&self, o: &R) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    fn gp(&self) -> RectF {
        RectF { X: self.x, Y: self.y, Width: self.w, Height: self.h }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Near,
    Center,
    Far,
}

pub fn startup() {
    let input = GdiplusStartupInput { GdiplusVersion: 1, ..Default::default() };
    let mut token = 0usize;
    unsafe { GdiplusStartup(&mut token, &input, null_mut()) };
}

pub struct Font(*mut GpFont);

impl Font {
    pub fn new(family: &str, px: f32) -> Font {
        let name = wide(family);
        let mut fam = null_mut();
        let mut font = null_mut();
        unsafe {
            if GdipCreateFontFamilyFromName(name.as_ptr(), null_mut(), &mut fam) != 0 {
                let fallback = wide("Segoe UI");
                GdipCreateFontFamilyFromName(fallback.as_ptr(), null_mut(), &mut fam);
            }
            GdipCreateFont(fam, px, 0, UnitPixel, &mut font);
            GdipDeleteFontFamily(fam);
        }
        Font(font)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe { GdipDeleteFont(self.0) };
    }
}

/// The fonts DS4-Native uses, at the window's DPI.
pub struct Fonts {
    pub title: Font,
    pub body: Font,
    pub caption: Font,
    pub section: Font,
    pub icon: Font,
    pub glyph: Font,
    pub tiny: Font,
    pub big_icon: Font,
}

impl Fonts {
    pub fn new(scale: f32) -> Fonts {
        Fonts {
            title: Font::new("Segoe UI Semibold", 20.0 * scale),
            body: Font::new("Segoe UI", 14.0 * scale),
            caption: Font::new("Segoe UI", 12.0 * scale),
            section: Font::new("Segoe UI Semibold", 12.0 * scale),
            icon: Font::new("Segoe MDL2 Assets", 15.0 * scale),
            glyph: Font::new("Segoe MDL2 Assets", 10.0 * scale),
            tiny: Font::new("Segoe UI Semibold", 10.0 * scale),
            big_icon: Font::new("Segoe MDL2 Assets", 28.0 * scale),
        }
    }
}

pub struct Gfx {
    g: *mut GpGraphics,
}

fn brush(c: Color) -> *mut GpSolidFill {
    let mut b = null_mut();
    unsafe { GdipCreateSolidFill(c, &mut b) };
    b
}

fn round_path(r: R, radius: f32) -> *mut GpPath {
    let mut p = null_mut();
    unsafe {
        GdipCreatePath(FillModeAlternate, &mut p);
        let d = (radius * 2.0).min(r.w).min(r.h);
        if d <= 0.5 {
            GdipAddPathRectangle(p, r.x, r.y, r.w, r.h);
        } else {
            GdipAddPathArc(p, r.x, r.y, d, d, 180.0, 90.0);
            GdipAddPathArc(p, r.right() - d, r.y, d, d, 270.0, 90.0);
            GdipAddPathArc(p, r.right() - d, r.bottom() - d, d, d, 0.0, 90.0);
            GdipAddPathArc(p, r.x, r.bottom() - d, d, d, 90.0, 90.0);
            GdipClosePathFigure(p);
        }
    }
    p
}

impl Gfx {
    pub fn from_hdc(hdc: HDC) -> Gfx {
        let mut g = null_mut();
        unsafe {
            GdipCreateFromHDC(hdc, &mut g);
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
            GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        }
        Gfx { g }
    }

    pub fn clear(&self, c: Color) {
        unsafe { GdipGraphicsClear(self.g, c) };
    }


    pub fn fill_round(&self, r: R, radius: f32, c: Color) {
        if alpha(c) == 0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let p = round_path(r, radius);
        let b = brush(c);
        unsafe {
            GdipFillPath(self.g, b.cast(), p);
            GdipDeleteBrush(b.cast());
            GdipDeletePath(p);
        }
    }

    pub fn stroke_round(&self, r: R, radius: f32, c: Color, width: f32) {
        let p = round_path(r, radius);
        let mut pen = null_mut();
        unsafe {
            GdipCreatePen1(c, width, UnitPixel, &mut pen);
            GdipDrawPath(self.g, pen, p);
            GdipDeletePen(pen);
            GdipDeletePath(p);
        }
    }

    /// A dashed outline; moving `offset` (in dash lengths) makes the dashes march.
    pub fn dashed_round(&self, r: R, radius: f32, c: Color, width: f32, offset: f32) {
        let p = round_path(r, radius);
        let mut pen = null_mut();
        unsafe {
            GdipCreatePen1(c, width, UnitPixel, &mut pen);
            GdipSetPenDashStyle(pen, DashStyleDash);
            GdipSetPenDashOffset(pen, offset);
            GdipDrawPath(self.g, pen, p);
            GdipDeletePen(pen);
            GdipDeletePath(p);
        }
    }

    pub fn ellipse(&self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let b = brush(c);
        unsafe {
            GdipFillEllipse(self.g, b.cast(), x, y, w, h);
            GdipDeleteBrush(b.cast());
        }
    }

    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, c: Color, width: f32) {
        let mut pen = null_mut();
        unsafe {
            GdipCreatePen1(c, width, UnitPixel, &mut pen);
            GdipDrawLine(self.g, pen, x1, y1, x2, y2);
            GdipDeletePen(pen);
        }
    }

    /// A soft elliptical glow fading to transparent (the ambient light behind the content).
    pub fn glow(&self, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
        unsafe {
            let mut p = null_mut();
            GdipCreatePath(FillModeAlternate, &mut p);
            GdipAddPathEllipse(p, cx - rx, cy - ry, rx * 2.0, ry * 2.0);
            let mut grad = null_mut();
            GdipCreatePathGradientFromPath(p, &mut grad);
            GdipSetPathGradientCenterColor(grad, c);
            let edge = with_alpha(c, 0);
            let mut n = 1;
            GdipSetPathGradientSurroundColorsWithCount(grad, &edge, &mut n);
            GdipFillPath(self.g, grad.cast(), p);
            GdipDeleteBrush(grad.cast());
            GdipDeletePath(p);
        }
    }

    pub fn text(&self, s: &str, font: &Font, c: Color, r: R, h: Align, v: Align) {
        if s.is_empty() {
            return;
        }
        let w: Vec<u16> = s.encode_utf16().collect();
        let a = |x: Align| match x {
            Align::Near => 0,
            Align::Center => 1,
            Align::Far => 2,
        };
        unsafe {
            let mut fmt = null_mut();
            GdipCreateStringFormat(0x1000 /* NoWrap */, 0, &mut fmt);
            GdipSetStringFormatAlign(fmt, a(h));
            GdipSetStringFormatLineAlign(fmt, a(v));
            GdipSetStringFormatTrimming(fmt, StringTrimmingEllipsisCharacter);
            let b = brush(c);
            let rect = r.gp();
            GdipDrawString(self.g, w.as_ptr(), w.len() as i32, font.0, &rect, fmt, b.cast());
            GdipDeleteBrush(b.cast());
            GdipDeleteStringFormat(fmt);
        }
    }

    /// Left-aligned, vertically centred text.
    pub fn label(&self, s: &str, font: &Font, c: Color, r: R) {
        self.text(s, font, c, r, Align::Near, Align::Center);
    }

    pub fn measure(&self, s: &str, font: &Font) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let layout = RectF { X: 0.0, Y: 0.0, Width: 10000.0, Height: 1000.0 };
        let mut out = RectF::default();
        unsafe {
            let mut fmt = null_mut();
            GdipCreateStringFormat(0x1000, 0, &mut fmt);
            GdipMeasureString(self.g, w.as_ptr(), w.len() as i32, font.0, &layout, fmt, &mut out, null_mut(), null_mut());
            GdipDeleteStringFormat(fmt);
        }
        out.Width
    }

    pub fn clip(&self, r: R) {
        unsafe { GdipSetClipRect(self.g, r.x, r.y, r.w, r.h, CombineModeReplace) };
    }

    pub fn unclip(&self) {
        unsafe { GdipResetClip(self.g) };
    }

    /// Draws a cursor (or icon) with GDI, which is what animates .ani files and keeps their alpha.
    pub fn cursor(&self, h: HICON, x: f32, y: f32, size: f32, step: u32, clip: Option<R>) {
        if h.is_null() {
            return;
        }
        unsafe {
            let mut hdc: HDC = null_mut();
            if GdipGetDC(self.g, &mut hdc) != 0 {
                return;
            }
            let saved = SaveDC(hdc);
            if let Some(c) = clip {
                IntersectClipRect(hdc, c.x.floor() as i32, c.y.floor() as i32, c.right().ceil() as i32, c.bottom().ceil() as i32);
            }
            let s = size.round() as i32;
            DrawIconEx(hdc, x.round() as i32, y.round() as i32, h, s, s, step, null_mut(), DI_NORMAL);
            RestoreDC(hdc, saved);
            GdipReleaseDC(self.g, hdc);
        }
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.g) };
    }
}

/// A 32-bit top-down DIB the window is painted into, then copied to the screen in one go.
pub struct Canvas {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    pub w: i32,
    pub h: i32,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w.max(1);
            bi.bmiHeader.biHeight = -h.max(1);
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            let old = SelectObject(dc, bmp);
            Canvas { dc, bmp, old, w, h }
        }
    }

    pub fn save_png(&self, path: &str) -> bool {
        // {557CF406-1A04-11D3-9A73-0000F81EF32E}
        let png = windows_sys::core::GUID {
            data1: 0x557CF406,
            data2: 0x1A04,
            data3: 0x11D3,
            data4: [0x9A, 0x73, 0x00, 0x00, 0xF8, 0x1E, 0xF3, 0x2E],
        };
        unsafe {
            SelectObject(self.dc, self.old);
            let mut bitmap = null_mut();
            GdipCreateBitmapFromHBITMAP(self.bmp, null_mut(), &mut bitmap);
            let p = wide(path);
            let ok = GdipSaveImageToFile(bitmap.cast(), p.as_ptr(), &png, null()) == 0;
            GdipDisposeImage(bitmap.cast());
            SelectObject(self.dc, self.bmp);
            ok
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

// ---------------------------------------------------------------- cursor images

/// A loaded cursor file: the handle plus its animation timing (from the .ani header).
pub struct CursorImg {
    pub h: HCURSOR,
    /// Display time of each step in 1/60 s; empty for still cursors.
    rates: Vec<u32>,
}

impl CursorImg {
    /// The animation step to show `ms` milliseconds into the animation.
    pub fn step(&self, ms: u64) -> u32 {
        let total: u64 = self.rates.iter().map(|&r| r.max(1) as u64).sum();
        if total == 0 {
            return 0;
        }
        let mut t = (ms * 60 / 1000) % total;
        for (i, &r) in self.rates.iter().enumerate() {
            let r = r.max(1) as u64;
            if t < r {
                return i as u32;
            }
            t -= r;
        }
        0
    }
}

impl Drop for CursorImg {
    fn drop(&mut self) {
        unsafe { DestroyCursor(self.h) };
    }
}

/// Reads step count and per-step rates from an .ani file's RIFF chunks.
fn ani_rates(path: &str) -> Vec<u32> {
    let b = match std::fs::read(path) {
        std::result::Result::Ok(b) => b,
        _ => return vec![],
    };
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"ACON" {
        return vec![];
    }
    let u32_at = |i: usize| b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let (mut steps, mut disp, mut rates) = (0u32, 0u32, Vec::new());
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let size = u32_at(i + 4).unwrap_or(0) as usize;
        let body = i + 8;
        if id == b"anih" && size >= 36 {
            steps = u32_at(body + 8).unwrap_or(0);
            disp = u32_at(body + 28).unwrap_or(0);
        } else if id == b"rate" {
            rates = (0..size / 4).filter_map(|k| u32_at(body + k * 4)).collect();
        }
        i = body + size + (size & 1);
    }
    let steps = steps.min(4096) as usize;
    if steps <= 1 {
        return vec![];
    }
    if rates.len() < steps {
        rates = vec![disp.max(1); steps];
    }
    rates.truncate(steps);
    rates
}

/// Cache of cursor files loaded at a given pixel size.
#[derive(Default)]
pub struct CursorCache {
    map: HashMap<(String, i32), Option<CursorImg>>,
}

impl CursorCache {
    pub fn get(&mut self, path: &str, size: i32) -> Option<&CursorImg> {
        self.map
            .entry((path.to_string(), size))
            .or_insert_with(|| {
                let w = wide(path);
                let h = unsafe { LoadImageW(null_mut(), w.as_ptr(), IMAGE_CURSOR, size, size, LR_LOADFROMFILE) };
                if h.is_null() {
                    return None;
                }
                let rates = if path.to_ascii_lowercase().ends_with(".ani") { ani_rates(path) } else { vec![] };
                Some(CursorImg { h: h as HCURSOR, rates })
            })
            .as_ref()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

pub fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, null(), FALSE) };
}

// ---------------------------------------------------------------- app icon

/// Draws the CursorDrop icon (a gradient tile with a white pointer) at `px` pixels into an
/// ARGB bitmap and returns it as PNG bytes.
fn icon_png(px: i32, tmp: &std::path::Path) -> Option<Vec<u8>> {
    const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;
    unsafe {
        let mut bmp = null_mut();
        if GdipCreateBitmapFromScan0(px, px, 0, PIXEL_FORMAT_32BPP_ARGB, null(), &mut bmp) != 0 {
            return None;
        }
        let mut g = null_mut();
        GdipGetImageGraphicsContext(bmp.cast(), &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        GdipGraphicsClear(g, 0);
        let k = px as f32 / 256.0;
        let gfx = Gfx { g };

        // Tile: pink to violet, corner to corner, with a soft light along the top.
        let tile = R::new(10.0 * k, 10.0 * k, 236.0 * k, 236.0 * k);
        let path = round_path(tile, 58.0 * k);
        let (p1, p2) = (PointF { X: tile.x, Y: tile.y }, PointF { X: tile.right(), Y: tile.bottom() });
        let mut grad = null_mut();
        GdipCreateLineBrush(&p1, &p2, argb(255, 255, 79, 139), argb(255, 123, 92, 255), WrapModeTileFlipXY, &mut grad);
        GdipFillPath(g, grad.cast(), path);
        GdipDeleteBrush(grad.cast());
        GdipDeletePath(path);
        gfx.glow(128.0 * k, 40.0 * k, 150.0 * k, 70.0 * k, argb(70, 255, 255, 255));

        // The pointer: the classic arrow, white with a dark outline and a little shadow.
        let pts: [(f32, f32); 7] = [(86.0, 46.0), (86.0, 196.0), (121.0, 163.0), (146.0, 216.0), (171.0, 204.0), (146.0, 152.0), (194.0, 152.0)];
        let draw = |dx: f32, dy: f32, fill: Color, outline: Option<(Color, f32)>| {
            let p: Vec<PointF> = pts.iter().map(|&(x, y)| PointF { X: (x + dx) * k, Y: (y + dy) * k }).collect();
            let mut path = null_mut();
            GdipCreatePath(FillModeAlternate, &mut path);
            GdipAddPathPolygon(path, p.as_ptr(), p.len() as i32);
            GdipClosePathFigure(path);
            if let Some((c, w)) = outline {
                let mut pen = null_mut();
                GdipCreatePen1(c, (w * k).max(1.0), UnitPixel, &mut pen);
                GdipSetPenLineJoin(pen, LineJoinRound);
                GdipDrawPath(g, pen, path);
                GdipDeletePen(pen);
            }
            let b = brush(fill);
            GdipFillPath(g, b.cast(), path);
            GdipDeleteBrush(b.cast());
            GdipDeletePath(path);
        };
        draw(6.0, 8.0, argb(70, 40, 0, 60), Some((argb(70, 40, 0, 60), 20.0)));
        draw(0.0, 0.0, WHITE, Some((argb(255, 28, 28, 30), 20.0)));
        std::mem::forget(gfx); // `g` is deleted below, after the image it draws on
        GdipDeleteGraphics(g);

        let file = tmp.join(format!("icon-{px}.png"));
        let png = windows_sys::core::GUID {
            data1: 0x557CF406,
            data2: 0x1A04,
            data3: 0x11D3,
            data4: [0x9A, 0x73, 0x00, 0x00, 0xF8, 0x1E, 0xF3, 0x2E],
        };
        let w = wide(&file.to_string_lossy());
        let ok = GdipSaveImageToFile(bmp.cast(), w.as_ptr(), &png, null()) == 0;
        GdipDisposeImage(bmp.cast());
        let bytes = if ok { std::fs::read(&file).ok() } else { None };
        let _ = std::fs::remove_file(&file);
        bytes
    }
}

/// Writes a multi-size .ico (PNG-compressed images, which Windows Vista and later read).
/// Used to make assets\CursorDrop.ico: `CursorDrop.exe --make-icon <file.ico>`.
pub fn make_icon(out: &std::path::Path) -> Result<(), String> {
    startup();
    let sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
    let tmp = std::env::temp_dir();
    let images: Vec<(i32, Vec<u8>)> =
        sizes.iter().map(|&s| icon_png(s, &tmp).map(|b| (s, b)).ok_or(format!("could not draw {s} px"))).collect::<Result<_, _>>()?;
    let mut ico = Vec::new();
    ico.extend([0u8, 0, 1, 0]);
    ico.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, data) in &images {
        let d = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend([d, d, 0, 0]);
        ico.extend(1u16.to_le_bytes());
        ico.extend(32u16.to_le_bytes());
        ico.extend((data.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        ico.extend(data);
    }
    std::fs::write(out, ico).map_err(|e| e.to_string())
}
