# CursorDrop

A small Windows tray app (Rust) that applies mouse-cursor packs automatically.

Drop a cursor pack archive (`.zip`, `.7z` or `.rar`) into the `cursors` folder next to
`CursorDrop.exe` and within a couple of seconds it becomes your Windows cursor scheme.

## How it works

- Every 2 s the `cursors` folder is checked. A new or changed archive is processed once its
  size has stopped changing (so half-copied downloads are not read).
- Only `.cur`, `.ani` and `.inf` files are extracted, into `data\schemes\<archive>\`.
- Cursor roles are mapped from the pack's install `.inf` when present, otherwise guessed from
  file names (`Normal Select`, `Busy`, `aero_ns`, `Diagonal Resize 1`, `link`, ...).
  Roles a pack doesn't provide fall back to the Windows built-in cursor.
- An archive with several variants (one `.inf` or folder per variant) gives several schemes;
  the first is applied and the others can be chosen from the tray menu.
- Applying writes `HKCU\Control Panel\Cursors`, registers the scheme in Mouse Properties and
  reloads cursors (`SPI_SETCURSORS`). Your previous cursors are saved first.
- Deleting an archive from the folder removes its files, and if it was active your previous
  cursors come back.

## Window

Styled like DS4-Native and MagicMouseNative: translucent cards, Windows accent colour, an
Automatic / Light / Dark theme menu and an always-on-top pin.

- **Packs list**, like Explorer's details view: Name, Date modified, Type, Size and a preview of
  each pack's cursors. Click a column header to sort, drag a divider to resize. Right-click a pack
  for more.
- **Click a pack** to see a grid of all 17 cursors it replaces, what you have now next to what
  you'd get (animated `.ani` cursors play). Packs with variants get browser-style tabs: they keep a minimum width and
  scroll when there are many (arrows at the ends, or the mouse wheel / tilt over them), a
  hovered tab shows its full name, and Left / Right switch variants. From there: use the
  cursors, show the archive in Explorer, or remove the pack (to the Recycle Bin).
- **Add pack…** or drag archives onto the window to copy them into the cursors folder.
- Closing the window keeps CursorDrop in the tray; click the tray icon to bring it back.
  With "Start with Windows" on it starts in the tray (`--tray`).

## Tray menu

Open CursorDrop · Pick a scheme · Open cursors folder · Rescan now · Restore original cursors ·
Start with Windows · Exit.

## Command line

- `CursorDrop.exe --inspect <archive>` prints which file would go to which cursor role.
- `CursorDrop.exe --restore` restores the cursors saved before CursorDrop applied one.
- `CURSORDROP_SELFTEST=<folder>` installs the packs without applying them, renders every view
  to PNGs in that folder, clicks through the UI and prints ok/FAILED per check, then exits.

## Build

`cargo build --release` (MSVC toolchain; the RAR support compiles bundled C++ unrar code).
