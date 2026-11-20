//! Choosing cursors by hand, and presets.
//!
//! Every cursor grid ("sheet") is a pack variant, the cursors in use right now ("Current"), or
//! a saved preset. Any of its 17 slots can be pointed at another cursor file (from the same
//! pack, another pack or a preset), told to keep whatever Windows uses now, or set to the
//! Windows default:
//! - on a pack variant the choice is an override kept per variant, so the pack itself stays
//!   as it came and "Reset changes" goes back to it;
//! - on Current it's a draft, meant to be saved as a preset;
//! - on a preset it changes the preset (its files are copied into the preset's own folder,
//!   so presets keep working when the packs they came from are removed).

use crate::scheme::{ROLE_LABELS, ROLES};
use crate::{Active, App, cursors};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A choice for one cursor slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path")]
pub enum Pick {
    /// A cursor file (absolute path; for presets, a file name inside the preset's folder).
    File(String),
    /// Leave this cursor as Windows has it when the sheet is used.
    Keep,
    /// Windows' built-in cursor.
    Default,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub created: u64,
    pub picks: Vec<Pick>,
    #[serde(default)]
    pub size: u64,
    /// The original file name of each cursor (preset files are renamed "NN Role.ext").
    #[serde(default)]
    pub from: Vec<String>,
}

/// Which cursor grid is meant.
#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    Pack { archive: String, variant: usize },
    Current,
    Preset { id: String },
}

/// What a slot comes to, ready to draw or apply.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    File(String),
    Keep,
    Default,
}

fn override_key(archive: &str, variant: usize) -> String {
    format!("{archive}\u{1}{variant}")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What Windows uses for a role right now, as stored (may contain %SystemRoot%; "" = built-in).
fn current_raw(role: usize) -> Option<String> {
    cursors::get_string(cursors::CURSORS_KEY, Some(ROLES[role])).filter(|v| !v.trim().is_empty())
}

impl App {
    pub fn presets_dir(&self) -> PathBuf {
        self.data_dir.join("presets")
    }

    pub fn preset(&self, id: &str) -> Option<&Preset> {
        self.state.presets.iter().find(|p| p.id == id)
    }

    pub fn presets(&self) -> &[Preset] {
        &self.state.presets
    }

    /// The slot as it comes out on a sheet, with the user's choices applied.
    pub fn slot(&self, sheet: &Sheet, role: usize) -> Slot {
        match sheet {
            Sheet::Pack { archive, variant } => {
                if let Some(p) = self.state.overrides.get(&override_key(archive, *variant)).and_then(|m| m.get(&role)) {
                    return self.pick_slot(p, None);
                }
                let base = self.find(archive).and_then(|i| i.schemes.get(*variant)).and_then(|s| s.files.get(role).cloned().flatten());
                match base {
                    Some(rel) => Slot::File(self.scheme_dir(archive).join(rel).to_string_lossy().to_string()),
                    None => Slot::Default,
                }
            }
            Sheet::Current => match self.state.current_draft.get(&role) {
                Some(p) => self.pick_slot(p, None),
                None => Slot::Keep,
            },
            Sheet::Preset { id } => match self.preset(id).and_then(|p| p.picks.get(role)) {
                Some(p) => self.pick_slot(p, Some(&self.presets_dir().join(id))),
                None => Slot::Default,
            },
        }
    }

    fn pick_slot(&self, pick: &Pick, base: Option<&Path>) -> Slot {
        match pick {
            Pick::File(f) => Slot::File(match base {
                Some(dir) => dir.join(f).to_string_lossy().to_string(),
                None => f.clone(),
            }),
            Pick::Keep => Slot::Keep,
            Pick::Default => Slot::Default,
        }
    }

    /// Whether a slot was changed by hand (as opposed to coming from the pack itself).
    pub fn slot_edited(&self, sheet: &Sheet, role: usize) -> bool {
        match sheet {
            Sheet::Pack { archive, variant } => {
                self.state.overrides.get(&override_key(archive, *variant)).is_some_and(|m| m.contains_key(&role))
            }
            Sheet::Current => self.state.current_draft.contains_key(&role),
            Sheet::Preset { .. } => false,
        }
    }

    /// How many slots on the sheet were changed by hand.
    pub fn edits(&self, sheet: &Sheet) -> usize {
        match sheet {
            Sheet::Pack { archive, variant } => self.state.overrides.get(&override_key(archive, *variant)).map_or(0, |m| m.len()),
            Sheet::Current => self.state.current_draft.len(),
            Sheet::Preset { .. } => 0,
        }
    }

    /// Sets a slot. `None` puts it back to what the sheet had on its own (pack's file, or for
    /// Current: no change).
    pub fn set_pick(&mut self, sheet: &Sheet, role: usize, pick: Option<Pick>) -> Result<(), String> {
        match sheet {
            Sheet::Pack { archive, variant } => {
                let key = override_key(archive, *variant);
                let base = self.find(archive).and_then(|i| i.schemes.get(*variant)).and_then(|s| s.files.get(role).cloned().flatten());
                let own = match &base {
                    Some(rel) => Pick::File(self.scheme_dir(archive).join(rel).to_string_lossy().to_string()),
                    None => Pick::Default,
                };
                let map = self.state.overrides.entry(key.clone()).or_default();
                match pick {
                    // Picking what the pack already has is the same as no change.
                    Some(p) if p != own => {
                        map.insert(role, p);
                    }
                    _ => {
                        map.remove(&role);
                    }
                }
                if map.is_empty() {
                    self.state.overrides.remove(&key);
                }
            }
            Sheet::Current => match pick {
                Some(Pick::Keep) | None => {
                    self.state.current_draft.remove(&role);
                }
                Some(p) => {
                    self.state.current_draft.insert(role, p);
                }
            },
            Sheet::Preset { id } => {
                let dir = self.presets_dir().join(id);
                let mut origin = String::new();
                let stored = match pick.unwrap_or(Pick::Default) {
                    Pick::File(src) => {
                        origin = self.origin_name(&src);
                        Pick::File(copy_into(&dir, role, Path::new(&src))?)
                    }
                    other => other,
                };
                let size = dir_size(&dir);
                let p = self.state.presets.iter_mut().find(|p| p.id == *id).ok_or("preset not found")?;
                if let Some(slot) = p.picks.get_mut(role) {
                    *slot = stored;
                }
                p.from.resize(ROLES.len(), String::new());
                p.from[role] = origin;
                p.size = size;
            }
        }
        self.save();
        Ok(())
    }

    pub fn reset_sheet(&mut self, sheet: &Sheet) {
        match sheet {
            Sheet::Pack { archive, variant } => {
                self.state.overrides.remove(&override_key(archive, *variant));
            }
            Sheet::Current => self.state.current_draft.clear(),
            Sheet::Preset { .. } => {}
        }
        self.save();
    }

    /// Forgets the choices made on a pack's variants (when the pack goes away or changes).
    pub fn forget_overrides(&mut self, archive: &str) {
        let prefix = format!("{archive}\u{1}");
        self.state.overrides.retain(|k, _| !k.starts_with(&prefix));
    }

    /// The registry values for applying a sheet: a path, or None for the Windows default.
    /// "Keep" slots get the value Windows has now.
    pub fn sheet_paths(&self, sheet: &Sheet) -> Vec<Option<String>> {
        (0..ROLES.len())
            .map(|role| match self.slot(sheet, role) {
                Slot::File(f) => Some(f),
                Slot::Keep => current_raw(role),
                Slot::Default => None,
            })
            .collect()
    }

    /// Saves a sheet as a new preset: every cursor it would use is copied into the preset.
    /// Returns the new preset's id.
    pub fn save_preset(&mut self, sheet: &Sheet, name: &str) -> Result<String, String> {
        let name = self.unique_preset_name(name.trim(), None);
        let id = format!("p{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let dir = self.presets_dir().join(&id);
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let mut picks = Vec::with_capacity(ROLES.len());
        let mut from = Vec::with_capacity(ROLES.len());
        for role in 0..ROLES.len() {
            let source = match self.slot(sheet, role) {
                Slot::File(f) => Some(f),
                Slot::Keep => current_raw(role).map(|v| expand(&v)),
                Slot::Default => None,
            };
            let pick = match &source {
                Some(src) if Path::new(src).is_file() => Pick::File(copy_into(&dir, role, Path::new(src))?),
                _ => Pick::Default,
            };
            from.push(match (&pick, &source) {
                (Pick::File(_), Some(src)) => self.origin_name(src),
                _ => String::new(),
            });
            picks.push(pick);
        }
        let size = dir_size(&dir);
        self.state.presets.push(Preset { id: id.clone(), name: name.clone(), created: now_secs(), picks, size, from });
        if *sheet == Sheet::Current {
            self.state.current_draft.clear();
        }
        self.save();
        self.log(&format!("saved preset '{name}'"));
        Ok(id)
    }

    pub fn rename_preset(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = self.unique_preset_name(name.trim(), Some(id));
        let was_current = self.ours_is_current();
        let p = self.state.presets.iter_mut().find(|p| p.id == id).ok_or("preset not found")?;
        let old = std::mem::replace(&mut p.name, name);
        if !self.no_apply {
            cursors::unregister_scheme(&old);
        }
        self.save();
        // A preset in use is re-applied so Windows knows it by its new name.
        if was_current && self.state.active.as_ref().is_some_and(|a| a.preset.as_deref() == Some(id)) {
            let target = self.state.active.clone().unwrap();
            self.apply_inner(target, false)?;
        }
        Ok(())
    }

    pub fn delete_preset(&mut self, id: &str) {
        let Some(p) = self.preset(id).cloned() else { return };
        if self.state.active.as_ref().is_some_and(|a| a.preset.as_deref() == Some(id)) {
            if self.ours_is_current() {
                let _ = self.restore();
            }
            self.state.active = None;
        }
        if !self.no_apply {
            cursors::unregister_scheme(&p.name);
        }
        let _ = fs::remove_dir_all(self.presets_dir().join(id));
        self.state.presets.retain(|p| p.id != id);
        self.save();
        self.log(&format!("deleted preset '{}'", p.name));
    }

    /// A cursor file's original name, also for files that already sit in a preset.
    pub fn origin_name(&self, src: &str) -> String {
        let path = Path::new(src);
        let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        if path.starts_with(self.presets_dir()) {
            let id = path.parent().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let role: Option<usize> = file.get(..2).and_then(|n| n.parse().ok());
            if let (Some(p), Some(r)) = (self.preset(&id), role) {
                if let Some(name) = p.from.get(r).filter(|n| !n.is_empty()) {
                    return name.clone();
                }
            }
        }
        crate::scheme::fix_mojibake(&file)
    }

    fn unique_preset_name(&self, wanted: &str, except: Option<&str>) -> String {
        let base = if wanted.is_empty() { "My cursors".to_string() } else { wanted.replace(',', " ") };
        let taken = |n: &str| self.state.presets.iter().any(|p| Some(p.id.as_str()) != except && p.name.eq_ignore_ascii_case(n));
        if !taken(&base) {
            return base;
        }
        (2..).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).unwrap()
    }

    /// The scheme name Windows knows the active scheme or preset by.
    pub fn active_name(&self) -> Option<String> {
        let a = self.state.active.as_ref()?;
        match &a.preset {
            Some(id) => self.preset(id).map(|p| p.name.clone()),
            None => self.find(&a.archive)?.schemes.get(a.scheme).map(|s| s.name.clone()),
        }
    }

    /// The name shown for the active scheme or preset.
    pub fn active_display(&self) -> Option<String> {
        let a = self.state.active.as_ref()?;
        match &a.preset {
            Some(id) => self.preset(id).map(|p| p.name.clone()),
            None => self.find(&a.archive)?.schemes.get(a.scheme).map(|s| s.display().to_string()),
        }
    }
}

impl Active {
    pub fn pack(archive: &str, scheme: usize) -> Active {
        Active { archive: archive.to_string(), scheme, preset: None }
    }

    pub fn preset(id: &str) -> Active {
        Active { archive: String::new(), scheme: 0, preset: Some(id.to_string()) }
    }

    pub fn sheet(&self) -> Sheet {
        match &self.preset {
            Some(id) => Sheet::Preset { id: id.clone() },
            None => Sheet::Pack { archive: self.archive.clone(), variant: self.scheme },
        }
    }
}

fn expand(s: &str) -> String {
    let w = cursors::wide(s);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW(w.as_ptr(), buf.as_mut_ptr(), 1024) };
    if n == 0 || n > 1024 { s.to_string() } else { String::from_utf16_lossy(&buf[..(n - 1) as usize]) }
}

/// Copies a cursor into a preset folder as "NN Role.ext" (replacing that role's old file).
fn copy_into(dir: &Path, role: usize, src: &Path) -> Result<String, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let prefix = format!("{role:02} ");
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let ext = src.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "cur".into());
    let name = format!("{prefix}{}.{ext}", ROLE_LABELS[role]);
    fs::copy(src, dir.join(&name)).map_err(|e| format!("could not copy {}: {e}", src.display()))?;
    Ok(name)
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Cursor files on offer in the picker, from one pack or preset.
pub struct PickSource {
    pub label: String,
    pub detail: String,
    /// Folder label (empty for the top) and its files.
    pub groups: Vec<(String, Vec<PickFile>)>,
}

pub struct PickFile {
    pub path: String,
    pub name: String,
    /// The file's name says it's this kind of cursor.
    pub suggested: bool,
}

impl PickSource {
    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.1.len()).sum()
    }

    pub fn file(&self, mut i: usize) -> Option<&PickFile> {
        for (_, files) in &self.groups {
            if i < files.len() {
                return files.get(i);
            }
            i -= files.len();
        }
        None
    }
}

fn walk_cursors(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort_by(|a, b| {
        crate::scheme::natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy())
    });
    for p in entries {
        if p.is_dir() {
            walk_cursors(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cur") || e.eq_ignore_ascii_case("ani")) {
            out.push(p);
        }
    }
}

impl App {
    /// Everything the picker offers for `role` on `sheet`: the sheet's own pack first (its
    /// variant's folder first), then the other packs, then the presets.
    pub fn pick_sources(&self, sheet: &Sheet, role: usize) -> Vec<PickSource> {
        let mut out = Vec::new();
        let first = match sheet {
            Sheet::Pack { archive, .. } => Some(archive.clone()),
            _ => None,
        };
        let variant_dir = match sheet {
            Sheet::Pack { archive, variant } => self
                .find(archive)
                .and_then(|i| i.schemes.get(*variant))
                .and_then(|s| s.files.iter().flatten().next().cloned())
                .and_then(|rel| Path::new(&rel).parent().map(|p| p.to_string_lossy().to_string())),
            _ => None,
        };
        let mut packs: Vec<&crate::Installed> = self.state.installed.iter().filter(|i| i.error.is_none()).collect();
        packs.sort_by_key(|i| (Some(&i.archive) != first.as_ref(), i.archive.to_lowercase()));
        for inst in packs {
            let dir = self.scheme_dir(&inst.archive);
            let mut files = Vec::new();
            walk_cursors(&dir, &mut files);
            let mut groups: Vec<(String, Vec<PickFile>)> = Vec::new();
            for f in files {
                let rel_dir = f.parent().and_then(|p| p.strip_prefix(&dir).ok()).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                let stem = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let file = PickFile {
                    path: f.to_string_lossy().to_string(),
                    name: crate::scheme::fix_mojibake(&f.file_name().unwrap_or_default().to_string_lossy()),
                    suggested: crate::scheme::classify(&stem) == Some(role),
                };
                match groups.iter_mut().find(|g| g.0 == rel_dir) {
                    Some(g) => g.1.push(file),
                    None => groups.push((rel_dir, vec![file])),
                }
            }
            // The variant's own folder first; within a folder, likely matches first.
            if let Some(vd) = &variant_dir {
                if Some(&inst.archive) == first.as_ref() {
                    groups.sort_by_key(|g| g.0 != *vd);
                }
            }
            for g in &mut groups {
                g.1.sort_by_key(|f| !f.suggested);
            }
            let groups: Vec<(String, Vec<PickFile>)> = groups
                .into_iter()
                .map(|(d, f)| (d.split(['\\', '/']).map(crate::scheme::fix_mojibake).collect::<Vec<_>>().join(" / "), f))
                .collect();
            let n: usize = groups.iter().map(|g| g.1.len()).sum();
            if n == 0 {
                continue;
            }
            let label = Path::new(&inst.archive).file_stem().unwrap_or_default().to_string_lossy().to_string();
            let detail = if Some(&inst.archive) == first.as_ref() { format!("This pack \u{00B7} {n} cursors") } else { format!("{n} cursors") };
            out.push(PickSource { label, detail, groups });
        }
        for p in &self.state.presets {
            if *sheet == (Sheet::Preset { id: p.id.clone() }) {
                continue;
            }
            let dir = self.presets_dir().join(&p.id);
            let files: Vec<PickFile> = p
                .picks
                .iter()
                .enumerate()
                .filter_map(|(r, pick)| match pick {
                    Pick::File(f) => Some(PickFile { path: dir.join(f).to_string_lossy().to_string(), name: ROLE_LABELS[r].to_string(), suggested: r == role }),
                    _ => None,
                })
                .collect();
            if files.is_empty() {
                continue;
            }
            let mut files = files;
            files.sort_by_key(|f| !f.suggested);
            out.push(PickSource { label: p.name.clone(), detail: format!("Preset \u{00B7} {} cursors", files.len()), groups: vec![(String::new(), files)] });
        }
        out
    }
}
