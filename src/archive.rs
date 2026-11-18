//! Extracts only the files we care about (.cur, .ani, .inf) from zip / 7z / rar archives.
//! Entry paths are sanitized so nothing can escape the destination folder.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

const MAX_ENTRY: u64 = 32 * 1024 * 1024;

pub const ARCHIVE_EXTS: &[&str] = &["zip", "7z", "rar"];

pub fn is_archive(path: &Path) -> bool {
    ext_of(path).is_some_and(|e| ARCHIVE_EXTS.contains(&e.as_str()))
}

fn ext_of(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn wanted(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".cur") || lower.ends_with(".ani") || lower.ends_with(".inf")
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
    io::copy(&mut reader.take(MAX_ENTRY), &mut file)?;
    Ok(())
}

/// Extracts cursor-related files from `archive` into `dest`. Returns how many files were written.
pub fn extract(archive: &Path, dest: &Path) -> Result<usize, String> {
    fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    match ext_of(archive).as_deref() {
        Some("zip") => extract_zip(archive, dest),
        Some("7z") => extract_7z(archive, dest),
        Some("rar") => extract_rar(archive, dest),
        _ => Err("unsupported archive type".into()),
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
        let Some(rel) = entry.enclosed_name() else { continue };
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
        let target = if entry.is_file() && wanted(&name) && entry.unpacked_size <= MAX_ENTRY {
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
