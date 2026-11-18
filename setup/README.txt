CursorDrop
==========

Drop a mouse-cursor pack into the "cursors" folder and it becomes your Windows cursor
scheme. No installer, no admin rights.

Getting started
---------------
1. Unzip this folder anywhere you like (e.g. Documents\CursorDrop) and run CursorDrop.exe.
2. Put a cursor pack (.zip, .7z or .rar, as downloaded) into the "cursors" folder, click
   "Add pack..." in the window, or drag the archive onto the window.
3. It is applied within a couple of seconds. Click a pack in the list to see every cursor
   it replaces; packs with several variants get tabs to choose from.

With "Close to tray" on (the default), closing the window keeps CursorDrop running in the
tray (bottom right): click the tray icon to bring the window back, right-click it for the
menu, including Exit. Turn "Close to tray" off and closing the window quits CursorDrop.

Getting your old cursors back
-----------------------------
Before applying a pack, CursorDrop remembers the cursors you had. Use "Restore original
cursors" in the window or tray menu, or delete the pack from the cursors folder.
You can also pick any scheme in Windows: Control Panel > Mouse > Pointers (applied packs
are listed there too).

Good to know
------------
- CursorDrop keeps its data in the "data" folder next to the exe. If that folder can't be
  written (e.g. inside Program Files), it uses %LOCALAPPDATA%\CursorDrop instead.
- It only changes your own user's cursor settings (HKEY_CURRENT_USER\Control Panel\Cursors).
- "Start with Windows" (in the window) starts it quietly in the tray at sign-in.
- Command line: CursorDrop.exe --inspect <pack.zip> shows which file would be used for
  which cursor, without changing anything. CursorDrop.exe --restore restores your cursors.

Licence: MIT (LICENSE.txt). Third-party components: THIRD_PARTY_NOTICES.txt.
Source and updates: https://github.com/toasty-wheeler/CursorDrop
