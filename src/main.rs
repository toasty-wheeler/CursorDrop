#![windows_subsystem = "windows"]
//! CursorDrop: drop a cursor-pack archive (.zip / .7z / .rar) into the `cursors` folder next to
//! the exe and it becomes your Windows cursor scheme. Lives in the system tray.

mod archive;
mod cursors;
mod scheme;

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
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const APP_NAME: &str = "CursorDrop";
const WM_TRAY: u32 = WM_APP + 1;
const TIMER_SCAN: usize = 1;
const SCAN_MS: u32 = 2000;

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
}

#[derive(Clone, Serialize, Deserialize)]
struct Installed {
    archive: String,
    size: u64,
    mtime: u64,
    schemes: Vec<Scheme>,
    error: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Active {
    archive: String,
    scheme: usize,
}

struct App {
    exe: PathBuf,
    drop_dir: PathBuf,
    data_dir: PathBuf,
    state: State,
    pending: HashMap<String, (u64, u64)>,
    hwnd: HWND,
    icon: HICON,
    menu_map: Vec<Active>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn file_sig(p: &Path) -> Option<(u64, u64)> {
    let m = fs::metadata(p).ok()?;
    let mtime = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), mtime))
}

impl App {
    fn new() -> App {
        let exe = std::env::current_exe().expect("exe path");
        let base = exe.parent().unwrap().to_path_buf();
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
        }
    }

    fn save(&self) {
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

    fn scheme_dir(&self, archive: &str) -> PathBuf {
        self.data_dir.join("schemes").join(archive)
    }

    fn find(&self, archive: &str) -> Option<&Installed> {
        self.state.installed.iter().find(|i| i.archive == archive)
    }

    fn active_scheme(&self) -> Option<&Scheme> {
        let a = self.state.active.as_ref()?;
        self.find(&a.archive)?.schemes.get(a.scheme)
    }

    /// True if the cursors Windows is using right now are the scheme we applied.
    fn ours_is_current(&self) -> bool {
        match self.active_scheme() {
            Some(s) => cursors::current_scheme_name().is_some_and(|n| n == s.name),
            None => false,
        }
    }

    fn apply(&mut self, target: Active) -> Result<String, String> {
        let inst = self.find(&target.archive).ok_or("archive not installed")?;
        let scheme = inst.schemes.get(target.scheme).ok_or("no such scheme")?.clone();
        let dir = self.scheme_dir(&target.archive);
        let paths: Vec<Option<String>> = scheme
            .files
            .iter()
            .map(|f| f.as_ref().map(|rel| dir.join(rel).to_string_lossy().to_string()))
            .collect();
        if !self.ours_is_current() {
            // Remember what the user had so "Restore original cursors" can bring it back.
            self.state.backup = Some(cursors::take_backup());
        }
        cursors::apply(&scheme.name, &paths)?;
        self.state.active = Some(target);
        self.save();
        self.log(&format!("applied '{}' ({} of {} cursors)", scheme.name, scheme.mapped(), ROLES.len()));
        Ok(scheme.name)
    }

    fn restore(&mut self) -> Result<(), String> {
        let b = self.state.backup.clone().ok_or("nothing to restore")?;
        cursors::restore(&b)?;
        self.state.active = None;
        self.save();
        self.log("restored original cursors");
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

        // Archives that were deleted from the folder.
        let gone: Vec<Installed> = self
            .state
            .installed
            .iter()
            .filter(|i| !present.iter().any(|(n, _)| *n == i.archive))
            .cloned()
            .collect();
        for inst in gone {
            let was_active = self.state.active.as_ref().is_some_and(|a| a.archive == inst.archive);
            if was_active && self.ours_is_current() {
                let _ = self.restore();
                note = Some((format!("{} was removed, original cursors restored.", inst.archive), false));
            }
            if was_active {
                self.state.active = None;
            }
            for s in &inst.schemes {
                cursors::unregister_scheme(&s.name);
            }
            let _ = fs::remove_dir_all(self.scheme_dir(&inst.archive));
            self.state.installed.retain(|i| i.archive != inst.archive);
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
            self.pending.remove(&name);
            let inst = self.install(&name, sig);
            match &inst.error {
                Some(e) => {
                    self.log(&format!("{name}: {e}"));
                    note = Some((format!("{name}: {e}"), true));
                }
                None => {
                    self.log(&format!("installed {name}: {} scheme(s)", inst.schemes.len()));
                    to_apply = Some(Active { archive: name.clone(), scheme: 0 });
                }
            }
            for old in self.find(&name).map(|i| i.schemes.clone()).unwrap_or_default() {
                cursors::unregister_scheme(&old.name);
            }
            self.state.installed.retain(|i| i.archive != name);
            self.state.installed.push(inst);
            self.save();
        }
        if let Some(t) = to_apply {
            let extra = self.find(&t.archive).map_or(0, |i| i.schemes.len().saturating_sub(1));
            match self.apply(t) {
                Ok(n) => {
                    let more = if extra > 0 { format!(" ({extra} more variant(s) in the tray menu)") } else { String::new() };
                    note = Some((format!("Now using \"{n}\"{more}"), false));
                }
                Err(e) => note = Some((format!("Could not apply: {e}"), true)),
            }
        }
        note
    }

    fn startup(&mut self) {
        // If the cursors in use are ours, re-apply so paths stay valid even if the folder moved.
        if let Some(a) = self.state.active.clone() {
            if self.ours_is_current() {
                let _ = self.apply(a);
            } else {
                self.state.active = None;
                self.save();
            }
        }
    }

    fn startup_enabled(&self) -> bool {
        cursors::get_string(cursors::RUN_KEY, Some(APP_NAME)).is_some()
    }

    fn toggle_startup(&self) {
        if self.startup_enabled() {
            cursors::delete_value(cursors::RUN_KEY, APP_NAME);
        } else {
            let _ = cursors::set_string(cursors::RUN_KEY, Some(APP_NAME), &format!("\"{}\"", self.exe.display()), false);
        }
    }
}

// ---------------------------------------------------------------- tray UI

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
    match app.active_scheme() {
        Some(s) if app.ours_is_current() => format!("{APP_NAME} - {}", s.name),
        _ => format!("{APP_NAME} - drop cursor packs into the cursors folder"),
    }
}

fn tray_update_tip(app: &App) {
    let mut nid = tray_data(app);
    nid.uFlags = NIF_TIP;
    copy_wide(&mut nid.szTip, &tooltip(app));
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

fn balloon(app: &App, text: &str, error: bool) {
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

        match app.active_scheme() {
            Some(s) if current => append(menu, MF_STRING | MF_GRAYED, 0, &format!("Active: {}", s.name)),
            _ => append(menu, MF_STRING | MF_GRAYED, 0, "Active: (not a CursorDrop scheme)"),
        }
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
                let t = Active { archive: inst.archive.clone(), scheme: i };
                let checked = if active.as_ref() == Some(&t) { MF_CHECKED } else { MF_UNCHECKED };
                let label = if s.mapped() < ROLES.len() {
                    format!("{}   ({}/{})", s.name, s.mapped(), ROLES.len())
                } else {
                    s.name.clone()
                };
                append(target, MF_STRING | checked, CMD_SCHEME_BASE + app.menu_map.len(), &label);
                app.menu_map.push(t);
            }
            if inst.schemes.len() > 1 {
                let w = wide(&inst.archive.replace('&', "&&"));
                AppendMenuW(menu, MF_POPUP, target as usize, w.as_ptr());
            }
        }

        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_OPEN, "Open cursors folder");
        append(menu, MF_STRING, CMD_RESCAN, "Rescan now");
        let can_restore = current && app.state.backup.is_some();
        append(menu, MF_STRING | if can_restore { 0 } else { MF_GRAYED }, CMD_RESTORE, "Restore original cursors");
        let st = if app.startup_enabled() { MF_CHECKED } else { MF_UNCHECKED };
        append(menu, MF_STRING | st, CMD_STARTUP, "Start with Windows");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_EXIT, "Exit");

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(app.hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            app.hwnd,
            null(),
        ) as usize;
        PostMessageW(app.hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu); // also destroys submenus
        if cmd != 0 {
            run_command(app, cmd);
        }
    }
}

fn open_folder(app: &App) {
    let op = wide("open");
    let path = wide(&app.drop_dir.to_string_lossy());
    unsafe { ShellExecuteW(null_mut(), op.as_ptr(), path.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

fn run_command(app: &mut App, cmd: usize) {
    match cmd {
        CMD_OPEN => open_folder(app),
        CMD_RESCAN => {
            if let Some((msg, err)) = app.scan(false) {
                balloon(app, &msg, err);
            }
        }
        CMD_RESTORE => match app.restore() {
            Ok(()) => balloon(app, "Original cursors restored.", false),
            Err(e) => balloon(app, &e, true),
        },
        CMD_STARTUP => app.toggle_startup(),
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
    tray_update_tip(app);
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|a| {
        if let Ok(mut guard) = a.try_borrow_mut() {
            if let Some(app) = guard.as_mut() {
                f(app);
            }
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp == TIMER_SCAN => {
            with_app(|app| {
                if let Some((m, err)) = app.scan(true) {
                    balloon(app, &m, err);
                }
                tray_update_tip(app);
            });
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_RBUTTONUP | WM_LBUTTONUP => with_app(show_menu),
                WM_LBUTTONDBLCLK => with_app(|app| open_folder(app)),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
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

fn run_tray() {
    unsafe {
        let mutex_name = wide("Local\\CursorDrop.SingleInstance");
        CreateMutexW(null(), 1, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let t = wide(APP_NAME);
            let m = wide("CursorDrop is already running - look for its icon in the system tray.");
            MessageBoxW(null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONINFORMATION);
            return;
        }

        let hinst = GetModuleHandleW(null());
        let class = wide("CursorDropWnd");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let title = wide(APP_NAME);
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        TASKBAR_CREATED.with(|c| c.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr())));

        // The mouse icon from the Mouse control panel; fall back to the generic app icon.
        let cpl = wide("main.cpl");
        let mut icon = ExtractIconW(hinst, cpl.as_ptr(), 0);
        if (icon as usize) <= 1 {
            icon = LoadIconW(null_mut(), IDI_APPLICATION);
        }

        let mut app = App::new();
        app.hwnd = hwnd;
        app.icon = icon;
        app.log("started");
        app.startup();
        tray_add(&app);
        let first = app.scan(true);
        if let Some((m, err)) = first {
            balloon(&app, &m, err);
        } else if app.state.installed.is_empty() {
            balloon(&app, "Drop a cursor pack (.zip, .7z or .rar) into the cursors folder and it will be applied automatically.", false);
        }
        tray_update_tip(&app);
        APP.with(|a| *a.borrow_mut() = Some(app));

        SetTimer(hwnd, TIMER_SCAN, SCAN_MS, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
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
                println!("\n[{}]  {}/{} roles", s.name, s.mapped(), ROLES.len());
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--inspect") if args.len() >= 2 => std::process::exit(inspect(Path::new(&args[1]))),
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
        _ => run_tray(),
    }
}
