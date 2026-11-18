# CursorDrop

Drop a mouse-cursor pack into a folder and it becomes your Windows cursors. No unzipping,
no right-click > Install on an `.inf`, no Control Panel.

CursorDrop is a small Windows app (Rust, no runtime needed) that watches its `cursors`
folder. Put a pack there as you downloaded it (`.zip`, `.7z` or `.rar`) and within a couple
of seconds it's applied. A window lists your packs and shows every cursor a pack will
replace before you use it.

![The pack list](docs/list.png)

![The cursors in a pack, now vs. new](docs/cursors.png)

![Dragging a pack onto the window](docs/drop.png)

## Download

Get `CursorDrop-v1.0.0-win64.zip` from [Releases](../../releases), unzip it anywhere and run
`CursorDrop.exe`. It's portable: no installer and no admin rights. Windows 10 or 11, 64-bit.

## Using it

- **Add a pack:** put the archive in the `cursors` folder next to `CursorDrop.exe`, click
  **Add pack…**, or drag it onto the window (it lights up to show it'll take the pack, or
  warns if what you're dragging isn't one). It's applied straight away.
- **See what changes:** click a pack. Each of the 17 Windows cursors is shown as it is now
  and as it would be with the pack; animated `.ani` cursors play.
- **Variants:** packs with several versions (sizes, colours, characters…) get browser-style
  tabs, named after their folders. Many variants scroll: use the arrows, the mouse wheel
  over the tabs, or Left / Right. Hover a tab to see its full name.
- **Undo:** CursorDrop remembers your cursors before it applies a pack. **Restore original
  cursors** brings them back, and so does deleting the pack from the folder (or
  **Remove pack**, which moves it to the Recycle Bin).
- **Tray:** with **Close to tray** on (the default), closing the window keeps CursorDrop in
  the tray, still watching the folder; turn it off and closing the window quits.
  **Start with Windows** starts it in the tray at sign-in.
- **Looks:** follows Windows' light/dark setting and accent colour, or pick Light / Dark;
  the pin keeps the window on top.

## How it works out the cursors

- If the pack has an install `.inf`, its cursor list is used as-is.
- Otherwise file names are matched to cursor roles: `Normal Select`, `Busy`, `Working In
  Background`, `Diagonal Resize 1`, `aero_ns`, `link`, `unavailable`, and so on.
- Every folder (or `.inf`) with cursors becomes one variant. Archives inside the archive
  (common for "all-in-one" packs) are opened too, up to three levels deep.
- Cursors a pack doesn't provide fall back to the Windows default.
- Only `.cur`, `.ani` and `.inf` files are extracted, and paths are checked so nothing can be
  written outside CursorDrop's folder.

Applying a pack writes your user's cursor settings (`HKEY_CURRENT_USER\Control Panel\Cursors`),
adds the scheme to Control Panel > Mouse > Pointers and tells Windows to reload the cursors.
Nothing system-wide is changed.

## Where things are kept

Next to `CursorDrop.exe`: `cursors\` (your packs) and `data\` (extracted cursors, settings,
a log). If that folder can't be written to, both live in `%LOCALAPPDATA%\CursorDrop`.

## Command line

| | |
|---|---|
| `CursorDrop.exe` | Opens the window (or brings up the running one). |
| `CursorDrop.exe --tray` | Starts in the tray only (used by Start with Windows). |
| `CursorDrop.exe --inspect <pack>` | Prints which file goes to which cursor; changes nothing. |
| `CursorDrop.exe --restore` | Restores the cursors you had before CursorDrop. |
| `CursorDrop.exe --version` | Prints the version. |

## Building

Needs Rust (MSVC toolchain) and the Windows SDK (for `rc.exe`; it comes with Visual Studio's
C++ workload). RAR support compiles the bundled UnRAR C++ source.

```
cargo build --release
```

`setup\package.bat` runs the tests, builds, and writes the release zip and its SHA-256 to
`dist\`.

For development, `CURSORDROP_SELFTEST=<folder>` installs the packs in the `cursors` folder
without applying them, renders every view to PNGs in `<folder>`, clicks through the UI and
prints `ok` / `FAILED` per check. No window is shown and your cursors aren't touched.
(`CURSORDROP_SELFTEST_NOW_DEFAULT=1` shows Windows' default cursors as "Now", as in the
screenshots above.)

## License

MIT, see [LICENSE](LICENSE). Third-party components and their licenses (including the
UnRAR license) are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

The screenshots use the classic and large cursor sets that ship with Windows.
