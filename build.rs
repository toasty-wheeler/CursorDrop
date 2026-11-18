use std::path::PathBuf;

fn main() {
    // unrar's bundled C++ uses the CryptoAPI for RAR encryption support.
    println!("cargo:rustc-link-lib=advapi32");

    // The exe's icon and version info (Explorer's Properties > Details), from Cargo.toml.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("assets").join("CursorDrop.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u32> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let commas = parts.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");

    let mut rc = String::new();
    if icon.exists() {
        // Generated with `CursorDrop.exe --make-icon assets\CursorDrop.ico`.
        rc += &format!("1 ICON \"{}\"\n", icon.display().to_string().replace('\\', "\\\\"));
    }
    rc += &format!(
        r#"1 VERSIONINFO
FILEVERSION {commas}
PRODUCTVERSION {commas}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "toasty-wheeler"
      VALUE "FileDescription", "CursorDrop - cursor packs, applied when you drop them in"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "CursorDrop"
      VALUE "LegalCopyright", "Copyright (c) 2026 toasty-wheeler. MIT License."
      VALUE "OriginalFilename", "CursorDrop.exe"
      VALUE "ProductName", "CursorDrop"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("CursorDrop.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
}
