# Releasing

Checklist for publishing a CursorDrop release to GitHub (`toasty-wheeler/CursorDrop`,
public, MIT). Commands are for Git Bash from the repo root.

## First release (v1.0.0): repo does not exist yet

1. **Clean tree, right version**
   - `git status` shows nothing to commit.
   - `Cargo.toml` `version = "1.0.0"`; `docs/release-notes/v1.0.0.md` exists;
     the README links to `releases/latest`, so it needs no change per version.
2. **Build the package**: `cmd //c setup\\package.bat`
   - runs `cargo test --release`, builds, checks `CursorDrop.exe --version`, writes
     `dist/CursorDrop-v1.0.0-win64.zip` and `.zip.sha256`.
3. **Smoke-test the zip** (in a scratch folder, not the working copy):
   - unzip, `CursorDrop.exe --version` prints `CursorDrop 1.0.0`;
   - `CursorDrop.exe --inspect <some pack>` lists the cursors;
   - `CURSORDROP_SELFTEST=<out>` with a pack in its `cursors` folder: every line `ok`.
4. **Create the repo and push** (`gh` is logged in as toasty-wheeler):
   ```
   gh repo create toasty-wheeler/CursorDrop --public \
     --description "Drop a cursor pack (.zip/.7z/.rar) into a folder and it becomes your Windows cursors. Tray app with a preview of every cursor." \
     --source . --remote origin --push
   gh repo edit toasty-wheeler/CursorDrop --add-topic windows --add-topic cursors \
     --add-topic mouse-cursor --add-topic cursor-theme --add-topic rust --add-topic tray-app
   ```
5. **Tag and release**
   ```
   git tag -a v1.0.0 -m "CursorDrop 1.0.0"
   git push origin v1.0.0
   gh release create v1.0.0 dist/CursorDrop-v1.0.0-win64.zip dist/CursorDrop-v1.0.0-win64.zip.sha256 \
     --title "CursorDrop 1.0.0" --notes-file docs/release-notes/v1.0.0.md
   ```
6. **Check**: the release page shows both assets; the README's screenshots render; the
   "Releases" link in the README goes to the release.

## Later releases

1. Bump `version` in `Cargo.toml` (the exe's version info and `--version` follow it).
2. Add `docs/release-notes/v<version>.md` (the README's download button and link point at
   `releases/latest` and follow on their own).
3. Commit, then steps 2, 3 and 5 above with the new version (the repo already exists, so
   just `git push` instead of step 4).

## Notes

- Commits use the repo-local identity `toasty-wheeler <352984143+toasty-wheeler@users.noreply.github.com>`.
- The icon is `assets/CursorDrop.ico`, made with `CursorDrop.exe --make-icon assets\CursorDrop.ico`
  (drawn in code, see `gfx::make_icon`); `build.rs` embeds it with the version info.
- README screenshots: build the demo packs from Windows' own cursors (classic `*_r/_rm/_rl`,
  `*_i/_im/_il`, `*_m/_l` sets and the `aero_*_xl` set), then run the self-test with
  `CURSORDROP_SELFTEST_NOW_DEFAULT=1` and copy `list-light.png` / `detail-dark.png` to
  `docs/list.png` / `docs/cursors.png`.
- If `THIRD_PARTY_NOTICES.md` needs refreshing after dependency changes:
  `cargo tree -e normal --prefix none --format "{p}|{l}|{r}"` lists what's in the exe.
