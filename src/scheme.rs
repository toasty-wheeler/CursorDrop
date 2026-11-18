//! Works out which cursor file goes to which Windows cursor role.
//! Uses the pack's install .inf when it has one, otherwise guesses from file names.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Registry value names under HKCU\Control Panel\Cursors, in the order Windows uses for schemes.
pub const ROLES: [&str; 17] = [
    "Arrow", "Help", "AppStarting", "Wait", "Crosshair", "IBeam", "NWPen", "No", "SizeNS",
    "SizeWE", "SizeNWSE", "SizeNESW", "SizeAll", "UpArrow", "Hand", "Pin", "Person",
];

pub const ROLE_LABELS: [&str; 17] = [
    "Normal select", "Help select", "Working in background", "Busy", "Precision select",
    "Text select", "Handwriting", "Unavailable", "Vertical resize", "Horizontal resize",
    "Diagonal resize 1", "Diagonal resize 2", "Move", "Alternate select", "Link select",
    "Location select", "Person select",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scheme {
    pub name: String,
    /// One entry per ROLES slot: path relative to the extraction folder.
    pub files: Vec<Option<String>>,
}

impl Scheme {
    pub fn mapped(&self) -> usize {
        self.files.iter().filter(|f| f.is_some()).count()
    }
}

fn is_cursor(p: &Path) -> bool {
    matches!(
        p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref(),
        Some("cur") | Some("ani")
    )
}

/// Compares names the way Explorer sorts them: case-insensitive, numbers by value ("v2" < "v10").
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, text)) if *d == digit => text.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for ((da, ta), (db, tb)) in ca.iter().zip(cb.iter()) {
        let o = if *da && *db {
            let (na, nb) = (ta.trim_start_matches('0'), tb.trim_start_matches('0'));
            na.len().cmp(&nb.len()).then_with(|| na.cmp(nb))
        } else {
            ta.to_lowercase().cmp(&tb.to_lowercase())
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort_by(|a, b| natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn rel_string(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('/', "\\")
}

/// Finds every usable cursor scheme inside an extracted archive folder.
pub fn discover(root: &Path, fallback_name: &str) -> Vec<Scheme> {
    let mut all = Vec::new();
    walk(root, &mut all);
    let cursors: Vec<PathBuf> = all.iter().filter(|p| is_cursor(p)).cloned().collect();
    let infs: Vec<PathBuf> = all
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf")))
        .cloned()
        .collect();

    let mut schemes = Vec::new();
    let mut covered_dirs: HashSet<PathBuf> = HashSet::new();

    // 1) Packs with an install .inf say exactly which file is which.
    for inf in &infs {
        let Some(text) = read_text(inf) else { continue };
        let Some((name, names)) = parse_inf(&text) else { continue };
        let inf_dir = inf.parent().unwrap_or(root);
        let mut files = vec![None; ROLES.len()];
        for (i, fname) in names.iter().enumerate() {
            let Some(fname) = fname else { continue };
            let found = find_file(&cursors, inf_dir, fname).or_else(|| find_file(&cursors, root, fname));
            if let Some(p) = found {
                if let Some(d) = p.parent() {
                    covered_dirs.insert(d.to_path_buf());
                }
                files[i] = Some(rel_string(root, &p));
            }
        }
        let s = Scheme {
            name: name.unwrap_or_else(|| inf.file_stem().unwrap().to_string_lossy().to_string()),
            files,
        };
        if usable(&s) {
            covered_dirs.insert(inf_dir.to_path_buf());
            schemes.push(s);
        }
    }

    // 2) Folders of cursors without (working) .inf: guess from file names.
    let mut by_dir: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for c in &cursors {
        let d = c.parent().unwrap_or(root).to_path_buf();
        match by_dir.iter_mut().find(|(k, _)| *k == d) {
            Some((_, v)) => v.push(c.clone()),
            None => by_dir.push((d, vec![c.clone()])),
        }
    }
    let mut heuristic_count = 0;
    for (dir, files_in_dir) in by_dir {
        if covered_dirs.contains(&dir) {
            continue;
        }
        let mut files = vec![None; ROLES.len()];
        for f in &files_in_dir {
            let stem = f.file_stem().unwrap_or_default().to_string_lossy();
            if let Some(i) = classify(&stem) {
                if files[i].is_none() {
                    files[i] = Some(rel_string(root, f));
                }
            }
        }
        // Variant folders are named after their own folder ("Pack - Dark"); fixed up below
        // when the archive turns out to hold just one scheme.
        let name = match dir.strip_prefix(root).ok().and_then(|r| r.file_name()) {
            Some(leaf) => format!("{fallback_name} - {}", leaf.to_string_lossy()),
            None => fallback_name.to_string(),
        };
        let s = Scheme { name, files };
        if usable(&s) {
            heuristic_count += 1;
            schemes.push(s);
        }
    }
    if schemes.len() == 1 && heuristic_count == 1 {
        schemes[0].name = fallback_name.to_string();
    }

    // Unique names (the name is also used as the Control Panel scheme name).
    let mut seen: HashMap<String, usize> = HashMap::new();
    for s in &mut schemes {
        s.name = s.name.replace(',', " ").trim().to_string();
        if s.name.is_empty() {
            s.name = fallback_name.to_string();
        }
        let n = seen.entry(s.name.to_lowercase()).or_insert(0);
        *n += 1;
        if *n > 1 {
            s.name = format!("{} ({})", s.name, n);
        }
    }
    schemes
}

fn usable(s: &Scheme) -> bool {
    s.files[0].is_some() || s.mapped() >= 3
}

/// Case-insensitive file-name lookup, restricted to files under `under`.
fn find_file(cursors: &[PathBuf], under: &Path, name: &str) -> Option<PathBuf> {
    cursors
        .iter()
        .filter(|p| p.starts_with(under))
        .find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name)))
        .cloned()
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return Some(String::from_utf16_lossy(&u));
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    match std::str::from_utf8(b) {
        Ok(s) => Some(s.to_string()),
        Err(_) => Some(b.iter().map(|&c| c as char).collect()), // Latin-1-ish fallback
    }
}

/// Splits an INF line on commas outside quotes, dropping quotes and trailing comments.
fn split_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            ';' if !quoted => break,
            _ => cur.push(ch),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn substitute(s: &str, strings: &HashMap<String, String>) -> String {
    let mut s = s.to_string();
    for _ in 0..6 {
        let mut out = String::new();
        let mut rest = s.as_str();
        let mut changed = false;
        while let Some(a) = rest.find('%') {
            out.push_str(&rest[..a]);
            let after = &rest[a + 1..];
            let Some(b) = after.find('%') else {
                out.push_str(&rest[a..]);
                rest = "";
                break;
            };
            let key = &after[..b];
            if key.is_empty() {
                out.push('%');
            } else if key.chars().all(|c| c.is_ascii_digit()) {
                changed = true; // DIRID such as %10% (the Windows folder): irrelevant to us
            } else if let Some(v) = strings.get(&key.to_lowercase()) {
                out.push_str(v);
                changed = true;
            } else {
                out.push('%');
                out.push_str(key);
                out.push('%');
            }
            rest = &after[b + 1..];
        }
        out.push_str(rest);
        s = out;
        if !changed {
            break;
        }
    }
    s
}

fn base_name(s: &str) -> Option<String> {
    let s = s.trim().trim_matches('"');
    let b = s.rsplit(['\\', '/']).next().unwrap_or(s).trim();
    if b.is_empty() { None } else { Some(b.to_string()) }
}

/// Returns (scheme name, file name per role) if the INF describes a cursor scheme.
pub fn parse_inf(text: &str) -> Option<(Option<String>, Vec<Option<String>>)> {
    let mut section = String::new();
    let mut strings: HashMap<String, String> = HashMap::new();
    let mut reg_lines: Vec<Vec<String>> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).trim().to_lowercase();
            continue;
        }
        if section == "strings" || section.starts_with("strings.") {
            if let Some((k, v)) = line.split_once('=') {
                let v = split_fields(v).into_iter().next().unwrap_or_default();
                strings.entry(k.trim().to_lowercase()).or_insert(v);
            }
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("HKCU") || upper.starts_with("HKEY_CURRENT_USER") {
            reg_lines.push(split_fields(line));
        }
    }

    let mut files: Vec<Option<String>> = vec![None; ROLES.len()];
    let mut name = strings.get("scheme_name").cloned();
    let mut found = false;

    for f in &reg_lines {
        if f.len() < 5 {
            continue;
        }
        let key = substitute(&f[1], &strings).to_lowercase().replace('/', "\\");
        let key = key.trim_end_matches('\\');
        let vname = substitute(&f[2], &strings);
        let value = substitute(&f[4..].join(","), &strings);
        if key.ends_with("control panel\\cursors\\schemes") {
            found = true;
            if name.is_none() && !vname.is_empty() {
                name = Some(vname.clone());
            }
            for (i, item) in value.split(',').enumerate().take(ROLES.len()) {
                if files[i].is_none() {
                    files[i] = base_name(item);
                }
            }
        } else if key.ends_with("control panel\\cursors") {
            if vname.is_empty() {
                if name.is_none() && !value.trim().is_empty() {
                    name = Some(value.trim().to_string());
                }
            } else if let Some(i) = ROLES.iter().position(|r| r.eq_ignore_ascii_case(vname.trim())) {
                found = true;
                if files[i].is_none() {
                    files[i] = base_name(&value);
                }
            }
        }
    }
    if !found {
        return None;
    }
    // Ignore unresolved %vars% that slipped through.
    for f in files.iter_mut() {
        if f.as_ref().is_some_and(|s| s.contains('%')) {
            *f = None;
        }
    }
    Some((name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()), files))
}

/// Splits a file stem into lowercase words: on punctuation, letter/digit edges and camelCase.
fn tokens(stem: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for ch in stem.chars() {
        if !ch.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let boundary = (p.is_lowercase() && ch.is_uppercase())
                || (p.is_alphabetic() && ch.is_ascii_digit())
                || (p.is_ascii_digit() && ch.is_alphabetic());
            if boundary && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(ch.to_lowercase());
        prev = Some(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Guesses the cursor role from a file name. Order matters: more specific roles first.
pub fn classify(stem: &str) -> Option<usize> {
    const RULES: &[(&str, &[&str])] = &[
        ("AppStarting", &["appstart", "background", "working", "work", "progress", "bgbusy"]),
        ("Wait", &["busy", "wait", "loading", "hourglass", "load"]),
        ("Help", &["help", "question"]),
        ("Crosshair", &["precision", "crosshair", "cross", "precise"]),
        ("IBeam", &["ibeam", "beam", "text", "caret"]),
        ("NWPen", &["handwrit", "pen", "nwpen", "write", "writing"]),
        ("SizeNWSE", &["nwse", "dgn1", "diag1", "diagonal1", "diagonalresize1", "diagonal 1", "sizenwse"]),
        ("SizeNESW", &["nesw", "dgn2", "diag2", "diagonal2", "diagonalresize2", "sizenesw"]),
        ("SizeNS", &["vertical", "vert", "sizens", "ns", "updown"]),
        ("SizeWE", &["horizontal", "horz", "horiz", "sizewe", "ew", "we", "leftright"]),
        ("SizeAll", &["move", "sizeall", "all", "fourway"]),
        ("No", &["unavail", "notallowed", "forbidden", "no", "denied", "blocked", "unavailable"]),
        ("UpArrow", &["alternate", "alt", "uparrow", "up"]),
        ("Hand", &["link", "hand", "finger"]),
        ("Pin", &["pin", "location"]),
        ("Person", &["person"]),
        ("Arrow", &["normal", "arrow", "pointer", "default", "select", "cursor", "main"]),
    ];
    let toks = tokens(stem);
    let joined: String = toks.concat();
    for (role, keys) in RULES {
        let hit = keys.iter().any(|k| {
            let k_joined: String = k.split(' ').collect();
            if k_joined.len() <= 3 {
                toks.iter().any(|t| *t == k_joined)
            } else {
                joined.contains(&k_joined)
            }
        });
        if hit {
            return ROLES.iter().position(|r| r == role);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(stem: &str) -> &'static str {
        classify(stem).map(|i| ROLES[i]).unwrap_or("-")
    }

    #[test]
    fn heuristics() {
        let cases = [
            ("Normal Select", "Arrow"), ("Help Select", "Help"), ("Working in Background", "AppStarting"),
            ("Busy", "Wait"), ("Precision Select", "Crosshair"), ("Text Select", "IBeam"),
            ("Handwriting", "NWPen"), ("Unavailable", "No"), ("Vertical Resize", "SizeNS"),
            ("Horizontal Resize", "SizeWE"), ("Diagonal Resize 1", "SizeNWSE"),
            ("Diagonal Resize 2", "SizeNESW"), ("Move", "SizeAll"), ("Alternate Select", "UpArrow"),
            ("Link Select", "Hand"), ("Location Select", "Pin"), ("Person Select", "Person"),
            ("aero_arrow", "Arrow"), ("aero_helpsel", "Help"), ("aero_working", "AppStarting"),
            ("aero_busy", "Wait"), ("aero_unavail", "No"), ("aero_ns", "SizeNS"), ("aero_ew", "SizeWE"),
            ("aero_nwse", "SizeNWSE"), ("aero_nesw", "SizeNESW"), ("aero_move", "SizeAll"),
            ("aero_up", "UpArrow"), ("aero_link", "Hand"), ("aero_pen", "NWPen"), ("aero_pin", "Pin"),
            ("aero_person", "Person"), ("beam_r", "IBeam"), ("cross_r", "Crosshair"),
            ("SizeNS", "SizeNS"), ("SizeWE", "SizeWE"), ("SizeAll", "SizeAll"), ("IBeam", "IBeam"),
            ("01-Normal", "Arrow"), ("dgn1", "SizeNWSE"), ("horz", "SizeWE"), ("unavailiable", "No"),
        ];
        for (stem, want) in cases {
            assert_eq!(role(stem), want, "{stem}");
        }
    }

    #[test]
    fn inf_scheme_line() {
        let inf = r#"
[Version]
signature="$CHICAGO$"
[DefaultInstall]
CopyFiles = Scheme.Cur
AddReg    = Scheme.Reg

[Scheme.Reg]
HKCU,"Control Panel\Cursors\Schemes","%SCHEME_NAME%",,"%10%\%CUR_DIR%\%pointer%,%10%\%CUR_DIR%\%help%,%10%\%CUR_DIR%\%work%,%10%\%CUR_DIR%\%busy%,%10%\%CUR_DIR%\%cross%,%10%\%CUR_DIR%\%Text%,%10%\%CUR_DIR%\%Hand%,%10%\%CUR_DIR%\%unavailiable%,%10%\%CUR_DIR%\%Vert%,%10%\%CUR_DIR%\%Horz%,%10%\%CUR_DIR%\%Dgn1%,%10%\%CUR_DIR%\%Dgn2%,%10%\%CUR_DIR%\%move%,%10%\%CUR_DIR%\%alternate%,%10%\%CUR_DIR%\%link%"

[Strings]
CUR_DIR       = "Cursors\Fancy Pack"
SCHEME_NAME   = "Fancy Pack"
pointer       = "Normal.ani"
help          = "Help.cur"
work          = "Working.ani"
busy          = "Busy.ani"
cross         = "Precision.cur"
text          = "Text.cur"
hand          = "Handwriting.cur"
unavailiable  = "Unavailable.cur"
vert          = "Vertical.cur"
horz          = "Horizontal.cur"
dgn1          = "Diagonal1.cur"
dgn2          = "Diagonal2.cur"
move          = "Move.cur"
alternate     = "Alternate.cur"
link          = "Link.ani"
"#;
        let (name, files) = parse_inf(inf).unwrap();
        assert_eq!(name.as_deref(), Some("Fancy Pack"));
        assert_eq!(files[0].as_deref(), Some("Normal.ani"));
        assert_eq!(files[7].as_deref(), Some("Unavailable.cur"));
        assert_eq!(files[14].as_deref(), Some("Link.ani"));
        assert!(files[15].is_none());
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["v10", "v2", "V1", "v0", "Dark 12", "Dark 3"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["Dark 3", "Dark 12", "v0", "V1", "v2", "v10"]);
    }

    #[test]
    fn inf_value_lines() {
        let inf = r#"
[Wreg]
HKCU,"Control Panel\Cursors",,0x00020000,"%SCHEME_NAME%"
HKCU,"Control Panel\Cursors",Arrow,0x00020000,"%10%\%CUR_DIR%\%pointer%"
HKCU,"Control Panel\Cursors",Wait,0x00020000,"%10%\%CUR_DIR%\%busy%"
[Strings]
CUR_DIR = "Cursors\X"
SCHEME_NAME = "Value Lines"
pointer = "a.cur"
busy = "b.ani"
"#;
        let (name, files) = parse_inf(inf).unwrap();
        assert_eq!(name.as_deref(), Some("Value Lines"));
        assert_eq!(files[0].as_deref(), Some("a.cur"));
        assert_eq!(files[3].as_deref(), Some("b.ani"));
    }
}
