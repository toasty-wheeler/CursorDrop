//! Extracts only the files we care about (.cur, .ani, .inf) from zip / 7z / rar archives, and
//! opens archives nested inside them (a pack shipped as "zip in a zip") a few levels deep.
//! Entry paths are sanitized so nothing can escape the destination folder.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

const MAX_ENTRY: u64 = 32 * 1024 * 1024;
const MAX_NESTED_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_DEPTH: u32 = 3;

pub const ARCHIVE_EXTS: &[&str] = &["zip", "7z", "rar"];

pub fn is_archive(path: &Path) -> bool {
    ext_of(path).is_some_and(|e| ARCHIVE_EXTS.contains(&e.as_str()))
}

fn ext_of(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn is_cursor_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".cur") || lower.ends_with(".ani") || lower.ends_with(".inf")
}

/// Cursor files, install .infs, and archives that may hold more of them.
fn wanted(name: &str) -> bool {
    is_cursor_file(name) || is_archive(Path::new(name))
}

fn limit_for(name: &str) -> u64 {
    if is_archive(Path::new(name)) { MAX_NESTED_ARCHIVE } else { MAX_ENTRY }
}

/// Turns an archive entry name into a safe relative path (None if it tries to escape).
fn safe_rel(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() { None } else { Some(out) }
}

fn write_entry(dest: &Path, rel: &Path, reader: &mut dyn Read) -> io::Result<()> {
    let target = dest.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(&target)?;
    io::copy(&mut reader.take(limit_for(&rel.to_string_lossy())), &mut file)?;
    Ok(())
}

/// Extracts cursor-related files from `archive` into `dest`. Archives found inside are opened
/// into a folder named after them (and then deleted). Returns how many files were written.
pub fn extract(archive: &Path, dest: &Path) -> Result<usize, String> {
    extract_at(archive, dest, 0)
}

fn extract_at(archive: &Path, dest: &Path, depth: u32) -> Result<usize, String> {
    fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut count = match ext_of(archive).as_deref() {
        Some("zip") => extract_zip(archive, dest)?,
        Some("7z") => extract_7z(archive, dest)?,
        Some("rar") => extract_rar(archive, dest)?,
        _ => return Err("unsupported archive type".into()),
    };
    let mut inner = Vec::new();
    find_archives(dest, &mut inner);
    for nested in inner {
        count -= 1; // counted as written; replaced by what's inside it
        if depth < MAX_DEPTH {
            let folder = nested.with_extension("");
            // A broken inner archive shouldn't sink the whole pack.
            count += extract_at(&nested, &folder, depth + 1).unwrap_or(0);
        }
        let _ = fs::remove_file(&nested);
    }
    Ok(count)
}

fn find_archives(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            find_archives(&p, out);
        } else if is_archive(&p) {
            out.push(p);
        }
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<usize, String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("bad zip: {e}"))?;
    let mut count = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("bad zip entry: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        // Many zip tools store UTF-8 names without setting the UTF-8 flag, and the zip crate
        // then reads them as code page 437 ("Σ" becomes "Î£"). Valid UTF-8 wins.
        let raw = entry.name_raw();
        let rel = match std::str::from_utf8(raw) {
            Ok(s) if !raw.is_ascii() => safe_rel(s),
            _ => entry.enclosed_name(),
        };
        let Some(rel) = rel else { continue };
        if !wanted(&rel.to_string_lossy()) {
            continue;
        }
        write_entry(dest, &rel, &mut entry).map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

fn extract_7z(archive: &Path, dest: &Path) -> Result<usize, String> {
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
        .map_err(|e| format!("bad 7z: {e}"))?;
    let mut count = 0;
    let mut err: Option<String> = None;
    reader
        .for_each_entries(|entry, data| {
            if entry.is_directory() || !wanted(entry.name()) {
                return Ok(true);
            }
            if let Some(rel) = safe_rel(entry.name()) {
                match write_entry(dest, &rel, data) {
                    Ok(()) => count += 1,
                    Err(e) => {
                        err = Some(e.to_string());
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        })
        .map_err(|e| format!("7z error: {e}"))?;
    match err {
        Some(e) => Err(e),
        None => Ok(count),
    }
}

fn extract_rar(archive: &Path, dest: &Path) -> Result<usize, String> {
    let path = archive.to_path_buf();
    let mut arc = unrar::Archive::new(&path)
        .open_for_processing()
        .map_err(|e| format!("bad rar: {e}"))?;
    let mut count = 0;
    while let Some(header) = arc.read_header().map_err(|e| format!("rar error: {e}"))? {
        let entry = header.entry();
        let name = entry.filename.to_string_lossy().to_string();
        let target = if entry.is_file() && wanted(&name) && entry.unpacked_size <= limit_for(&name) {
            safe_rel(&name).map(|rel| dest.join(rel))
        } else {
            None
        };
        arc = match target {
            Some(target) => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                count += 1;
                header.extract_to(&target).map_err(|e| format!("rar error: {e}"))?
            }
            None => header.skip().map_err(|e| format!("rar error: {e}"))?,
        };
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        assert!(safe_rel("../evil.cur").is_none());
        assert!(safe_rel("C:\\evil.cur").is_none());
        assert!(safe_rel("/abs/evil.cur").is_none());
        assert_eq!(safe_rel("a\\b\\c.cur").unwrap(), PathBuf::from("a/b/c.cur"));
    }
}
