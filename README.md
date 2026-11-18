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

## Tray menu

Pick a scheme · Open cursors folder · Rescan now · Restore original cursors ·
Start with Windows · Exit. Double-click the icon to open the cursors folder.

## Command line

- `CursorDrop.exe --inspect <archive>` prints which file would go to which cursor role.
- `CursorDrop.exe --restore` restores the cursors saved before CursorDrop applied one.

## Build

`cargo build --release` (MSVC toolchain; the RAR support compiles bundled C++ unrar code).
