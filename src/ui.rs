//! The CursorDrop window, painted the same way as DS4-Native / MagicMouseNative: GDI+ on a
//! back buffer, translucent rounded cards over a soft accent glow, Segoe UI, Windows' accent
//! colour, and an Automatic / Light / Dark theme menu with an always-on-top pin beside it.
//!
//! Two views: the packs in the cursors folder as an Explorer-style details list, and, after
//! clicking one, a grid of every cursor it replaces (what you have now and what you get).

use crate::cursors::{self, wide};
use crate::gfx::*;
use crate::scheme::{ROLE_LABELS, ROLES, Scheme};
use crate::{Active, App, Installed};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::Instant;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Globalization::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows_sys::Win32::System::Time::*;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// Segoe MDL2 Assets glyphs (the same ones DS4-Native uses where they overlap).
const G_BACK: &str = "\u{E72B}";
const G_ARROW: &str = "\u{E72A}";
const G_MOUSE: &str = "\u{E962}";
const G_SUN: &str = "\u{E706}";
const G_MOON: &str = "\u{E708}";
const G_PIN: &str = "\u{E718}";
const G_PINNED: &str = "\u{E840}";
const G_CHEVRON: &str = "\u{E70D}";
const G_CHEVRON_UP: &str = "\u{E70E}";
const G_CHECK: &str = "\u{E73E}";
const G_DROP: &str = "\u{E896}";
const G_WARNING: &str = "\u{E7BA}";
const G_LEFT: &str = "\u{E76B}";
const G_RIGHT: &str = "\u{E76C}";

pub const THEME_AUTO: u8 = 0;
pub const THEME_LIGHT: u8 = 1;
pub const THEME_DARK: u8 = 2;
const THEME_NAMES: [&str; 3] = ["Automatic", "Light", "Dark"];

const COLUMN_NAMES: [&str; 5] = ["Name", "Date modified", "Type", "Size", "Cursors"];
const MIN_COLUMN: f32 = 60.0;

// Buttons.
const BTN_ADD: usize = 1;
const BTN_FOLDER: usize = 2;
const BTN_RESTORE: usize = 3;
const BTN_APPLY: usize = 4;
const BTN_SHOW: usize = 5;
const BTN_REMOVE: usize = 6;

// Layout, in 96-dpi units (like DS4-Native).
const MARGIN: f32 = 24.0;
const FOOTER_H: f32 = 72.0;
const HEADER_ROW: f32 = 34.0;
const ROW_H: f32 = 40.0;
const TILE_MIN_W: f32 = 176.0;
const TILE_H: f32 = 158.0;
const TILE_GAP: f32 = 12.0;
const TOGGLE_W: f32 = 236.0;
const TAB_MIN: f32 = 150.0;
const TAB_MAX: f32 = 240.0;
const TAB_H: f32 = 34.0;
const TAB_ARROW: f32 = 34.0;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub theme: u8,
    pub on_top: bool,
    pub columns: [f32; 5],
    pub sort_column: usize,
    pub sort_ascending: bool,
    /// Closing the window keeps CursorDrop in the tray; off: closing quits.
    pub close_to_tray: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        UiSettings {
            theme: THEME_AUTO,
            on_top: false,
            columns: [290.0, 160.0, 180.0, 90.0, 200.0],
            sort_column: 0,
            sort_ascending: true,
            close_to_tray: true,
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum View {
    List,
    Detail { archive: String, variant: usize },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Theme,
    ThemeItem,
    Pin,
    Back,
    Row,
    ColHeader,
    ColSplit,
    Toggle,
    Button,
    Segment,
    TabPrev,
    TabNext,
    Tile,
    Thumb,
}

type Hit = (Kind, usize);

struct Elem {
    kind: Kind,
    id: usize,
    r: R,
}

#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    card: Color,
    text: Color,
    subtext: Color,
    line: Color,
    hover: Color,
    switch_off: Color,
    segment: Color,
    segment_on: Color,
    shadow: Color,
}

const LIGHT: Palette = Palette {
    bg: argb(255, 242, 242, 247),
    card: argb(215, 255, 255, 255),
    text: argb(255, 28, 28, 30),
    subtext: argb(255, 110, 110, 115),
    line: argb(255, 229, 229, 234),
    hover: argb(255, 236, 236, 241),
    switch_off: argb(255, 214, 214, 219),
    segment: argb(255, 229, 229, 234),
    segment_on: argb(255, 255, 255, 255),
    shadow: argb(40, 0, 0, 0),
};

const DARK: Palette = Palette {
    bg: argb(255, 28, 28, 30),
    card: argb(200, 44, 44, 46),
    text: argb(255, 242, 242, 247),
    subtext: argb(255, 152, 152, 157),
    line: argb(255, 58, 58, 60),
    hover: argb(255, 58, 58, 60),
    switch_off: argb(255, 72, 72, 74),
    segment: argb(255, 58, 58, 60),
    segment_on: argb(255, 99, 99, 102),
    shadow: argb(90, 0, 0, 0),
};

const WARNING: Color = argb(255, 255, 159, 10);

pub struct Ui {
    pub hwnd: HWND,
    pub scale: f32,
    fonts: Option<Fonts>,
    pub view: View,
    elems: Vec<Elem>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    tracking: bool,
    scroll: f32,
    scroll_max: f32,
    scroll_area: R,
    thumb_drag: Option<(f32, f32)>,
    col_drag: Option<(usize, f32, f32)>,
    theme_open: bool,
    pub dark: bool,
    accent: Color,
    knobs: [f32; 2],
    /// Files being dragged over the window (None when nothing is), what the overlay last
    /// showed (kept while it fades out), and how far it has faded in.
    drag: Option<DragInfo>,
    drag_shown: Option<DragInfo>,
    drag_t: f32,
    /// Files dropped on the window, handled after the drag finishes (WM_APP + 3).
    pub dropped: Vec<PathBuf>,
    pub cursors: CursorCache,
    shell: HashMap<String, (HICON, String)>,
    canvas: Option<Canvas>,
    t0: Instant,
    row_order: Vec<String>,
    animated: bool,
    pub told_about_tray: bool,
    mouse: Option<(f32, f32)>,
    tabs: TabStrip,
}

/// The variant tabs: browser-style, with a minimum width; when they don't fit they scroll
/// (arrows at the ends, or the mouse wheel over them) and ease towards `target`.
#[derive(Default)]
struct TabStrip {
    scroll: f32,
    target: f32,
    max: f32,
    /// Where the strip is (for the wheel) and one tab's width (the arrows' step).
    area: R,
    tab_w: f32,
    /// Bring the selected tab into view on the next paint; `snap` skips the easing.
    reveal: bool,
    snap: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            hwnd: null_mut(),
            scale: 1.0,
            fonts: None,
            view: View::List,
            elems: vec![],
            hover: None,
            pressed: None,
            tracking: false,
            scroll: 0.0,
            scroll_max: 0.0,
            scroll_area: R::default(),
            thumb_drag: None,
            col_drag: None,
            theme_open: false,
            dark: false,
            accent: argb(255, 10, 132, 255),
            knobs: [-1.0; 2],
            drag: None,
            drag_shown: None,
            drag_t: 0.0,
            dropped: vec![],
            cursors: CursorCache::default(),
            shell: HashMap::new(),
            canvas: None,
            t0: Instant::now(),
            row_order: vec![],
            animated: false,
            told_about_tray: false,
            tabs: TabStrip::default(),
            mouse: None,
        }
    }
}

// ---------------------------------------------------------------- theme

fn system_uses_dark_apps() -> bool {
    let v = crate::cursors::get_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme");
    v == Some(0)
}

fn accent_colour() -> Color {
    match crate::cursors::get_dword(r"Software\Microsoft\Windows\DWM", "AccentColor") {
        Some(abgr) => argb(255, abgr as u8, (abgr >> 8) as u8, (abgr >> 16) as u8),
        None => argb(255, 10, 132, 255),
    }
}

/// Text on the accent colour: white, or near-black on a light accent.
fn on_accent(accent: Color) -> Color {
    let (r, g, b) = (((accent >> 16) & 255) as f32, ((accent >> 8) & 255) as f32, (accent & 255) as f32);
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 165.0 { argb(255, 28, 28, 30) } else { WHITE }
}

pub fn apply_theme(app: &mut App) {
    let mode = app.state.ui.theme;
    app.ui.dark = mode == THEME_DARK || (mode == THEME_AUTO && system_uses_dark_apps());
    app.ui.accent = accent_colour();
    let hwnd = app.ui.hwnd;
    if hwnd.is_null() {
        return;
    }
    let dark: windows_sys::core::BOOL = app.ui.dark as i32;
    unsafe {
        if DwmSetWindowAttribute(hwnd, 20, (&dark as *const i32).cast(), 4) < 0 {
            DwmSetWindowAttribute(hwnd, 19, (&dark as *const i32).cast(), 4);
        }
        SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    invalidate(hwnd);
}

pub fn apply_on_top(app: &App) {
    let after = if app.state.ui.on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe { SetWindowPos(app.ui.hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
}

// ---------------------------------------------------------------- helpers

/// Windows' own file type name and small icon for an extension, as Explorer shows them.
fn shell_info<'a>(cache: &'a mut HashMap<String, (HICON, String)>, file: &str) -> &'a (HICON, String) {
    let ext = Path::new(file).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    cache.entry(ext.clone()).or_insert_with(|| unsafe {
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let name = wide(&format!("x.{ext}"));
        SHGetFileInfoW(
            name.as_ptr(),
            FILE_ATTRIBUTE_NORMAL,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES,
        );
        let len = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(0);
        let mut type_name = String::from_utf16_lossy(&info.szTypeName[..len]);
        if type_name.is_empty() {
            type_name = format!("{} archive", ext.to_uppercase());
        }
        (info.hIcon, type_name)
    })
}

/// "18.11.2026. 22:22" (or whatever the user's locale says), like Explorer's Date modified.
fn format_date(unix_secs: u64) -> String {
    unsafe {
        let ft_val = (unix_secs + 11_644_473_600) * 10_000_000;
        let ft = FILETIME { dwLowDateTime: ft_val as u32, dwHighDateTime: (ft_val >> 32) as u32 };
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        FileTimeToSystemTime(&ft, &mut utc);
        SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local);
        let mut d = [0u16; 64];
        let mut t = [0u16; 64];
        let dn = GetDateFormatEx(null(), DATE_SHORTDATE, &local, null(), d.as_mut_ptr(), 64, null());
        let tn = GetTimeFormatEx(null(), TIME_NOSECONDS, &local, null(), t.as_mut_ptr(), 64);
        let d = String::from_utf16_lossy(&d[..(dn.max(1) - 1) as usize]);
        let t = String::from_utf16_lossy(&t[..(tn.max(1) - 1) as usize]);
        format!("{d} {t}")
    }
}

/// "15 KB", formatted by Windows like Explorer's Size column.
fn format_size(bytes: u64) -> String {
    let mut buf = [0u16; 64];
    unsafe { StrFormatKBSizeW(bytes as i64, buf.as_mut_ptr(), 64) };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf16_lossy(&buf[..len])
}

fn expand_env(s: &str) -> String {
    let w = wide(s);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW(w.as_ptr(), buf.as_mut_ptr(), 1024) };
    if n == 0 || n > 1024 {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..(n - 1) as usize])
}

/// The file Windows shows for a role when a scheme leaves it empty.
fn default_cursor(role: usize) -> String {
    const FILES: [&str; 17] = [
        "aero_arrow.cur", "aero_helpsel.cur", "aero_working.ani", "aero_busy.ani", "cross_r.cur", "beam_r.cur",
        "aero_pen.cur", "aero_unavail.cur", "aero_ns.cur", "aero_ew.cur", "aero_nwse.cur", "aero_nesw.cur",
        "aero_move.cur", "aero_up.cur", "aero_link.cur", "aero_pin.cur", "aero_person.cur",
    ];
    expand_env(&format!(r"%SystemRoot%\Cursors\{}", FILES[role]))
}

/// What Windows uses for each role right now.
fn current_cursors() -> Vec<String> {
    // Screenshots for the README show Windows' own cursors as "Now", not whoever runs them.
    if std::env::var_os("CURSORDROP_SELFTEST_NOW_DEFAULT").is_some() {
        return (0..ROLES.len()).map(default_cursor).collect();
    }
    (0..ROLES.len())
        .map(|i| match cursors::get_string(cursors::CURSORS_KEY, Some(ROLES[i])) {
            Some(v) if !v.trim().is_empty() => expand_env(&v),
            _ => default_cursor(i),
        })
        .collect()
}

thread_local! {
    static SAME: std::cell::RefCell<HashMap<(String, String), bool>> = std::cell::RefCell::new(HashMap::new());
}

/// Whether two cursor files are the same picture: same path, or byte-identical files (e.g. the
/// scheme you use now and the copy inside a pack). Cached, since it's asked every frame.
fn same_file(a: &str, b: &str) -> bool {
    if a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\")) {
        return true;
    }
    SAME.with(|c| {
        *c.borrow_mut().entry((a.to_lowercase(), b.to_lowercase())).or_insert_with(|| {
            let (ma, mb) = (std::fs::metadata(a), std::fs::metadata(b));
            match (ma, mb) {
                (std::result::Result::Ok(ma), std::result::Result::Ok(mb)) if ma.len() == mb.len() => {
                    matches!((std::fs::read(a), std::fs::read(b)), (std::result::Result::Ok(x), std::result::Result::Ok(y)) if x == y)
                }
                _ => false,
            }
        })
    })
}

fn forget_file_comparisons() {
    SAME.with(|c| c.borrow_mut().clear());
}

// ---------------------------------------------------------------- a snapshot of what to draw

struct Pack {
    inst: Installed,
    dir: PathBuf,
}

impl Pack {
    fn path(&self, scheme: &Scheme, role: usize) -> Option<String> {
        scheme.files.get(role).cloned().flatten().map(|rel| self.dir.join(rel).to_string_lossy().to_string())
    }
}

struct Model {
    packs: Vec<Pack>,
    active: Option<Active>,
    startup: bool,
    can_restore: bool,
    close_to_tray: bool,
    theme: u8,
    on_top: bool,
    columns: [f32; 5],
    sort_column: usize,
    sort_ascending: bool,
}

impl Model {
    fn new(app: &App) -> Model {
        let current = app.ours_is_current();
        let s = &app.state.ui;
        let mut packs: Vec<Pack> = app
            .state
            .installed
            .iter()
            .map(|i| Pack { inst: i.clone(), dir: app.scheme_dir(&i.archive) })
            .collect();
        let key_type = |p: &Pack| Path::new(&p.inst.archive).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        packs.sort_by(|a, b| {
            let o = match s.sort_column {
                1 => a.inst.mtime.cmp(&b.inst.mtime),
                2 => key_type(a).cmp(&key_type(b)),
                3 => a.inst.size.cmp(&b.inst.size),
                4 => {
                    let n = |p: &Pack| p.inst.schemes.first().map_or(0, |s| s.mapped());
                    n(a).cmp(&n(b))
                }
                _ => std::cmp::Ordering::Equal,
            };
            let o = o.then_with(|| a.inst.archive.to_lowercase().cmp(&b.inst.archive.to_lowercase()));
            if s.sort_ascending { o } else { o.reverse() }
        });
        Model {
            packs,
            active: if current { app.state.active.clone() } else { None },
            startup: app.startup_enabled(),
            can_restore: current && app.state.backup.is_some(),
            close_to_tray: s.close_to_tray,
            theme: s.theme,
            on_top: s.on_top,
            columns: s.columns,
            sort_column: s.sort_column,
            sort_ascending: s.sort_ascending,
        }
    }

    fn pack(&self, archive: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.inst.archive == archive)
    }

    fn active_name(&self) -> Option<String> {
        let a = self.active.as_ref()?;
        Some(self.pack(&a.archive)?.inst.schemes.get(a.scheme)?.display().to_string())
    }
}

// ---------------------------------------------------------------- painter

struct Painter<'a> {
    g: Gfx,
    f: &'a Fonts,
    p: Palette,
    s: f32,
    accent: Color,
    dark: bool,
    elems: Vec<Elem>,
    hover: Option<Hit>,
    cursors: &'a mut CursorCache,
    shell: &'a mut HashMap<String, (HICON, String)>,
    ms: u64,
    animated: bool,
}

impl Painter<'_> {
    fn s(&self, v: f32) -> f32 {
        v * self.s
    }

    /// Registers a clickable element and says whether the mouse is over it.
    fn add(&mut self, kind: Kind, id: usize, r: R) -> bool {
        self.elems.push(Elem { kind, id, r });
        self.hover == Some((kind, id))
    }

    fn draw_cursor(&mut self, path: &str, cx: f32, cy: f32, size: f32, clip: Option<R>, animate: bool) {
        let px = size.round() as i32;
        let ms = self.ms;
        if let Some(img) = self.cursors.get(path, px) {
            let step = if animate { img.step(ms) } else { 0 };
            let h = img.h;
            if animate && img.animated() {
                self.animated = true;
            }
            self.g.cursor(h, cx - size / 2.0, cy - size / 2.0, size, step, clip);
        }
    }

    fn switch(&self, r: R, t: f32, on: Color) {
        let p = self.p;
        self.g.fill_round(r, r.h / 2.0, mix(p.switch_off, on, t));
        let d = r.h - self.s(4.0);
        let x = r.x + self.s(2.0) + t * (r.w - d - self.s(4.0));
        self.g.ellipse(x, r.y + self.s(3.0), d, d, argb(50, 0, 0, 0));
        self.g.ellipse(x, r.y + self.s(2.0), d, d, WHITE);
    }

    fn toggle_row(&mut self, r: R, id: usize, title: &str, detail: &str, t: f32) {
        let p = self.p;
        if self.add(Kind::Toggle, id, r) {
            self.g.fill_round(R::new(r.x + self.s(4.0), r.y + self.s(3.0), r.w - self.s(8.0), r.h - self.s(6.0)), self.s(8.0), p.hover);
        }
        let text_w = r.w - self.s(16.0 + 46.0 + 28.0);
        self.g.label(title, &self.f.body, p.text, R::new(r.x + self.s(16.0), r.y + self.s(9.0), text_w, self.s(20.0)));
        self.g.label(detail, &self.f.caption, p.subtext, R::new(r.x + self.s(16.0), r.y + self.s(30.0), text_w, self.s(18.0)));
        let sw = R::new(r.right() - self.s(16.0 + 46.0), r.y + (r.h - self.s(28.0)) / 2.0, self.s(46.0), self.s(28.0));
        self.switch(sw, t, self.accent);
    }

    fn button(&mut self, r: R, id: usize, text: &str, primary: bool, live: bool) {
        let p = self.p;
        let hovered = live && self.add(Kind::Button, id, r);
        let mut fill = if primary { self.accent } else { with_alpha(p.card, 255) };
        if hovered {
            fill = if primary { mix(self.accent, WHITE, 0.15) } else { p.hover };
        }
        if !live {
            fill = with_alpha(fill, if primary { 90 } else { 120 });
        }
        self.g.fill_round(r, self.s(8.0), fill);
        if !primary {
            self.g.stroke_round(r, self.s(8.0), p.line, self.s(1.0));
        }
        let c = if primary { on_accent(self.accent) } else { p.text };
        self.g.text(text, &self.f.caption, if live { c } else { with_alpha(c, 120) }, r, Align::Center, Align::Center);
    }

    fn icon_button(&mut self, r: R, kind: Kind, glyph: &str, label: Option<&str>, on: bool) {
        let p = self.p;
        let hovered = self.add(kind, 0, r);
        self.g.fill_round(r, self.s(10.0), if hovered || on { p.hover } else { with_alpha(p.card, 255) });
        self.g.stroke_round(r, self.s(10.0), p.line, self.s(1.0));
        let c = if on { self.accent } else { p.text };
        match label {
            None => self.g.text(glyph, &self.f.icon, c, r, Align::Center, Align::Center),
            Some(label) => {
                self.g.text(glyph, &self.f.icon, c, R::new(r.x + self.s(10.0), r.y, self.s(20.0), r.h), Align::Center, Align::Center);
                self.g.label(label, &self.f.caption, p.text, R::new(r.x + self.s(34.0), r.y, r.w - self.s(58.0), r.h));
                self.g.text(G_CHEVRON, &self.f.glyph, p.subtext, R::new(r.right() - self.s(26.0), r.y, self.s(16.0), r.h), Align::Center, Align::Center);
            }
        }
    }

    /// A small square arrow button at the end of the tab strip.
    fn tab_arrow(&mut self, r: R, kind: Kind, glyph: &str, live: bool) {
        let p = self.p;
        let hovered = live && self.add(kind, 0, r);
        self.g.fill_round(r, self.s(9.0), if hovered { p.segment_on } else { p.segment });
        let c = if live { p.text } else { with_alpha(p.text, 70) };
        self.g.text(glyph, &self.f.glyph, c, r, Align::Center, Align::Center);
    }

    fn section_label(&self, x: f32, y: f32, w: f32, text: &str) {
        self.g.label(text, &self.f.section, self.p.subtext, R::new(x + self.s(4.0), y, w, self.s(20.0)));
    }

    fn scrollbar(&mut self, area: R, scroll: f32, max: f32) {
        if max <= 0.5 {
            return;
        }
        let track_h = area.h - self.s(8.0);
        let th = (track_h * area.h / (area.h + max)).max(self.s(36.0));
        let y = area.y + self.s(4.0) + (track_h - th) * (scroll / max);
        let hit = R::new(area.right() - self.s(14.0), y, self.s(14.0), th);
        let hovered = self.add(Kind::Thumb, 0, hit);
        let w = if hovered { self.s(6.0) } else { self.s(4.0) };
        let c = with_alpha(self.p.subtext, if hovered { 170 } else { 100 });
        self.g.fill_round(R::new(area.right() - self.s(5.0) - w, y, w, th), w / 2.0, c);
    }

    fn menu(&mut self, anchor: R, width: f32, names: &[&str], glyphs: &[&str], selected: usize) {
        let p = self.p;
        let row_h = self.s(34.0);
        let h = row_h * names.len() as f32 + self.s(8.0);
        let menu = R::new(anchor.right() - width, anchor.bottom() + self.s(6.0), width, h);
        self.g.fill_round(R::new(menu.x, menu.y + self.s(3.0), menu.w, menu.h), self.s(12.0), p.shadow);
        self.g.fill_round(menu, self.s(12.0), with_alpha(p.card, 255));
        self.g.stroke_round(menu, self.s(12.0), p.line, self.s(1.0));
        // The menu swallows clicks that miss its rows (registered first, so the rows win).
        self.elems.push(Elem { kind: Kind::Theme, id: 1, r: menu });
        for (i, name) in names.iter().enumerate() {
            let row = R::new(menu.x + self.s(4.0), menu.y + self.s(4.0) + row_h * i as f32, menu.w - self.s(8.0), row_h);
            if self.add(Kind::ThemeItem, i, row) {
                self.g.fill_round(row, self.s(8.0), p.hover);
            }
            self.g.text(glyphs[i], &self.f.icon, p.text, R::new(row.x + self.s(8.0), row.y, self.s(22.0), row.h), Align::Center, Align::Center);
            self.g.label(name, &self.f.body, p.text, R::new(row.x + self.s(38.0), row.y, row.w - self.s(72.0), row.h));
            if i == selected {
                self.g.text(G_CHECK, &self.f.icon, self.accent, R::new(row.right() - self.s(30.0), row.y, self.s(22.0), row.h), Align::Center, Align::Center);
            }
        }
    }
}

impl CursorImg {
    fn animated(&self) -> bool {
        self.step(0) != self.step(1000) || self.step(0) != self.step(250)
    }
}

// ---------------------------------------------------------------- painting

pub fn paint_to(app: &mut App, w: i32, h: i32) -> HDC {
    if app.ui.canvas.as_ref().is_none_or(|c| c.w != w || c.h != h) {
        app.ui.canvas = Some(Canvas::new(w, h));
    }
    let dc = app.ui.canvas.as_ref().unwrap().dc;
    paint(app, dc, w as f32, h as f32);
    dc
}

fn paint(app: &mut App, dc: HDC, w: f32, h: f32) {
    let model = Model::new(app);
    if app.ui.fonts.is_none() {
        app.ui.fonts = Some(Fonts::new(app.ui.scale));
    }
    let fonts = app.ui.fonts.take().unwrap();
    let mut cursors = std::mem::take(&mut app.ui.cursors);
    let mut shell = std::mem::take(&mut app.ui.shell);
    let ui = &mut app.ui;

    // Switch knobs ease towards their state; the drop overlay fades in and out.
    for (knob, on) in ui.knobs.iter_mut().zip([model.startup, model.close_to_tray]) {
        let goal = if on { 1.0 } else { 0.0 };
        if *knob < 0.0 {
            *knob = goal;
        }
        *knob += (goal - *knob) * 0.3;
        if (goal - *knob).abs() < 0.01 {
            *knob = goal;
        }
    }
    let drag_goal = if ui.drag.is_some() { 1.0 } else { 0.0 };
    ui.drag_t += (drag_goal - ui.drag_t) * 0.28;
    if (drag_goal - ui.drag_t).abs() < 0.01 {
        ui.drag_t = drag_goal;
    }
    if ui.drag.is_some() {
        ui.drag_shown = ui.drag.clone();
    }

    let mut pt = Painter {
        g: Gfx::from_hdc(dc),
        f: &fonts,
        p: if ui.dark { DARK } else { LIGHT },
        s: ui.scale,
        accent: ui.accent,
        dark: ui.dark,
        elems: Vec::new(),
        hover: ui.hover,
        cursors: &mut cursors,
        shell: &mut shell,
        ms: ui.t0.elapsed().as_millis() as u64,
        animated: false,
    };

    // A detail view whose pack disappeared goes back to the list.
    if let View::Detail { archive, .. } = &ui.view {
        if model.pack(archive).is_none_or(|p| p.inst.schemes.is_empty()) {
            ui.view = View::List;
            ui.scroll = 0.0;
        }
    }

    let p = pt.p;
    pt.g.clear(p.bg);
    // The soft ambient light DS4-Native has behind its controller, in the accent colour.
    pt.g.glow(w * 0.5, h * 0.42, w * 0.62, h * 0.55, with_alpha(ui.accent, if ui.dark { 46 } else { 34 }));

    let m = pt.s(MARGIN);
    let content = R::new(m, pt.s(88.0), w - 2.0 * m, h - pt.s(88.0) - pt.s(FOOTER_H) - pt.s(12.0));
    let theme_btn = R::new(w - m - pt.s(124.0), pt.s(20.0), pt.s(124.0), pt.s(36.0));

    match ui.view.clone() {
        View::List => {
            draw_list_header(&mut pt, &model, m);
            draw_list(&mut pt, ui, &model, content);
            draw_list_footer(&mut pt, &model, ui.knobs, w, h);
        }
        View::Detail { archive, variant } => {
            let pack = model.pack(&archive).unwrap();
            let variant = variant.min(pack.inst.schemes.len() - 1);
            draw_detail(&mut pt, ui, &model, pack, variant, content, m);
            draw_detail_footer(&mut pt, &model, pack, variant, ui.knobs, w, h);
        }
    }

    // On-top pin and theme, top right, as in DS4-Native.
    let pin = R::new(theme_btn.x - pt.s(46.0), theme_btn.y, pt.s(36.0), pt.s(36.0));
    pt.icon_button(pin, Kind::Pin, if model.on_top { G_PINNED } else { G_PIN }, None, model.on_top);
    let theme_glyph = match model.theme {
        THEME_LIGHT => G_SUN,
        THEME_DARK => G_MOON,
        _ if pt.dark => G_MOON,
        _ => G_SUN,
    };
    let theme_label = if model.theme == THEME_AUTO { "Auto" } else { THEME_NAMES[model.theme as usize] };
    pt.icon_button(theme_btn, Kind::Theme, theme_glyph, Some(theme_label), ui.theme_open);
    if ui.theme_open {
        let auto_glyph = if pt.dark { G_MOON } else { G_SUN };
        pt.menu(theme_btn, pt.s(170.0), &THEME_NAMES, &[auto_glyph, G_SUN, G_MOON], model.theme as usize);
    }
    if ui.drag_t > 0.0 {
        if let Some(info) = &ui.drag_shown {
            draw_drop_overlay(&mut pt, info, ui.drag_t, w, h);
        }
    }

    ui.animated = pt.animated;
    ui.elems = std::mem::take(&mut pt.elems);
    // Things can move under a still mouse (the tabs ease along): keep the hover honest.
    if let Some((x, y)) = ui.mouse {
        if ui.col_drag.is_none() && ui.thumb_drag.is_none() {
            let hit = hit_test(ui, x, y);
            if hit != ui.hover {
                ui.hover = hit;
                invalidate(ui.hwnd);
            }
        }
    }
    drop(pt);
    app.ui.fonts = Some(fonts);
    app.ui.cursors = cursors;
    app.ui.shell = shell;
}

/// The exe's own icon (resource 1) at a pixel size, loaded once per size.
fn app_icon(px: i32) -> HICON {
    thread_local! {
        static ICONS: std::cell::RefCell<HashMap<i32, usize>> = std::cell::RefCell::new(HashMap::new());
    }
    ICONS.with(|c| {
        *c.borrow_mut().entry(px).or_insert_with(|| unsafe {
            let hinst = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null());
            LoadImageW(hinst, 1 as *const u16, IMAGE_ICON, px, px, 0) as usize
        }) as HICON
    })
}

fn draw_list_header(pt: &mut Painter, model: &Model, m: f32) {
    let p = pt.p;
    let y = pt.s(20.0);
    let icon_px = pt.s(24.0).round();
    let icon = app_icon(icon_px as i32);
    if icon.is_null() {
        pt.g.text(G_MOUSE, &pt.f.icon, pt.accent, R::new(m, y, pt.s(22.0), pt.s(28.0)), Align::Center, Align::Center);
    } else {
        pt.g.cursor(icon, m - pt.s(1.0), y + (pt.s(28.0) - icon_px) / 2.0, icon_px, 0, None);
    }
    pt.g.label("CursorDrop", &pt.f.title, p.text, R::new(m + pt.s(28.0), y, pt.s(400.0), pt.s(28.0)));
    let packs = model.packs.iter().filter(|p| p.inst.error.is_none()).count();
    let status = match (packs, model.active_name()) {
        (0, _) => "Drop .zip, .7z or .rar cursor packs here or into the cursors folder".to_string(),
        (n, Some(a)) => format!("{n} cursor pack{}   \u{00B7}   Using {a}", if n == 1 { "" } else { "s" }),
        (n, None) => format!("{n} cursor pack{}   \u{00B7}   Click one to see its cursors", if n == 1 { "" } else { "s" }),
    };
    pt.g.label(&status, &pt.f.caption, p.subtext, R::new(m, y + pt.s(30.0), pt.s(560.0), pt.s(20.0)));
}

fn column_x(pt: &Painter, model: &Model, card: R) -> [f32; 6] {
    let mut xs = [0.0; 6];
    xs[0] = card.x + pt.s(14.0);
    for i in 0..5 {
        xs[i + 1] = xs[i] + pt.s(model.columns[i]);
    }
    xs
}

fn draw_list(pt: &mut Painter, ui: &mut Ui, model: &Model, card: R) {
    let p = pt.p;
    pt.g.fill_round(card, pt.s(12.0), p.card);
    let xs = column_x(pt, model, card);

    // Column headers, like Explorer's: click to sort, drag the dividers to resize.
    let hr = R::new(card.x, card.y + pt.s(4.0), card.w, pt.s(HEADER_ROW));
    for i in 0..5 {
        let cell = R::new(xs[i] - pt.s(8.0), hr.y, (xs[i + 1] - xs[i]).min(card.right() - xs[i] - pt.s(4.0)) + pt.s(4.0), hr.h);
        if cell.w <= 0.0 {
            continue;
        }
        if pt.add(Kind::ColHeader, i, cell) {
            pt.g.fill_round(cell.inset(pt.s(1.0), pt.s(3.0)), pt.s(6.0), p.hover);
        }
        let text_r = R::new(xs[i], hr.y, cell.w - pt.s(28.0), hr.h);
        pt.g.label(COLUMN_NAMES[i], &pt.f.caption, p.subtext, text_r);
        if model.sort_column == i {
            let tw = pt.g.measure(COLUMN_NAMES[i], &pt.f.caption).min(text_r.w);
            let glyph = if model.sort_ascending { G_CHEVRON_UP } else { G_CHEVRON };
            pt.g.text(glyph, &pt.f.glyph, p.subtext, R::new(xs[i] + tw + pt.s(4.0), hr.y, pt.s(14.0), hr.h), Align::Center, Align::Center);
        }
        let div = xs[i + 1] - pt.s(4.0);
        if div < card.right() - pt.s(8.0) {
            pt.g.line(div, hr.y + pt.s(9.0), div, hr.bottom() - pt.s(9.0), p.line, pt.s(1.0));
        }
    }
    // Dividers last, so they win over the header cells they overlap.
    for i in 0..5 {
        let div = xs[i + 1] - pt.s(4.0);
        if div < card.right() - pt.s(8.0) {
            pt.add(Kind::ColSplit, i, R::new(div - pt.s(4.0), hr.y, pt.s(8.0), hr.h));
        }
    }
    pt.g.line(card.x + pt.s(10.0), hr.bottom(), card.right() - pt.s(10.0), hr.bottom(), p.line, pt.s(1.0));

    let area = R::new(card.x, hr.bottom() + pt.s(1.0), card.w, card.bottom() - hr.bottom() - pt.s(6.0));
    ui.scroll_area = area;
    ui.row_order = model.packs.iter().map(|p| p.inst.archive.clone()).collect();

    if model.packs.is_empty() {
        let hint = R::new(area.x + area.w / 2.0 - pt.s(220.0), area.y + area.h / 2.0 - pt.s(80.0), pt.s(440.0), pt.s(160.0));
        pt.g.dashed_round(hint, pt.s(14.0), with_alpha(p.subtext, 120), pt.s(1.5), 0.0);
        pt.g.text(G_DROP, &pt.f.title, pt.accent, R::new(hint.x, hint.y + pt.s(26.0), hint.w, pt.s(32.0)), Align::Center, Align::Center);
        pt.g.text("Drop cursor packs here", &pt.f.body, p.text, R::new(hint.x, hint.y + pt.s(70.0), hint.w, pt.s(22.0)), Align::Center, Align::Center);
        pt.g.text(
            ".zip, .7z or .rar \u{00B7} they're applied straight away",
            &pt.f.caption,
            p.subtext,
            R::new(hint.x, hint.y + pt.s(96.0), hint.w, pt.s(20.0)),
            Align::Center,
            Align::Center,
        );
        ui.scroll_max = 0.0;
        return;
    }

    let total = pt.s(6.0) + pt.s(ROW_H) * model.packs.len() as f32;
    ui.scroll_max = (total - area.h).max(0.0);
    ui.scroll = ui.scroll.clamp(0.0, ui.scroll_max);
    pt.g.clip(area);
    let mut y = area.y + pt.s(4.0) - ui.scroll;
    for (i, pack) in model.packs.iter().enumerate() {
        let row = R::new(card.x + pt.s(6.0), y, card.w - pt.s(12.0), pt.s(ROW_H));
        y += pt.s(ROW_H);
        if !row.intersects(&area) {
            continue;
        }
        let visible = R::new(row.x, row.y.max(area.y), row.w, row.bottom().min(area.bottom()) - row.y.max(area.y));
        let inst = &pack.inst;
        let ok = inst.error.is_none();
        let is_active = model.active.as_ref().is_some_and(|a| a.archive == inst.archive);
        if pt.add(Kind::Row, i, visible) {
            pt.g.fill_round(row, pt.s(8.0), p.hover);
        }
        if is_active {
            pt.g.fill_round(R::new(row.x + pt.s(1.0), row.y + pt.s(11.0), pt.s(3.0), row.h - pt.s(22.0)), pt.s(1.5), pt.accent);
        }
        let text = if ok { p.text } else { with_alpha(p.text, 130) };
        // Name: the file type's icon and the file name, with an "In use" chip.
        let (icon, type_name) = shell_info(pt.shell, &inst.archive).clone();
        let pad = pt.s(14.0);
        let cell = |c: usize| R::new(xs[c], row.y, xs[c + 1] - xs[c] - pad, row.h);
        let icon_size = pt.s(16.0).round();
        pt.g.cursor(icon, xs[0], row.y + (row.h - icon_size) / 2.0, icon_size, 0, Some(area));
        let name_r = R::new(xs[0] + pt.s(26.0), row.y, cell(0).w - pt.s(26.0), row.h);
        let chip_w = if is_active { pt.s(52.0) } else { 0.0 };
        let name_w = pt.g.measure(&inst.archive, &pt.f.body).min(name_r.w - chip_w);
        pt.g.label(&inst.archive, &pt.f.body, text, R::new(name_r.x, row.y, name_w + pt.s(2.0), row.h));
        if is_active {
            let chip = R::new(name_r.x + name_w + pt.s(8.0), row.y + (row.h - pt.s(18.0)) / 2.0, pt.s(46.0), pt.s(18.0));
            pt.g.fill_round(chip, pt.s(9.0), pt.accent);
            pt.g.text("In use", &pt.f.tiny, on_accent(pt.accent), chip, Align::Center, Align::Center);
        }
        pt.g.label(&format_date(inst.mtime), &pt.f.caption, p.subtext, cell(1));
        pt.g.label(&type_name, &pt.f.caption, p.subtext, cell(2));
        pt.g.label(&format_size(inst.size), &pt.f.caption, p.subtext, cell(3));

        // Cursors: a few of the pack's cursors and how many roles it covers.
        let c = cell(4);
        match inst.schemes.first() {
            Some(s) if ok => {
                let mut x = c.x;
                let size = pt.s(24.0);
                for role in [0usize, 3, 14, 5] {
                    if x + size > c.right() - pt.s(40.0) {
                        break;
                    }
                    if let Some(path) = pack.path(s, role) {
                        pt.draw_cursor(&path, x + size / 2.0, row.y + row.h / 2.0, size, Some(area), false);
                        x += size + pt.s(6.0);
                    }
                }
                let n = if inst.schemes.len() > 1 { format!("{} variants", inst.schemes.len()) } else { format!("{}/{}", s.mapped(), ROLES.len()) };
                pt.g.label(&n, &pt.f.caption, p.subtext, R::new(x + pt.s(4.0), row.y, (c.right() - x).max(0.0), row.h));
            }
            _ => {
                let e = inst.error.clone().unwrap_or_default();
                let e = if e.is_empty() || e.contains("no recognizable") { "No cursors found".to_string() } else { e };
                pt.g.text(G_WARNING, &pt.f.glyph, WARNING, R::new(c.x, row.y, pt.s(14.0), row.h), Align::Center, Align::Center);
                pt.g.label(&e, &pt.f.caption, WARNING, R::new(c.x + pt.s(20.0), row.y, c.w - pt.s(20.0), row.h));
            }
        }
    }
    pt.g.unclip();
    pt.scrollbar(area, ui.scroll, ui.scroll_max);
}

fn footer_card(pt: &mut Painter, knobs: [f32; 2], close_to_tray: bool, w: f32, h: f32) -> R {
    let footer = R::new(pt.s(MARGIN), h - pt.s(FOOTER_H), w - 2.0 * pt.s(MARGIN), pt.s(FOOTER_H) - pt.s(16.0));
    pt.g.fill_round(footer, pt.s(12.0), pt.p.card);
    let tw = pt.s(TOGGLE_W);
    pt.toggle_row(R::new(footer.x, footer.y, tw, footer.h), 0, "Start with Windows", "Waits in the tray", knobs[0]);
    let detail = if close_to_tray { "Keeps running in the tray" } else { "Closing the window quits" };
    pt.toggle_row(R::new(footer.x + tw, footer.y, tw, footer.h), 1, "Close to tray", detail, knobs[1]);
    footer
}

/// Buttons right-aligned in the footer, last one rightmost.
fn footer_buttons(pt: &mut Painter, footer: R, buttons: &[(usize, &str, bool, bool)]) {
    let mut x = footer.right() - pt.s(12.0);
    let y = footer.y + (footer.h - pt.s(32.0)) / 2.0;
    for &(id, text, primary, live) in buttons.iter().rev() {
        let bw = (pt.g.measure(text, &pt.f.caption) + pt.s(40.0)).max(pt.s(118.0));
        x -= bw;
        if x < footer.x + pt.s(2.0 * TOGGLE_W + 10.0) {
            break;
        }
        pt.button(R::new(x, y, bw, pt.s(32.0)), id, text, primary, live);
        x -= pt.s(10.0);
    }
}

fn draw_list_footer(pt: &mut Painter, model: &Model, knobs: [f32; 2], w: f32, h: f32) {
    let footer = footer_card(pt, knobs, model.close_to_tray, w, h);
    footer_buttons(
        pt,
        footer,
        &[
            (BTN_RESTORE, "Restore original cursors", false, model.can_restore),
            (BTN_FOLDER, "Open cursors folder", false, true),
            (BTN_ADD, "Add pack\u{2026}", true, true),
        ],
    );
}

fn draw_detail_footer(pt: &mut Painter, model: &Model, pack: &Pack, variant: usize, knobs: [f32; 2], w: f32, h: f32) {
    let footer = footer_card(pt, knobs, model.close_to_tray, w, h);
    let in_use = model.active.as_ref() == Some(&Active { archive: pack.inst.archive.clone(), scheme: variant });
    footer_buttons(
        pt,
        footer,
        &[
            (BTN_REMOVE, "Remove pack", false, true),
            (BTN_SHOW, "Show in folder", false, true),
            (BTN_APPLY, if in_use { "In use" } else { "Use these cursors" }, true, !in_use),
        ],
    );
}

/// Browser-style tabs for a pack's variants. They share the row up to TAB_MAX each; below
/// TAB_MIN they stop shrinking and the strip scrolls instead, with arrows at both ends. A
/// hovered tab whose name doesn't fit grows over its neighbours to show all of it.
fn draw_tabs(pt: &mut Painter, t: &mut TabStrip, r: R, labels: &[String], selected: usize) {
    let p = pt.p;
    let n = labels.len() as f32;
    let fits = n * pt.s(TAB_MIN) <= r.w;
    let (view, tab_w) = if fits {
        let tw = (r.w / n).min(pt.s(TAB_MAX));
        (R::new(r.x, r.y, tw * n, r.h), tw)
    } else {
        let a = pt.s(TAB_ARROW + 6.0);
        (R::new(r.x + a, r.y, r.w - 2.0 * a, r.h), pt.s(TAB_MIN))
    };
    t.area = if fits { view } else { r };
    t.tab_w = tab_w;
    t.max = (tab_w * n - view.w).max(0.0);

    // Keep the selected tab in view; ease the scroll towards its target.
    if t.reveal {
        let (x0, x1) = (selected as f32 * tab_w, (selected + 1) as f32 * tab_w);
        if x0 < t.target {
            t.target = x0;
        } else if x1 > t.target + view.w {
            t.target = x1 - view.w;
        }
        t.reveal = false;
    }
    t.target = t.target.clamp(0.0, t.max);
    if t.snap {
        t.scroll = t.target;
        t.snap = false;
    } else {
        t.scroll += (t.target - t.scroll) * 0.35;
        if (t.target - t.scroll).abs() < 0.5 {
            t.scroll = t.target;
        }
    }
    t.scroll = t.scroll.clamp(0.0, t.max);

    if !fits {
        let left = R::new(r.x, r.y, pt.s(TAB_ARROW), r.h);
        let right = R::new(r.right() - pt.s(TAB_ARROW), r.y, pt.s(TAB_ARROW), r.h);
        pt.tab_arrow(left, Kind::TabPrev, G_LEFT, t.target > 0.5);
        pt.tab_arrow(right, Kind::TabNext, G_RIGHT, t.target < t.max - 0.5);
    }
    pt.g.fill_round(view, pt.s(9.0), p.segment);
    pt.g.clip(view);
    let mut grow: Option<(usize, R)> = None;
    for (i, label) in labels.iter().enumerate() {
        let seg = R::new(view.x + i as f32 * tab_w - t.scroll, view.y, tab_w, view.h);
        if seg.right() <= view.x || seg.x >= view.right() {
            continue;
        }
        let vx = seg.x.max(view.x);
        let visible = R::new(vx, seg.y, seg.right().min(view.right()) - vx, seg.h);
        let hovered = pt.add(Kind::Segment, i, visible);
        let pill = seg.inset(pt.s(2.0), pt.s(2.0));
        if i == selected {
            pt.g.fill_round(R::new(pill.x, pill.y + pt.s(1.0), pill.w, pill.h), pt.s(7.0), p.shadow);
            pt.g.fill_round(pill, pt.s(7.0), p.segment_on);
        } else if hovered {
            pt.g.fill_round(pill, pt.s(7.0), with_alpha(p.segment_on, 110));
        } else if i + 1 < labels.len() && i + 1 != selected {
            // A thin divider between neighbouring unselected tabs, like a browser's.
            let x = seg.right();
            pt.g.line(x, seg.y + pt.s(10.0), x, seg.bottom() - pt.s(10.0), with_alpha(p.subtext, 60), pt.s(1.0));
        }
        let font = if i == selected { &pt.f.section } else { &pt.f.caption };
        let text_r = seg.inset(pt.s(12.0), 0.0);
        pt.g.text(label, font, p.text, text_r, Align::Center, Align::Center);
        if hovered && pt.g.measure(label, font) > text_r.w {
            grow = Some((i, seg));
        }
    }
    if let Some((i, seg)) = grow {
        let font = if i == selected { &pt.f.section } else { &pt.f.caption };
        let w = (pt.g.measure(&labels[i], font) + pt.s(28.0)).min(view.w);
        let x = (seg.x + seg.w / 2.0 - w / 2.0).clamp(view.x, view.right() - w);
        let pill = R::new(x, seg.y + pt.s(2.0), w, seg.h - pt.s(4.0));
        pt.g.fill_round(R::new(pill.x, pill.y + pt.s(2.0), pill.w, pill.h), pt.s(7.0), p.shadow);
        let fill = if i == selected { p.segment_on } else { mix(p.segment, p.segment_on, 0.45) };
        pt.g.fill_round(pill, pt.s(7.0), with_alpha(fill, 255));
        pt.g.stroke_round(pill, pt.s(7.0), p.line, pt.s(1.0));
        pt.g.text(&labels[i], font, p.text, pill.inset(pt.s(10.0), 0.0), Align::Center, Align::Center);
    }
    pt.g.unclip();
}

fn draw_detail(pt: &mut Painter, ui: &mut Ui, model: &Model, pack: &Pack, variant: usize, content: R, m: f32) {
    let p = pt.p;
    let scheme = &pack.inst.schemes[variant];
    let in_use = model.active.as_ref() == Some(&Active { archive: pack.inst.archive.clone(), scheme: variant });

    // Header: back button, the scheme's name and what's in it.
    let y = pt.s(20.0);
    pt.icon_button(R::new(m, y, pt.s(36.0), pt.s(36.0)), Kind::Back, G_BACK, None, false);
    let tx = m + pt.s(50.0);
    pt.g.label(scheme.display(), &pt.f.title, p.text, R::new(tx, y - pt.s(2.0), pt.s(520.0), pt.s(28.0)));
    let status = format!(
        "{}   \u{00B7}   {} of {} cursors{}",
        pack.inst.archive,
        scheme.mapped(),
        ROLES.len(),
        if in_use { "   \u{00B7}   In use" } else { "" }
    );
    pt.g.label(&status, &pt.f.caption, p.subtext, R::new(tx, y + pt.s(26.0), pt.s(560.0), pt.s(20.0)));

    let mut top = content.y;
    if pack.inst.schemes.len() > 1 {
        let labels: Vec<String> = pack.inst.schemes.iter().map(|s| s.display().to_string()).collect();
        draw_tabs(pt, &mut ui.tabs, R::new(content.x, top, content.w, pt.s(TAB_H)), &labels, variant);
        top += pt.s(TAB_H + 12.0);
    } else {
        ui.tabs.area = R::default();
        ui.tabs.max = 0.0;
    }
    let current = current_cursors();
    let changes = (0..ROLES.len())
        .filter(|&r| !same_file(&pack.path(scheme, r).unwrap_or_else(|| default_cursor(r)), &current[r]))
        .count();
    let label = if in_use {
        "YOUR CURSORS".to_string()
    } else if changes == 0 {
        "SAME AS THE CURSORS YOU USE NOW".to_string()
    } else {
        format!("WHAT CHANGES  \u{00B7}  {changes} OF {}", ROLES.len())
    };
    pt.section_label(content.x, top, pt.s(400.0), &label);
    if !in_use && changes > 0 {
        pt.g.text(
            "Now   \u{2192}   with this pack",
            &pt.f.caption,
            p.subtext,
            R::new(content.right() - pt.s(300.0), top, pt.s(296.0), pt.s(20.0)),
            Align::Far,
            Align::Center,
        );
    }
    top += pt.s(26.0);

    let area = R::new(content.x - pt.s(4.0), top, content.w + pt.s(8.0), content.bottom() - top);
    ui.scroll_area = area;
    let gap = pt.s(TILE_GAP);
    let cols = (((content.w + gap) / (pt.s(TILE_MIN_W) + gap)).floor() as usize).max(1);
    let tile_w = (content.w - gap * (cols - 1) as f32) / cols as f32;
    let tile_h = pt.s(TILE_H);
    let rows = ROLES.len().div_ceil(cols);
    let total = rows as f32 * (tile_h + gap) - gap + pt.s(4.0);
    ui.scroll_max = (total - area.h).max(0.0);
    ui.scroll = ui.scroll.clamp(0.0, ui.scroll_max);

    pt.g.clip(area);
    for role in 0..ROLES.len() {
        let (r_i, c_i) = (role / cols, role % cols);
        let r = R::new(content.x + c_i as f32 * (tile_w + gap), top + pt.s(2.0) + r_i as f32 * (tile_h + gap) - ui.scroll, tile_w, tile_h);
        if !r.intersects(&area) {
            continue;
        }
        let visible = R::new(r.x, r.y.max(area.y), r.w, r.bottom().min(area.bottom()) - r.y.max(area.y));
        let hovered = pt.add(Kind::Tile, role, visible);
        pt.g.fill_round(r, pt.s(12.0), if hovered { mix(p.card, p.hover, 0.6) } else { p.card });

        let well = R::new(r.x + pt.s(8.0), r.y + pt.s(8.0), r.w - pt.s(16.0), pt.s(92.0));
        pt.g.fill_round(well, pt.s(9.0), with_alpha(p.segment, if pt.dark { 120 } else { 150 }));
        let new_path = pack.path(scheme, role);
        let shown = new_path.clone().unwrap_or_else(|| default_cursor(role));
        let cy = well.y + well.h / 2.0 - pt.s(6.0);
        let clip = Some(area);
        if same_file(&shown, &current[role]) {
            pt.draw_cursor(&shown, well.x + well.w / 2.0, cy, pt.s(48.0), clip, true);
            pt.g.text("Unchanged", &pt.f.tiny, p.subtext, R::new(well.x, well.bottom() - pt.s(20.0), well.w, pt.s(16.0)), Align::Center, Align::Center);
        } else {
            let (x1, x2) = (well.x + well.w * 0.27, well.x + well.w * 0.71);
            pt.draw_cursor(&current[role], x1, cy, pt.s(32.0), clip, true);
            pt.draw_cursor(&shown, x2, cy, pt.s(48.0), clip, true);
            pt.g.text(G_ARROW, &pt.f.glyph, p.subtext, R::new(well.x + well.w * 0.49 - pt.s(8.0), cy - pt.s(8.0), pt.s(16.0), pt.s(16.0)), Align::Center, Align::Center);
            let lab = |x: f32| R::new(x - pt.s(40.0), well.bottom() - pt.s(20.0), pt.s(80.0), pt.s(16.0));
            pt.g.text("Now", &pt.f.tiny, p.subtext, lab(x1), Align::Center, Align::Center);
            pt.g.text("New", &pt.f.tiny, pt.accent, lab(x2), Align::Center, Align::Center);
        }

        pt.g.label(ROLE_LABELS[role], &pt.f.body, p.text, R::new(r.x + pt.s(14.0), r.y + pt.s(106.0), r.w - pt.s(28.0), pt.s(20.0)));
        let file = match &scheme.files[role] {
            Some(rel) => Path::new(rel).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            None => "Windows default".to_string(),
        };
        pt.g.label(&file, &pt.f.caption, p.subtext, R::new(r.x + pt.s(14.0), r.y + pt.s(128.0), r.w - pt.s(28.0), pt.s(18.0)));
    }
    pt.g.unclip();
    pt.scrollbar(area, ui.scroll, ui.scroll_max);
}

// ---------------------------------------------------------------- input

fn hit_test(ui: &Ui, x: f32, y: f32) -> Option<Hit> {
    ui.elems.iter().rev().find(|e| e.r.contains(x, y)).map(|e| (e.kind, e.id))
}

fn mouse_pos(lp: LPARAM) -> (f32, f32) {
    ((lp & 0xFFFF) as i16 as f32, ((lp >> 16) & 0xFFFF) as i16 as f32)
}

pub fn on_mouse_move(app: &mut App, lp: LPARAM) {
    let (x, y) = mouse_pos(lp);
    let ui = &mut app.ui;
    ui.mouse = Some((x, y));
    if !ui.tracking {
        let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: ui.hwnd, dwHoverTime: 0 };
        unsafe { TrackMouseEvent(&mut tme) };
        ui.tracking = true;
    }
    if let Some((col, start_x, start_w)) = ui.col_drag {
        let w = (start_w + (x - start_x) / ui.scale).max(MIN_COLUMN);
        app.state.ui.columns[col] = w.round();
        invalidate(app.ui.hwnd);
        return;
    }
    if let Some((start_y, start_scroll)) = ui.thumb_drag {
        let area = ui.scroll_area;
        let track = area.h - 8.0 * ui.scale;
        let th = (track * area.h / (area.h + ui.scroll_max)).max(36.0 * ui.scale);
        let per_px = if track - th > 1.0 { ui.scroll_max / (track - th) } else { 0.0 };
        ui.scroll = (start_scroll + (y - start_y) * per_px).clamp(0.0, ui.scroll_max);
        invalidate(ui.hwnd);
        return;
    }
    let hit = hit_test(ui, x, y);
    if hit != ui.hover {
        ui.hover = hit;
        invalidate(ui.hwnd);
    }
}

pub fn on_mouse_leave(app: &mut App) {
    app.ui.tracking = false;
    app.ui.mouse = None;
    if app.ui.hover.is_some() && app.ui.col_drag.is_none() {
        app.ui.hover = None;
        invalidate(app.ui.hwnd);
    }
}

/// WM_SETCURSOR: a sizing arrow over the column dividers. Returns true if handled.
pub fn on_set_cursor(app: &App) -> bool {
    let over_split = app.ui.col_drag.is_some() || matches!(app.ui.hover, Some((Kind::ColSplit, _)));
    if over_split {
        unsafe { SetCursor(LoadCursorW(null_mut(), IDC_SIZEWE)) };
    }
    over_split
}

pub fn on_button_down(app: &mut App, lp: LPARAM) {
    let (x, y) = mouse_pos(lp);
    let hit = hit_test(&app.ui, x, y);
    unsafe { SetCapture(app.ui.hwnd) };
    app.ui.pressed = hit;
    match hit {
        Some((Kind::ColSplit, col)) => app.ui.col_drag = Some((col, x, app.state.ui.columns[col])),
        Some((Kind::Thumb, _)) => app.ui.thumb_drag = Some((y, app.ui.scroll)),
        _ => {}
    }
    // A click anywhere but the theme menu closes it.
    if app.ui.theme_open && !matches!(hit, Some((Kind::Theme, _)) | Some((Kind::ThemeItem, _))) {
        app.ui.theme_open = false;
        app.ui.pressed = None;
        invalidate(app.ui.hwnd);
    }
}

pub fn on_button_up(app: &mut App, lp: LPARAM) {
    let (x, y) = mouse_pos(lp);
    unsafe { ReleaseCapture() };
    if app.ui.col_drag.take().is_some() {
        app.save();
        return;
    }
    if app.ui.thumb_drag.take().is_some() {
        return;
    }
    let hit = hit_test(&app.ui, x, y);
    let pressed = app.ui.pressed.take();
    if let Some(h) = hit {
        if pressed == Some(h) {
            action(app, h);
        }
    }
    invalidate(app.ui.hwnd);
}

pub fn on_right_click(app: &mut App, lp: LPARAM) {
    let (x, y) = mouse_pos(lp);
    if let Some((Kind::Row, i)) = hit_test(&app.ui, x, y) {
        if let Some(archive) = app.ui.row_order.get(i).cloned() {
            row_menu(app, &archive);
        }
    }
}

/// WM_MOUSEWHEEL / WM_MOUSEHWHEEL. Over the variant tabs either wheel scrolls them sideways
/// (wheel down / tilt right = forwards); elsewhere the vertical wheel scrolls the list or grid.
pub fn on_wheel(app: &mut App, wp: WPARAM, lp: LPARAM, horizontal: bool) {
    let delta = ((wp >> 16) & 0xFFFF) as i16 as f32;
    let ui = &mut app.ui;
    let mut pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
    if !ui.hwnd.is_null() {
        unsafe { ScreenToClient(ui.hwnd, &mut pt) };
    }
    let tabs = &mut ui.tabs;
    if tabs.max > 0.0 && tabs.area.contains(pt.x as f32, pt.y as f32) {
        let step = delta / 120.0 * tabs.tab_w * 0.8;
        tabs.target = (tabs.target + if horizontal { step } else { -step }).clamp(0.0, tabs.max);
    } else if !horizontal {
        ui.scroll = (ui.scroll - delta / 120.0 * 72.0 * ui.scale).clamp(0.0, ui.scroll_max);
    }
    invalidate(ui.hwnd);
}

fn select_variant(app: &mut App, v: usize) {
    if let View::Detail { archive, variant } = &mut app.ui.view {
        let count = app.state.installed.iter().find(|i| i.archive == *archive).map_or(1, |i| i.schemes.len());
        *variant = v.min(count.saturating_sub(1));
        app.ui.tabs.reveal = true;
    }
}

pub fn on_key(app: &mut App, vk: u16) {
    if (vk == VK_ESCAPE || vk == VK_BACK) && app.ui.theme_open {
        app.ui.theme_open = false;
    } else if (vk == VK_ESCAPE || vk == VK_BACK) && app.ui.view != View::List {
        go_list(app);
    } else if let View::Detail { variant, .. } = app.ui.view {
        // Left / right arrows step through a pack's variants.
        if vk == VK_LEFT && variant > 0 {
            select_variant(app, variant - 1);
        } else if vk == VK_RIGHT {
            select_variant(app, variant + 1);
        }
    }
    invalidate(app.ui.hwnd);
}

fn go_list(app: &mut App) {
    app.ui.view = View::List;
    app.ui.scroll = 0.0;
    app.ui.hover = None;
}

pub fn open_detail(app: &mut App, archive: &str) {
    let Some(inst) = app.find(archive) else { return };
    if inst.schemes.is_empty() {
        return;
    }
    let variant = match &app.state.active {
        Some(a) if a.archive == archive && app.ours_is_current() => a.scheme,
        _ => 0,
    };
    app.ui.view = View::Detail { archive: archive.to_string(), variant };
    app.ui.scroll = 0.0;
    app.ui.tabs = TabStrip { reveal: true, snap: true, ..TabStrip::default() };
    app.ui.hover = None;
}

fn action(app: &mut App, (kind, id): Hit) {
    match kind {
        Kind::Theme if id == 0 => app.ui.theme_open = !app.ui.theme_open,
        Kind::ThemeItem => {
            app.state.ui.theme = id as u8;
            app.ui.theme_open = false;
            app.save();
            apply_theme(app);
        }
        Kind::Pin => {
            app.state.ui.on_top = !app.state.ui.on_top;
            app.save();
            apply_on_top(app);
        }
        Kind::Back => go_list(app),
        Kind::Row => {
            if let Some(archive) = app.ui.row_order.get(id).cloned() {
                open_detail(app, &archive);
            }
        }
        Kind::ColHeader => {
            let s = &mut app.state.ui;
            if s.sort_column == id {
                s.sort_ascending = !s.sort_ascending;
            } else {
                s.sort_column = id;
                s.sort_ascending = id == 0 || id == 2; // dates, sizes and counts: biggest first
            }
            app.save();
        }
        Kind::Toggle if id == 0 => app.toggle_startup(),
        Kind::Toggle => {
            app.state.ui.close_to_tray = !app.state.ui.close_to_tray;
            app.save();
        }
        Kind::Segment => select_variant(app, id),
        Kind::TabPrev => app.ui.tabs.target = (app.ui.tabs.target - app.ui.tabs.tab_w * 2.0).max(0.0),
        Kind::TabNext => app.ui.tabs.target = (app.ui.tabs.target + app.ui.tabs.tab_w * 2.0).min(app.ui.tabs.max),
        Kind::Button => button(app, id),
        _ => {}
    }
    crate::refresh_tray(app);
}

fn detail_target(app: &App) -> Option<Active> {
    match &app.ui.view {
        View::Detail { archive, variant } => Some(Active { archive: archive.clone(), scheme: *variant }),
        View::List => None,
    }
}

fn button(app: &mut App, id: usize) {
    match id {
        BTN_ADD => add_packs_dialog(app),
        BTN_FOLDER => crate::open_folder(app),
        BTN_RESTORE => {
            if let Err(e) = app.restore() {
                message(app, &e, true);
            }
        }
        BTN_APPLY => {
            if let Some(t) = detail_target(app) {
                if let Err(e) = app.apply(t) {
                    message(app, &format!("Could not apply the cursors: {e}"), true);
                }
            }
        }
        BTN_SHOW => {
            if let Some(t) = detail_target(app) {
                show_in_folder(&app.drop_dir.join(&t.archive));
            }
        }
        BTN_REMOVE => {
            if let Some(t) = detail_target(app) {
                remove_pack(app, &t.archive);
            }
        }
        _ => {}
    }
}

fn row_menu(app: &mut App, archive: &str) {
    let ok = app.find(archive).is_some_and(|i| !i.schemes.is_empty());
    unsafe {
        let menu = CreatePopupMenu();
        let add = |id: usize, text: &str, enabled: bool| {
            let w = wide(text);
            AppendMenuW(menu, MF_STRING | if enabled { 0 } else { MF_GRAYED }, id, w.as_ptr());
        };
        add(1, "Show cursors", ok);
        add(2, "Use these cursors", ok);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(3, "Show in folder", true);
        add(4, "Remove pack", true);
        if ok {
            SetMenuDefaultItem(menu, 1, 0);
        }
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, app.ui.hwnd, null());
        DestroyMenu(menu);
        match cmd {
            1 => open_detail(app, archive),
            2 => {
                if let Err(e) = app.apply(Active { archive: archive.to_string(), scheme: 0 }) {
                    message(app, &format!("Could not apply the cursors: {e}"), true);
                }
            }
            3 => show_in_folder(&app.drop_dir.join(archive)),
            4 => remove_pack(app, archive),
            _ => {}
        }
    }
    crate::refresh_tray(app);
    invalidate(app.ui.hwnd);
}

fn message(app: &App, text: &str, error: bool) {
    let t = wide("CursorDrop");
    let m = wide(text);
    unsafe { MessageBoxW(app.ui.hwnd, m.as_ptr(), t.as_ptr(), MB_OK | if error { MB_ICONWARNING } else { MB_ICONINFORMATION }) };
}

fn show_in_folder(path: &Path) {
    let args = wide(&format!("/select,\"{}\"", path.display()));
    let exe = wide("explorer.exe");
    let op = wide("open");
    unsafe { ShellExecuteW(null_mut(), op.as_ptr(), exe.as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL) };
}

/// Moves the archive to the Recycle Bin; the next scan cleans up (and restores if it was in use).
fn remove_pack(app: &mut App, archive: &str) {
    let t = wide("CursorDrop");
    let q = wide(&format!("Move \"{archive}\" to the Recycle Bin?\n\nIf its cursors are in use, your previous cursors come back."));
    if unsafe { MessageBoxW(app.ui.hwnd, q.as_ptr(), t.as_ptr(), MB_YESNO | MB_ICONQUESTION) } != IDYES {
        return;
    }
    let path = app.drop_dir.join(archive);
    let mut from: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().collect();
    from.extend([0, 0]);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.hwnd = app.ui.hwnd;
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;
    unsafe { SHFileOperationW(&mut op) };
    go_list(app);
    crate::rescan(app);
}

fn add_packs_dialog(app: &mut App) {
    let mut buf = vec![0u16; 32 * 1024];
    let filter: Vec<u16> = "Cursor packs (*.zip, *.7z, *.rar)\0*.zip;*.7z;*.rar\0All files\0*.*\0\0".encode_utf16().collect();
    let title = wide("Add cursor packs");
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = app.ui.hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return;
    }
    // Either one full path, or the folder followed by file names, all NUL-separated.
    let parts: Vec<String> = buf
        .split(|&c| c == 0)
        .take_while(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    let files: Vec<PathBuf> = match parts.len() {
        0 => vec![],
        1 => vec![PathBuf::from(&parts[0])],
        _ => parts[1..].iter().map(|f| Path::new(&parts[0]).join(f)).collect(),
    };
    add_packs(app, &files);
}

/// Copies archives into the cursors folder, installs them and shows (and uses) the last one.
pub fn add_packs(app: &mut App, files: &[PathBuf]) {
    let mut last = None;
    let mut skipped = Vec::new();
    for f in files {
        if !crate::archive::is_archive(f) {
            skipped.push(f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            continue;
        }
        let Some(name) = f.file_name() else { continue };
        let dest = app.drop_dir.join(name);
        if dest != *f {
            if let Err(e) = std::fs::copy(f, &dest) {
                message(app, &format!("Could not copy {}: {e}", f.display()), true);
                continue;
            }
            // A fresh modification time marks it as new even if an older copy was there.
            if let Ok(file) = std::fs::File::options().write(true).open(&dest) {
                let _ = file.set_modified(std::time::SystemTime::now());
            }
        }
        last = Some(name.to_string_lossy().to_string());
    }
    crate::rescan(app);
    if let Some(name) = last {
        if app.find(&name).is_some_and(|i| !i.schemes.is_empty()) {
            let _ = app.apply(Active { archive: name.clone(), scheme: 0 });
            open_detail(app, &name);
        } else if let Some(err) = app.find(&name).and_then(|i| i.error.clone()) {
            message(app, &format!("{name}: {err}"), true);
        }
    }
    if !skipped.is_empty() {
        message(app, &format!("Only .zip, .7z and .rar cursor packs can be added. Skipped:\n{}", skipped.join("\n")), false);
    }
    crate::refresh_tray(app);
    invalidate(app.ui.hwnd);
}

/// WM_DROPFILES (only used if OLE drag and drop couldn't be set up).
pub fn on_drop(app: &mut App, hdrop: HDROP) {
    let files = crate::dragdrop::hdrop_files(hdrop);
    unsafe { DragFinish(hdrop) };
    add_packs(app, &files);
}

/// What's being dragged over the window, for the overlay.
#[derive(Clone)]
pub struct DragInfo {
    packs: Vec<String>,
    others: usize,
}

impl DragInfo {
    fn new(files: &[PathBuf]) -> DragInfo {
        let packs: Vec<String> = files
            .iter()
            .filter(|f| crate::archive::is_archive(f))
            .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();
        DragInfo { others: files.len() - packs.len(), packs }
    }
}

/// From the OLE drop target. Returns whether the drag holds something we'd take.
pub fn on_drag(app: &mut App, event: crate::dragdrop::DragEvent) -> bool {
    use crate::dragdrop::DragEvent;
    let ui = &mut app.ui;
    let welcome = match event {
        DragEvent::Enter(files) => {
            let info = DragInfo::new(&files);
            let ok = !info.packs.is_empty();
            ui.drag = Some(info);
            ui.theme_open = false;
            ok
        }
        DragEvent::Over => ui.drag.as_ref().is_some_and(|d| !d.packs.is_empty()),
        DragEvent::Leave => {
            ui.drag = None;
            false
        }
        DragEvent::Drop(files) => {
            ui.drag = None;
            // Handled once the drag is over, so a message box can't hold up Explorer's drag.
            ui.dropped = files;
            unsafe { PostMessageW(ui.hwnd, crate::WM_DROPPED, 0, 0) };
            true
        }
    };
    if !ui.hwnd.is_null() {
        unsafe { SetTimer(ui.hwnd, crate::TIMER_ANIM, crate::ANIM_MS, None) };
    }
    invalidate(ui.hwnd);
    welcome
}

pub fn take_dropped(app: &mut App) {
    let files = std::mem::take(&mut app.ui.dropped);
    if !files.is_empty() {
        add_packs(app, &files);
    }
}

/// The drop overlay: the window dims, a dashed accent border "marches" around it and a card
/// in the middle bobs a drop icon and says what will happen. `t` fades it in and out.
fn draw_drop_overlay(pt: &mut Painter, info: &DragInfo, t: f32, w: f32, h: f32) {
    let p = pt.p;
    let ease = t * t * (3.0 - 2.0 * t);
    let fade = |c: Color| with_alpha(c, (alpha(c) as f32 * ease) as i32);
    let ok = !info.packs.is_empty();
    let tint = if ok { pt.accent } else { WARNING };
    let secs = pt.ms as f32 / 1000.0;

    pt.g.fill_round(R::new(0.0, 0.0, w, h), 0.0, fade(with_alpha(p.bg, 215)));
    pt.g.glow(w / 2.0, h / 2.0, w * 0.45, h * 0.45, fade(with_alpha(tint, if pt.dark { 34 } else { 30 })));
    let frame = R::new(pt.s(12.0), pt.s(12.0), w - pt.s(24.0), h - pt.s(24.0));
    pt.g.dashed_round(frame, pt.s(18.0), fade(tint), pt.s(2.5), secs * 6.0);

    // The card rises into place as it fades in.
    let (cw, ch) = (pt.s(420.0), pt.s(190.0));
    let card = R::new((w - cw) / 2.0, (h - ch) / 2.0 + (1.0 - ease) * pt.s(24.0), cw, ch);
    pt.g.fill_round(R::new(card.x, card.y + pt.s(4.0), card.w, card.h), pt.s(18.0), fade(p.shadow));
    pt.g.fill_round(card, pt.s(18.0), fade(with_alpha(p.card, 255)));
    pt.g.stroke_round(card, pt.s(18.0), fade(with_alpha(tint, 140)), pt.s(1.5));

    // A bobbing badge with the drop (or warning) glyph.
    let bob = (secs * std::f32::consts::TAU * 0.9).sin() * pt.s(5.0);
    let badge = R::new(card.x + card.w / 2.0 - pt.s(30.0), card.y + pt.s(24.0) + bob, pt.s(60.0), pt.s(60.0));
    pt.g.ellipse(badge.x, badge.y, badge.w, badge.h, fade(with_alpha(tint, 40)));
    pt.g.text(if ok { G_DROP } else { G_WARNING }, &pt.f.big_icon, fade(tint), badge, Align::Center, Align::Center);

    let (title, detail) = if ok {
        let title = match info.packs.len() {
            1 => format!("Drop to add {}", info.packs[0]),
            n => format!("Drop to add {n} cursor packs"),
        };
        let detail = match info.others {
            0 => "It's copied into the cursors folder and used straight away".to_string(),
            1 => "1 other file isn't a cursor pack and will be skipped".to_string(),
            n => format!("{n} other files aren't cursor packs and will be skipped"),
        };
        (title, detail)
    } else {
        ("That's not a cursor pack".to_string(), "CursorDrop takes .zip, .7z and .rar archives".to_string())
    };
    let tr = R::new(card.x + pt.s(20.0), card.y + pt.s(102.0), card.w - pt.s(40.0), pt.s(28.0));
    pt.g.text(&title, &pt.f.title, fade(p.text), tr, Align::Center, Align::Center);
    let dr = R::new(card.x + pt.s(20.0), card.y + pt.s(136.0), card.w - pt.s(40.0), pt.s(20.0));
    pt.g.text(&detail, &pt.f.caption, fade(p.subtext), dr, Align::Center, Align::Center);
}

/// Called ~30 times a second while the window is visible: animates .ani cursors, switches,
/// the tab strip and the drop overlay.
pub fn on_anim_tick(app: &mut App) {
    let goals = [app.startup_enabled(), app.state.ui.close_to_tray];
    let knobs_moving = app.ui.knobs.iter().zip(goals).any(|(k, on)| (k - if on { 1.0 } else { 0.0 }).abs() > 0.001);
    let tabs_moving = (app.ui.tabs.scroll - app.ui.tabs.target).abs() > 0.01;
    let overlay = app.ui.drag.is_some() || app.ui.drag_t > 0.0;
    if app.ui.animated || tabs_moving || knobs_moving || overlay {
        invalidate(app.ui.hwnd);
    }
}

pub fn set_dpi(app: &mut App, dpi: u32) {
    app.ui.scale = dpi as f32 / 96.0;
    app.ui.fonts = None;
    app.ui.cursors.clear();
}

pub fn changed_files(app: &mut App) {
    app.ui.cursors.clear();
    forget_file_comparisons();
    invalidate(app.ui.hwnd);
}


/// Self-test: renders the views into PNGs without showing a window or touching the cursors.
pub fn self_test(app: &mut App, dir: &Path) {
    let (w, h) = (1100, 720);
    let first = app.state.installed.iter().find(|i| !i.schemes.is_empty()).map(|i| i.archive.clone());
    let multi = app.state.installed.iter().find(|i| i.schemes.len() > 1).map(|i| i.archive.clone());
    let mut shots: Vec<(&str, u8, View)> = vec![("list-light", THEME_LIGHT, View::List), ("list-dark", THEME_DARK, View::List)];
    if let Some(a) = &first {
        shots.push(("detail-dark", THEME_DARK, View::Detail { archive: a.clone(), variant: 0 }));
        shots.push(("detail-light", THEME_LIGHT, View::Detail { archive: a.clone(), variant: 0 }));
    }
    if let Some(a) = &multi {
        shots.push(("variants-dark", THEME_DARK, View::Detail { archive: a.clone(), variant: 1 }));
    }
    for (name, theme, view) in shots {
        app.state.ui.theme = theme;
        apply_theme(app);
        app.ui.view = view;
        app.ui.theme_open = name == "list-dark";
        if name == "list-light" {
            app.ui.hover = None;
        }
        // Paint twice: the first pass lays out the elements, the second can show hover.
        paint_to(app, w, h);
        if name == "list-light" {
            app.ui.hover = Some((Kind::Row, 0));
        }
        paint_to(app, w, h);
        let ok = app.ui.canvas.as_ref().unwrap().save_png(&dir.join(format!("{name}.png")).to_string_lossy());
        println!("{name}: {}", if ok { "ok" } else { "FAILED" });
    }

    // Clicks, through the same handlers the window uses (nothing here touches the registry).
    let check = |what: &str, ok: bool| println!("{what}: {}", if ok { "ok" } else { "FAILED" });
    let click = |app: &mut App, kind: Kind, id: usize| -> bool {
        paint_to(app, w, h);
        let Some(e) = app.ui.elems.iter().rev().find(|e| e.kind == kind && e.id == id) else { return false };
        let (x, y) = ((e.r.x + e.r.w / 2.0) as i32, (e.r.y + e.r.h / 2.0) as i32);
        let lp = ((y as u32 as isize) << 16) | (x as u16 as isize);
        on_mouse_move(app, lp);
        on_button_down(app, lp);
        on_button_up(app, lp);
        true
    };
    app.state.ui.theme = THEME_DARK;
    apply_theme(app);
    app.ui.view = View::List;
    app.ui.theme_open = false;
    let row = app.ui.row_order.iter().position(|a| Some(a) == first.as_ref()).unwrap_or(0);
    paint_to(app, w, h);
    let row = app.ui.row_order.iter().position(|a| Some(a) == first.as_ref()).unwrap_or(row);
    click(app, Kind::Row, row);
    check("click row opens its cursors", matches!(&app.ui.view, View::Detail { archive, .. } if Some(archive) == first.as_ref()));
    paint_to(app, w, h);
    on_wheel(app, (-240i32 as u32 as usize) << 16, 0, false);
    paint_to(app, w, h);
    check("wheel scrolls the grid", app.ui.scroll > 0.0);
    paint_to(app, w, h);
    app.ui.canvas.as_ref().unwrap().save_png(&dir.join("detail-scrolled.png").to_string_lossy());
    click(app, Kind::Back, 0);
    check("back returns to the list", app.ui.view == View::List);
    click(app, Kind::ColHeader, 3);
    check("size header sorts by size, biggest first", app.state.ui.sort_column == 3 && !app.state.ui.sort_ascending);
    click(app, Kind::ColHeader, 3);
    check("again sorts smallest first", app.state.ui.sort_ascending);
    let sizes: Vec<u64> = app.ui.row_order.iter().filter_map(|a| app.find(a).map(|i| i.size)).collect();
    paint_to(app, w, h);
    let sizes2: Vec<u64> = app.ui.row_order.iter().filter_map(|a| app.find(a).map(|i| i.size)).collect();
    check("rows are in size order", sizes2.windows(2).all(|p| p[0] <= p[1]) && sizes.len() == sizes2.len());
    click(app, Kind::Theme, 0);
    check("theme button opens the menu", app.ui.theme_open);
    click(app, Kind::ThemeItem, 1);
    check("picking Light switches theme", !app.ui.theme_open && !app.ui.dark && app.state.ui.theme == THEME_LIGHT);
    let before = app.state.ui.columns[0];
    paint_to(app, w, h);
    if let Some(e) = app.ui.elems.iter().find(|e| e.kind == Kind::ColSplit && e.id == 0) {
        let (x, y) = ((e.r.x + e.r.w / 2.0) as i32, (e.r.y + e.r.h / 2.0) as i32);
        let at = |x: i32| ((y as u32 as isize) << 16) | (x as u16 as isize);
        on_mouse_move(app, at(x));
        on_button_down(app, at(x));
        on_mouse_move(app, at(x + 60));
        on_button_up(app, at(x + 60));
    }
    check("dragging a divider widens the column", (app.state.ui.columns[0] - before - 60.0).abs() < 1.0);
    paint_to(app, w, h);
    app.ui.canvas.as_ref().unwrap().save_png(&dir.join("after-clicks.png").to_string_lossy());

    // Close to tray: on by default, the second footer switch turns it off and on.
    check("close to tray is on by default", app.state.ui.close_to_tray);
    click(app, Kind::Toggle, 1);
    check("its switch turns it off", !app.state.ui.close_to_tray);
    click(app, Kind::Toggle, 1);
    check("and on again", app.state.ui.close_to_tray);

    // Dragging over the window: the overlay fades in, and a drop adds the pack.
    use crate::dragdrop::DragEvent;
    let source = first.as_ref().map(|a| app.drop_dir.join(a));
    if let Some(source) = source {
        let tmp = std::env::temp_dir().join(format!("cursordrop-drag-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let dragged = tmp.join("Dragged Pack.zip");
        let _ = std::fs::copy(&source, &dragged);
        let notes = tmp.join("notes.txt");
        let _ = std::fs::write(&notes, "not a pack");
        app.state.ui.theme = THEME_DARK;
        apply_theme(app);
        go_list(app);
        let welcome = on_drag(app, DragEvent::Enter(vec![dragged.clone(), notes.clone()]));
        check("a pack being dragged is welcome", welcome);
        for _ in 0..12 {
            paint_to(app, w, h);
        }
        check("the overlay fades in", app.ui.drag_t > 0.95);
        app.ui.canvas.as_ref().unwrap().save_png(&dir.join("drop-overlay.png").to_string_lossy());
        on_drag(app, DragEvent::Leave);
        let refused = !on_drag(app, DragEvent::Enter(vec![notes.clone()]));
        check("a plain file is refused", refused);
        app.state.ui.theme = THEME_LIGHT;
        apply_theme(app);
        for _ in 0..12 {
            paint_to(app, w, h);
        }
        app.ui.canvas.as_ref().unwrap().save_png(&dir.join("drop-refused.png").to_string_lossy());
        on_drag(app, DragEvent::Leave);
        for _ in 0..30 {
            paint_to(app, w, h);
        }
        check("the overlay fades out", app.ui.drag_t == 0.0);
        on_drag(app, DragEvent::Enter(vec![dragged.clone()]));
        on_drag(app, DragEvent::Drop(vec![dragged.clone()]));
        check("a drop is queued until the drag is over", app.ui.dropped.len() == 1 && app.ui.drag.is_none());
        take_dropped(app);
        check(
            "the dropped pack is added, opened and used",
            app.find("Dragged Pack.zip").is_some_and(|i| !i.schemes.is_empty())
                && matches!(&app.ui.view, View::Detail { archive, .. } if archive == "Dragged Pack.zip")
                && app.state.active.as_ref().is_some_and(|a| a.archive == "Dragged Pack.zip"),
        );
        let _ = std::fs::remove_dir_all(&tmp);
        go_list(app);
    }

    // The variant tabs of the pack with the most variants.
    let Some(many) = app.state.installed.iter().max_by_key(|i| i.schemes.len()).filter(|i| i.schemes.len() >= 8).map(|i| i.archive.clone())
    else {
        println!("tabs: skipped (no pack with 8+ variants)");
        return;
    };
    app.state.ui.theme = THEME_DARK;
    apply_theme(app);
    open_detail(app, &many);
    let settle = |app: &mut App| {
        for _ in 0..40 {
            paint_to(app, w, h);
        }
    };
    settle(app);
    check("too many tabs overflow and scroll", app.ui.tabs.max > 0.0 && app.ui.tabs.scroll == 0.0);
    check("tabs keep their minimum width", app.ui.tabs.tab_w >= TAB_MIN * app.ui.scale - 0.5);
    app.ui.canvas.as_ref().unwrap().save_png(&dir.join("tabs-start.png").to_string_lossy());
    let over = |app: &App| {
        let a = app.ui.tabs.area;
        let (x, y) = ((a.x + a.w / 2.0) as i32, (a.y + a.h / 2.0) as i32);
        ((y as u32 as isize) << 16) | (x as u16 as isize)
    };
    let down = (-120i32 as u32 as usize) << 16;
    let grid_before = app.ui.scroll;
    on_wheel(app, down, over(app), false);
    settle(app);
    check("wheel down over the tabs scrolls them forwards", app.ui.tabs.scroll > 0.0 && app.ui.scroll == grid_before);
    let s1 = app.ui.tabs.scroll;
    on_wheel(app, 120usize << 16, over(app), true);
    settle(app);
    check("tilting right scrolls further", app.ui.tabs.scroll > s1);
    let s2 = app.ui.tabs.scroll;
    click(app, Kind::TabPrev, 0);
    settle(app);
    check("left arrow scrolls back", app.ui.tabs.scroll < s2);
    for _ in 0..50 {
        click(app, Kind::TabNext, 0);
    }
    settle(app);
    check("right arrow reaches the end", (app.ui.tabs.scroll - app.ui.tabs.max).abs() < 0.5);
    app.ui.canvas.as_ref().unwrap().save_png(&dir.join("tabs-end.png").to_string_lossy());
    on_key(app, VK_RIGHT);
    settle(app);
    check("right key picks the next variant and scrolls back to it", matches!(app.ui.view, View::Detail { variant: 1, .. }) && app.ui.tabs.scroll < app.ui.tabs.max);
    on_key(app, VK_LEFT);
    settle(app);
    check("left key picks the previous variant and scrolls to it", matches!(app.ui.view, View::Detail { variant: 0, .. }) && app.ui.tabs.scroll == 0.0);
    // Hover a tab whose name is cut off: it grows to show the full name.
    paint_to(app, w, h);
    if let Some(e) = app.ui.elems.iter().find(|e| e.kind == Kind::Segment && e.id == 2) {
        let (x, y) = ((e.r.x + e.r.w / 2.0) as i32, (e.r.y + e.r.h / 2.0) as i32);
        on_mouse_move(app, ((y as u32 as isize) << 16) | (x as u16 as isize));
    }
    paint_to(app, w, h);
    check("hovering a tab", app.ui.hover == Some((Kind::Segment, 2)));
    app.ui.canvas.as_ref().unwrap().save_png(&dir.join("tabs-hover.png").to_string_lossy());
    app.state.ui.theme = THEME_LIGHT;
    apply_theme(app);
    paint_to(app, w, h);
    app.ui.canvas.as_ref().unwrap().save_png(&dir.join("tabs-hover-light.png").to_string_lossy());
}
