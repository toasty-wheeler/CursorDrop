@echo off
rem Builds dist\CursorDrop-v<version>-win64.zip (see package.ps1).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0package.ps1" || exit /b 1
