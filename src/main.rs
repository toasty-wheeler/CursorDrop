#![windows_subsystem = "windows"]
//! CursorDrop: drop a cursor-pack archive (.zip / .7z / .rar) into the `cursors` folder next to
//! the exe and it becomes your Windows cursor scheme. A window lists the packs (Explorer-style)
//! and shows every cursor a pack replaces; the app keeps running in the tray.

mod archive;
mod cursors;
mod dragdrop;
mod edit;
mod gfx;
mod schedule;
mod scheme;
mod ui;

use cursors::wide;
use scheme::{ROLE_LABELS, ROLES, Scheme};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::UNIX_EPOCH;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const APP_NAME: &str = "CursorDrop";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const CLASS_NAME: &str = "CursorDropWnd";
const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW_WINDOW: u32 = WM_APP + 2;
pub const WM_DROPPED: u32 = WM_APP + 3;
const TIMER_SCAN: usize = 1;
pub const TIMER_ANIM: usize = 2;
const SCAN_MS: u32 = 2000;
pub const ANIM_MS: u32 = 33;
const CLIENT_W: f32 = 1100.0;
const CLIENT_H: f32 = 720.0;

const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_MOUSEHWHEEL: u32 = 0x020E;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const CMD_SHOW: usize = 6;
const CMD_SCHEDULE: usize = 7;
const CMD_OPEN: usize = 1;
const CMD_RESCAN: usize = 2;
const CMD_RESTORE: usize = 3;
const CMD_STARTUP: usize = 4;
const CMD_EXIT: usize = 5;
const CMD_SCHEME_BASE: usize = 1000;

#[derive(Default, Serialize, Deserialize)]
struct State {
    installed: Vec<Installed>,
    active: Option<Active>,
    backup: Option<cursors::Backup>,
    #[serde(default)]
    ui: ui::UiSettings,
    /// Extraction / naming version the installed packs were made with (see FORMAT).
    #[serde(default)]
    format: u32,
    /// Cursors chosen by hand on pack variants (see edit.rs), keyed "archive{1}variant".
    #[serde(default)]
    overrides: std::collections::BTreeMap<String, std::collections::BTreeMap<usize, edit::Pick>>,
    /// Changes made on "Current cursors", waiting to be saved as a preset.
    #[serde(default)]
    current_draft: std::collections::BTreeMap<usize, edit::Pick>,
    #[serde(default)]
    presets: Vec<edit::Preset>,
    #[serde(default)]
    schedule: schedule::Schedule,
}

/// Bump when extraction or scheme naming changes: installed packs are then re-extracted once.
/// 2: archives inside archives, UTF-8 zip names, short variant labels.
/// 3: more file names recognised (accuracy, stop, width, ne/nw, "<name>" = normal pointer).
const FORMAT: u32 = 3;

#[derive(Clone, Serialize, Deserialize)]
pub struct Installed {
    pub archive: String,
    pub size: u64,
    pub mtime: u64,
    pub schemes: Vec<Scheme>,
    pub error: Option<String>,
}

/// What CursorDrop applied: a pack's variant, or a preset (then `archive` is empty).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Active {
    pub archive: String,
    pub scheme: usize,
    #[serde(default)]
    pub preset: Option<String>,
}

pub struct App {
    exe: PathBuf,
    pub drop_dir: PathBuf,
    data_dir: PathBuf,
    state: State,
    pending: HashMap<String, (u64, u64)>,
    hwnd: HWND,
    icon: HICON,
    menu_map: Vec<Active>,
    /// Self-test: never touch the real cursors.
    no_apply: bool,
    pub ui: ui::Ui,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Where the cursors and data folders live: next to the exe (portable), or, if that folder
/// can't be written to (e.g. Program Files), %LOCALAPPDATA%\CursorDrop.
fn base_dir(exe: &Path) -> PathBuf {
    let here = exe.parent().unwrap().to_path_buf();
    let probe = here.join(".cursordrop-write-test");
    if fs::write(&probe, b"").is_ok() {
        let _ = fs::remove_file(&probe);
        return here;
    }
    match std::env::var_os("LOCALAPPDATA") {
        Some(local) => PathBuf::from(local).join(APP_NAME),
        None => here,
    }
}

fn file_sig(p: &Path) -> Option<(u64, u64)> {
    let m = fs::metadata(p).ok()?;
    let mtime = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), mtime))
}

impl App {
    fn new() -> App {
        let exe = std::env::current_exe().expect("exe path");
        let base = base_dir(&exe);
        let drop_dir = base.join("cursors");
        let data_dir = base.join("data");
        let _ = fs::create_dir_all(&drop_dir);
        let _ = fs::create_dir_all(data_dir.join("schemes"));
        let state = fs::read_to_string(data_dir.join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        App {
            exe,
            drop_dir,
            data_dir,
            state,
            pending: HashMap::new(),
            hwnd: null_mut(),
            icon: null_mut(),
            menu_map: Vec::new(),
            no_apply: false,
            ui: ui::Ui::default(),
        }
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.state) {
            let _ = fs::write(self.data_dir.join("state.json"), s);
        }
    }

    fn log(&self, msg: &str) {
        let secs = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(self.data_dir.join("log.txt")) {
            let _ = writeln!(f, "[{secs}] {msg}");
        }
    }

    pub fn scheme_dir(&self, archive: &str) -> PathBuf {
        self.data_dir.join("schemes").join(archive)
    }

    pub fn find(&self, archive: &str) -> Option<&Installed> {
        self.state.installed.iter().find(|i| i.archive == archive)
    }

    /// The pack scheme in use (None for presets).
    fn active_scheme(&self) -> Option<&Scheme> {
        let a = self.state.active.as_ref()?;
        if a.preset.is_some() {
            return None;
        }
        self.find(&a.archive)?.schemes.get(a.scheme)
    }

    /// True if the cursors Windows is using right now are the scheme we applied.
    pub fn ours_is_current(&self) -> bool {
        match self.active_name() {
            Some(_) if self.no_apply => true,
            Some(name) => cursors::current_scheme_name().is_some_and(|n| n == name),
            None => false,
        }
    }

    /// Applies a scheme. Returns the name shown to the user.
    pub fn apply(&mut self, target: Active) -> Result<String, String> {
        self.apply_inner(target, true)
    }

    /// `may_back_up`: false when the cursors in use are known to be ours already (even if
    /// their scheme name changed), so they never get saved as the "original" cursors.
    fn apply_inner(&mut self, target: Active, may_back_up: bool) -> Result<String, String> {
        // Windows' name for it, and what the user sees.
        let (name, shown) = match &target.preset {
            Some(id) => {
                let p = self.preset(id).ok_or("preset not found")?;
                (p.name.clone(), p.name.clone())
            }
            None => {
                let inst = self.find(&target.archive).ok_or("archive not installed")?;
                let scheme = inst.schemes.get(target.scheme).ok_or("no such scheme")?;
                (scheme.name.clone(), scheme.display().to_string())
            }
        };
        // The pack's own cursors with whatever was chosen by hand ("keep" slots keep what
        // Windows has now, so they are read before anything changes).
        let paths = self.sheet_paths(&target.sheet());
        if !self.no_apply {
            if may_back_up && !self.ours_is_current() {
                // Remember what the user had so "Restore original cursors" can bring it back.
                self.state.backup = Some(cursors::take_backup());
            }
            cursors::apply(&name, &paths)?;
        }
        self.state.active = Some(target);
        self.save();
        let n = paths.iter().filter(|p| p.is_some()).count();
        self.log(&format!("applied '{name}' ({n} of {} cursors)", ROLES.len()));
        gfx::invalidate(self.ui.hwnd);
        Ok(shown)
    }

    pub fn restore(&mut self) -> Result<(), String> {
        let b = self.state.backup.clone().ok_or("nothing to restore")?;
        if !self.no_apply {
            cursors::restore(&b)?;
        }
        self.state.active = None;
        self.save();
        self.log("restored original cursors");
        gfx::invalidate(self.ui.hwnd);
        Ok(())
    }

    fn install(&self, name: &str, sig: (u64, u64)) -> Installed {
        let dir = self.scheme_dir(name);
        let _ = fs::remove_dir_all(&dir);
        let stem = Path::new(name).file_stem().unwrap_or_default().to_string_lossy().to_string();
        let mut inst = Installed { archive: name.to_string(), size: sig.0, mtime: sig.1, schemes: vec![], error: None };
        match archive::extract(&self.drop_dir.join(name), &dir) {
            Ok(_) => {
                inst.schemes = scheme::discover(&dir, &stem);
                if inst.schemes.is_empty() {
                    inst.error = Some("no recognizable cursors (.cur/.ani) found".into());
                }
            }
            Err(e) => inst.error = Some(e),
        }
        inst
    }

    /// Looks at the drop folder: installs new/changed archives, cleans up removed ones.
    /// Returns a notification message if something happened.
    fn scan(&mut self, wait_for_stable: bool) -> Option<(String, bool)> {
        let mut present: Vec<(String, (u64, u64))> = fs::read_dir(&self.drop_dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && archive::is_archive(p))
                    .filter_map(|p| Some((p.file_name()?.to_string_lossy().to_string(), file_sig(&p)?)))
                    .collect()
            })
            .unwrap_or_default();
        present.sort_by_key(|(_, sig)| sig.1);
        let mut note: Option<(String, bool)> = None;
        let mut changed = false;

        // Archives that were deleted from the folder.
        let gone: Vec<Installed> = self
            .state
            .installed
            .iter()
            .filter(|i| !present.iter().any(|(n, _)| *n == i.archive))
            .cloned()
            .collect();
        for inst in gone {
            changed = true;
            let was_active = self.state.active.as_ref().is_some_and(|a| a.archive == inst.archive);
            if was_active && self.ours_is_current() {
                let _ = self.restore();
                note = Some((format!("{} was removed, original cursors restored.", inst.archive), false));
            }
            if was_active {
                self.state.active = None;
            }
            if !self.no_apply {
                for s in &inst.schemes {
                    cursors::unregister_scheme(&s.name);
                }
            }
            let _ = fs::remove_dir_all(self.scheme_dir(&inst.archive));
            self.state.installed.retain(|i| i.archive != inst.archive);
            self.forget_overrides(&inst.archive);
            self.log(&format!("removed {}", inst.archive));
            self.save();
        }

        // New or changed archives (wait until the file stops changing, e.g. still copying).
        let mut to_apply: Option<Active> = None;
        for (name, sig) in present {
            let known = self.find(&name).is_some_and(|i| (i.size, i.mtime) == sig);
            if known {
                self.pending.remove(&name);
                continue;
            }
            if wait_for_stable && self.pending.get(&name) != Some(&sig) {
                self.pending.insert(name, sig);
                continue;
            }
            changed = true;
            self.pending.remove(&name);
            let inst = self.install(&name, sig);
            match &inst.error {
                Some(e) => {
                    self.log(&format!("{name}: {e}"));
                    note = Some((format!("{name}: {e}"), true));
                }
                None => {
                    self.log(&format!("installed {name}: {} scheme(s)", inst.schemes.len()));
                    to_apply = Some(Active::pack(&name, 0));
                }
            }
            if !self.no_apply {
                for old in self.find(&name).map(|i| i.schemes.clone()).unwrap_or_default() {
                    cursors::unregister_scheme(&old.name);
                }
            }
            // A changed archive may have different files: hand-picked cursors start over.
            self.forget_overrides(&name);
            self.state.installed.retain(|i| i.archive != name);
            self.state.installed.push(inst);
            self.save();
        }
        if changed {
            ui::changed_files(self);
        }
        if let Some(t) = to_apply {
            let extra = self.find(&t.archive).map_or(0, |i| i.schemes.len().saturating_sub(1));
            match self.apply(t) {
                Ok(n) => {
                    let more = if extra > 0 { format!(" ({extra} more variant(s) to pick from)") } else { String::new() };
                    note = Some((format!("Now using \"{n}\"{more}"), false));
                }
                Err(e) => note = Some((format!("Could not apply: {e}"), true)),
            }
        }
        note
    }

    /// Packs added by an older version are extracted and read again once, so changes to
    /// extraction (archives inside archives, file-name encoding) and naming reach them. The
    /// variant in use is found again by name, then by label, then by position.
    fn refresh_installed(&mut self) {
        if self.state.format >= FORMAT {
            return;
        }
        let active = self.state.active.clone();
        let active_scheme = self.active_scheme().cloned();
        let old: Vec<Installed> = self.state.installed.clone();
        for inst in &old {
            let path = self.drop_dir.join(&inst.archive);
            let Some(sig) = file_sig(&path) else { continue };
            let fresh = self.install(&inst.archive, sig);
            if !self.no_apply {
                for s in &inst.schemes {
                    cursors::unregister_scheme(&s.name);
                }
            }
            if let (Some(a), Some(was)) = (&active, &active_scheme) {
                if a.archive == inst.archive {
                    let idx = fresh
                        .schemes
                        .iter()
                        .position(|s| s.name == was.name)
                        .or_else(|| fresh.schemes.iter().position(|s| s.display() == was.display()))
                        .unwrap_or(a.scheme);
                    self.state.active = Some(Active::pack(&a.archive, idx));
                }
            }
            if let Some(slot) = self.state.installed.iter_mut().find(|i| i.archive == inst.archive) {
                *slot = fresh;
            }
            self.log(&format!("re-read {} for the new format", inst.archive));
        }
        self.state.format = FORMAT;
        self.save();
    }

    fn startup(&mut self) {
        // Checked before re-reading the packs, which can rename the scheme in use.
        let ours = self.ours_is_current();
        self.refresh_installed();
        // If the cursors in use are ours, re-apply so paths stay valid even if the folder moved.
        if let Some(a) = self.state.active.clone() {
            if ours && self.find(&a.archive).is_some_and(|i| a.scheme < i.schemes.len()) {
                let _ = self.apply_inner(a, false);
            } else {
                self.state.active = None;
                self.save();
            }
        }
    }

    pub fn startup_enabled(&self) -> bool {
        cursors::get_string(cursors::RUN_KEY, Some(APP_NAME)).is_some()
    }

    pub fn toggle_startup(&self) {
        if self.startup_enabled() {
            cursors::delete_value(cursors::RUN_KEY, APP_NAME);
        } else {
            let cmd = format!("\"{}\" --tray", self.exe.display());
            let _ = cursors::set_string(cursors::RUN_KEY, Some(APP_NAME), &cmd, false);
        }
        gfx::invalidate(self.ui.hwnd);
    }

    fn window_visible(&self) -> bool {
        !self.hwnd.is_null() && unsafe { IsWindowVisible(self.hwnd) } != 0
    }
}

/// Scans now (no waiting for the file to settle) and reports what happened.
pub fn rescan(app: &mut App) {
    if let Some((msg, err)) = app.scan(false) {
        if !app.window_visible() || err {
            balloon(app, &msg, err);
        }
    }
    gfx::invalidate(app.ui.hwnd);
}

// ---------------------------------------------------------------- tray

fn copy_wide(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn tray_data(app: &App) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = app.hwnd;
    nid.uID = 1;
    nid
}

fn tray_add(app: &App) {
    let mut nid = tray_data(app);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = app.icon;
    copy_wide(&mut nid.szTip, &tooltip(app));
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

fn tooltip(app: &App) -> String {
    match app.active_display() {
        Some(s) if app.ours_is_current() => format!("{APP_NAME} - {s}"),
        _ => format!("{APP_NAME} - drop cursor packs into the cursors folder"),
    }
}

pub fn refresh_tray(app: &App) {
    if app.hwnd.is_null() {
        return;
    }
    let mut nid = tray_data(app);
    nid.uFlags = NIF_TIP;
    copy_wide(&mut nid.szTip, &tooltip(app));
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

fn balloon(app: &App, text: &str, error: bool) {
    if app.hwnd.is_null() {
        return;
    }
    let mut nid = tray_data(app);
    nid.uFlags = NIF_INFO;
    copy_wide(&mut nid.szInfoTitle, APP_NAME);
    copy_wide(&mut nid.szInfo, text);
    nid.dwInfoFlags = if error { NIIF_WARNING } else { NIIF_INFO };
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

fn tray_remove(app: &App) {
    let nid = tray_data(app);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}

fn append(menu: HMENU, flags: u32, id: usize, text: &str) {
    let w = wide(&text.replace('&', "&&"));
    unsafe { AppendMenuW(menu, flags, id, w.as_ptr()) };
}

fn show_menu(app: &mut App) {
    unsafe {
        let menu = CreatePopupMenu();
        app.menu_map.clear();
        let current = app.ours_is_current();
        let active = if current { app.state.active.clone() } else { None };

        append(menu, MF_STRING, CMD_SHOW, "Open CursorDrop");
        SetMenuDefaultItem(menu, CMD_SHOW as u32, 0);
        append(menu, MF_SEPARATOR, 0, "");

        if app.state.installed.is_empty() {
            append(menu, MF_STRING | MF_GRAYED, 0, "Drop .zip / .7z / .rar cursor packs");
            append(menu, MF_STRING | MF_GRAYED, 0, "into the cursors folder");
        }
        let installed = app.state.installed.clone();
        for inst in &installed {
            if let Some(e) = &inst.error {
                append(menu, MF_STRING | MF_GRAYED, 0, &format!("{} - {}", inst.archive, e));
                continue;
            }
            let target = if inst.schemes.len() == 1 { menu } else { CreatePopupMenu() };
            for (i, s) in inst.schemes.iter().enumerate() {
                let t = Active::pack(&inst.archive, i);
                let checked = if active.as_ref() == Some(&t) { MF_CHECKED } else { MF_UNCHECKED };
                let label = if s.mapped() < ROLES.len() {
                    format!("{}   ({}/{})", s.display(), s.mapped(), ROLES.len())
                } else {
                    s.display().to_string()
                };
                append(target, MF_STRING | checked, CMD_SCHEME_BASE + app.menu_map.len(), &label);
                app.menu_map.push(t);
            }
            if inst.schemes.len() > 1 {
                let w = wide(&inst.archive.replace('&', "&&"));
                AppendMenuW(menu, MF_POPUP, target as usize, w.as_ptr());
            }
        }
        let presets = app.state.presets.clone();
        if !presets.is_empty() {
            append(menu, MF_SEPARATOR, 0, "");
            append(menu, MF_STRING | MF_GRAYED, 0, "Presets");
        }
        for p in &presets {
            let t = Active::preset(&p.id);
            let checked = if active.as_ref() == Some(&t) { MF_CHECKED } else { MF_UNCHECKED };
            append(menu, MF_STRING | checked, CMD_SCHEME_BASE + app.menu_map.len(), &p.name);
            app.menu_map.push(t);
        }

        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_OPEN, "Open cursors folder");
        append(menu, MF_STRING, CMD_RESCAN, "Rescan now");
        let can_restore = current && app.state.backup.is_some();
        append(menu, MF_STRING | if can_restore { 0 } else { MF_GRAYED }, CMD_RESTORE, "Restore original cursors");
        let sc = if app.state.schedule.enabled { MF_CHECKED } else { MF_UNCHECKED };
        append(menu, MF_STRING | sc, CMD_SCHEDULE, "Follow the schedule");
        let st = if app.startup_enabled() { MF_CHECKED } else { MF_UNCHECKED };
        append(menu, MF_STRING | st, CMD_STARTUP, "Start with Windows");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_EXIT, "Exit");

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(app.hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, app.hwnd, null()) as usize;
        PostMessageW(app.hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu); // also destroys submenus
        if cmd != 0 {
            run_command(app, cmd);
        }
    }
}

pub fn open_folder(app: &App) {
    let op = wide("open");
    let path = wide(&app.drop_dir.to_string_lossy());
    unsafe { ShellExecuteW(null_mut(), op.as_ptr(), path.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

fn show_window(app: &mut App) {
    unsafe {
        if IsIconic(app.hwnd) != 0 {
            ShowWindow(app.hwnd, SW_RESTORE);
        } else {
            ShowWindow(app.hwnd, SW_SHOW);
        }
        SetForegroundWindow(app.hwnd);
        SetTimer(app.hwnd, TIMER_ANIM, ANIM_MS, None);
    }
    gfx::invalidate(app.hwnd);
}

fn run_command(app: &mut App, cmd: usize) {
    match cmd {
        CMD_SHOW => show_window(app),
        CMD_OPEN => open_folder(app),
        CMD_RESCAN => rescan(app),
        CMD_RESTORE => match app.restore() {
            Ok(()) => balloon(app, "Original cursors restored.", false),
            Err(e) => balloon(app, &e, true),
        },
        CMD_STARTUP => app.toggle_startup(),
        CMD_SCHEDULE => app.toggle_schedule(),
        CMD_EXIT => unsafe {
            DestroyWindow(app.hwnd);
        },
        c if c >= CMD_SCHEME_BASE => {
            if let Some(t) = app.menu_map.get(c - CMD_SCHEME_BASE).cloned() {
                match app.apply(t) {
                    Ok(n) => balloon(app, &format!("Now using \"{n}\""), false),
                    Err(e) => balloon(app, &format!("Could not apply: {e}"), true),
                }
            }
        }
        _ => {}
    }
    refresh_tray(app);
}

/// The drop target's events, passed to the window (see dragdrop.rs).
fn drag_event(event: dragdrop::DragEvent) -> bool {
    let mut welcome = false;
    with_app(|app| welcome = ui::on_drag(app, event));
    welcome
}

fn with_app(f: impl FnOnce(&mut App)) -> bool {
    APP.with(|a| {
        if let Ok(mut guard) = a.try_borrow_mut() {
            if let Some(app) = guard.as_mut() {
                f(app);
                return true;
            }
        }
        false
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp == TIMER_SCAN => {
            with_app(|app| {
                if let Some((m, err)) = app.scan(true) {
                    if !app.window_visible() || err {
                        balloon(app, &m, err);
                    }
                }
                // A new day: the schedule picks its cursors.
                if let Some(name) = app.run_schedule(false) {
                    balloon(app, &format!("Today's cursors: {name}"), false);
                }
                refresh_tray(app);
            });
            0
        }
        WM_TIMER if wp == TIMER_ANIM => {
            with_app(|app| {
                if app.window_visible() {
                    ui::on_anim_tick(app);
                } else {
                    unsafe { KillTimer(hwnd, TIMER_ANIM) };
                }
            });
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_RBUTTONUP => {
                    with_app(show_menu);
                }
                WM_LBUTTONUP => {
                    with_app(show_window);
                }
                _ => {}
            }
            0
        }
        WM_SHOW_WINDOW => {
            with_app(show_window);
            0
        }
        WM_DROPPED => {
            with_app(ui::take_dropped);
            0
        }
        WM_PAINT => {
            unsafe {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(hwnd, &mut rc);
                with_app(|app| {
                    let src = ui::paint_to(app, rc.right.max(1), rc.bottom.max(1));
                    BitBlt(hdc, 0, 0, rc.right, rc.bottom, src, 0, 0, SRCCOPY);
                    ui::place_dialog_edit(app);
                });
                EndPaint(hwnd, &ps);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLOREDIT => {
            let mut brush = 0;
            with_app(|app| brush = ui::edit_colours(app, wp as HDC));
            if brush != 0 { brush } else { unsafe { DefWindowProcW(hwnd, msg, wp, lp) } }
        }
        WM_SIZE => {
            gfx::invalidate(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            with_app(|app| ui::on_mouse_move(app, lp));
            0
        }
        WM_MOUSELEAVE => {
            with_app(ui::on_mouse_leave);
            0
        }
        WM_LBUTTONDOWN => {
            with_app(|app| ui::on_button_down(app, lp));
            0
        }
        WM_LBUTTONUP => {
            with_app(|app| ui::on_button_up(app, lp));
            0
        }
        WM_RBUTTONUP => {
            with_app(|app| ui::on_right_click(app, lp));
            0
        }
        WM_MOUSEWHEEL => {
            with_app(|app| ui::on_wheel(app, wp, lp, false));
            0
        }
        WM_MOUSEHWHEEL => {
            with_app(|app| ui::on_wheel(app, wp, lp, true));
            0
        }
        WM_KEYDOWN => {
            with_app(|app| ui::on_key(app, wp as u16));
            0
        }
        WM_SETCURSOR if (lp & 0xFFFF) as u32 == HTCLIENT => {
            let mut handled = false;
            with_app(|app| handled = ui::on_set_cursor(app));
            if handled { 1 } else { unsafe { DefWindowProcW(hwnd, msg, wp, lp) } }
        }
        WM_DROPFILES => {
            with_app(|app| ui::on_drop(app, wp as HDROP));
            0
        }
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            let info = unsafe { &mut *(lp as *mut MINMAXINFO) };
            info.ptMinTrackSize = POINT { x: (1000.0 * dpi) as i32, y: (520.0 * dpi) as i32 };
            0
        }
        WM_DPICHANGED => {
            with_app(|app| ui::set_dpi(app, (wp & 0xFFFF) as u32));
            let r = unsafe { &*(lp as *const RECT) };
            unsafe {
                SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE)
            };
            0
        }
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
            with_app(ui::apply_theme);
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_CLOSE => {
            // With "Close to tray" on (the default) closing keeps CursorDrop in the tray, like
            // DS4-Native; with it off, closing the window quits.
            let mut to_tray = true;
            with_app(|app| to_tray = app.state.ui.close_to_tray);
            if !to_tray {
                unsafe { DestroyWindow(hwnd) };
                return 0;
            }
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
                KillTimer(hwnd, TIMER_ANIM);
            }
            with_app(|app| {
                if !app.ui.told_about_tray {
                    app.ui.told_about_tray = true;
                    balloon(app, "Still running in the tray - packs dropped into the cursors folder are applied automatically.", false);
                }
            });
            0
        }
        WM_DESTROY => {
            dragdrop::unregister(hwnd);
            with_app(|app| tray_remove(app));
            unsafe { PostQuitMessage(0) };
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.with(|c| c.get()) => {
            with_app(|app| tray_add(app)); // Explorer restarted
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn run(show: bool) {
    unsafe {
        let mutex_name = wide("Local\\CursorDrop.SingleInstance");
        CreateMutexW(null(), 1, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Already running: bring its window up instead.
            let class = wide(CLASS_NAME);
            let other = FindWindowW(class.as_ptr(), null());
            if !other.is_null() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(other, &mut pid);
                windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
                PostMessageW(other, WM_SHOW_WINDOW, 0, 0);
            }
            return;
        }

        gfx::startup();
        let hinst = GetModuleHandleW(null());
        // Our own icon (resource 1), at the tray's small size; the window gets both sizes.
        let load = |metric_x: SYSTEM_METRICS_INDEX, metric_y: SYSTEM_METRICS_INDEX| {
            LoadImageW(hinst, 1 as *const u16, IMAGE_ICON, GetSystemMetrics(metric_x), GetSystemMetrics(metric_y), 0) as HICON
        };
        let mut icon = load(SM_CXSMICON, SM_CYSMICON);
        let mut big = load(SM_CXICON, SM_CYICON);
        if icon.is_null() {
            icon = LoadIconW(null_mut(), IDI_APPLICATION);
            big = icon;
        }

        let class = wide(CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            hIcon: big,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);

        // Centre a 1100 x 720 (at 96 dpi) client area on the primary screen's work area.
        let dpi = GetDpiForSystem().max(96);
        let scale = dpi as f32 / 96.0;
        let mut rc = RECT { left: 0, top: 0, right: (CLIENT_W * scale) as i32, bottom: (CLIENT_H * scale) as i32 };
        AdjustWindowRectExForDpi(&mut rc, WS_OVERLAPPEDWINDOW, 0, 0, dpi);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let mut work = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0);
        let x = work.left + ((work.right - work.left) - ww).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - wh).max(0) / 2;

        let title = wide(APP_NAME);
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, x, y, ww, wh, null_mut(), null_mut(), hinst, null());
        TASKBAR_CREATED.with(|c| c.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr())));
        // OLE drag and drop shows the drop overlay while dragging; WM_DROPFILES is the fallback.
        if !dragdrop::register(hwnd, drag_event) {
            DragAcceptFiles(hwnd, 1);
        }
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);

        let mut app = App::new();
        app.hwnd = hwnd;
        app.icon = icon;
        app.ui.hwnd = hwnd;
        ui::set_dpi(&mut app, GetDpiForWindow(hwnd).max(96));
        ui::apply_theme(&mut app);
        if app.state.ui.on_top {
            ui::apply_on_top(&app);
        }
        app.log("started");
        app.startup();
        tray_add(&app);
        let first = app.scan(true);
        if let Some((m, err)) = first {
            balloon(&app, &m, err);
        } else if app.state.installed.is_empty() && !show {
            balloon(&app, "Drop a cursor pack (.zip, .7z or .rar) into the cursors folder and it will be applied automatically.", false);
        }
        refresh_tray(&app);
        APP.with(|a| *a.borrow_mut() = Some(app));

        SetTimer(hwnd, TIMER_SCAN, SCAN_MS, None);
        if show {
            with_app(show_window);
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Enter and Esc in the preset name box confirm / cancel the dialog.
            if msg.message == WM_KEYDOWN && !msg.hwnd.is_null() && msg.hwnd == ui::name_edit() && (msg.wParam == VK_RETURN as usize || msg.wParam == VK_ESCAPE as usize) {
                let vk = msg.wParam as u16;
                with_app(|app| ui::on_dialog_key(app, vk));
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---------------------------------------------------------------- command line

/// `CursorDrop.exe --inspect <archive>`: shows what would be applied, without changing anything.
fn inspect(path: &Path) -> i32 {
    let tmp = std::env::temp_dir().join(format!("cursordrop-inspect-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let code = match archive::extract(path, &tmp) {
        Err(e) => {
            println!("error: {e}");
            1
        }
        Ok(n) => {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let schemes = scheme::discover(&tmp, &stem);
            println!("{}: {n} cursor/inf file(s) extracted, {} scheme(s)", path.display(), schemes.len());
            for s in &schemes {
                println!("\n[{}]  {}/{} roles   (Windows scheme name: {})", s.display(), s.mapped(), ROLES.len(), s.name);
                for (i, f) in s.files.iter().enumerate() {
                    println!("  {:<22} {}", ROLE_LABELS[i], f.as_deref().unwrap_or("(Windows default)"));
                }
            }
            if schemes.is_empty() { 2 } else { 0 }
        }
    };
    let _ = fs::remove_dir_all(&tmp);
    code
}

/// `CURSORDROP_SELFTEST=<folder>`: installs the packs in the cursors folder without applying
/// them, renders each view to a PNG in <folder> and exits. No window, tray icon or cursor change.
fn self_test(out: &Path) {
    gfx::startup();
    let mut app = App::new();
    app.no_apply = true;
    app.scan(false);
    if let Ok(a) = std::env::var("CURSORDROP_SELFTEST_ACTIVE") {
        app.state.active = Some(Active::pack(&a, 0));
    }
    let _ = fs::create_dir_all(out);
    ui::self_test(&mut app, out);
}

fn main() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if let Ok(dir) = std::env::var("CURSORDROP_SELFTEST") {
        self_test(Path::new(&dir));
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--inspect") if args.len() >= 2 => std::process::exit(inspect(Path::new(&args[1]))),
        Some("--version") => println!("{APP_NAME} {VERSION}"),
        Some("--make-icon") if args.len() >= 2 => {
            if let Err(e) = gfx::make_icon(Path::new(&args[1])) {
                println!("error: {e}");
                std::process::exit(1);
            }
        }
        Some("--restore") => {
            let mut app = App::new();
            match app.restore() {
                Ok(()) => println!("original cursors restored"),
                Err(e) => {
                    println!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("--tray") => run(false),
        _ => run(true),
    }
}
