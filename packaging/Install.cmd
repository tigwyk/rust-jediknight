@echo off
rem Double-click installer for bevi-knight. Windows PowerShell 5.1+ only; no admin needed.
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
if "%~1"=="" pause
endlocal
