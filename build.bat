@echo off
title Build ZapretGUI (Rust)
cd /d "%~dp0"

rem Rust GNU toolchain: its bundled MinGW tools (dlltool, ld) must be on PATH.
set "SC=%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained"
set "PATH=%USERPROFILE%\.cargo\bin;%SC%;%PATH%"

where cargo >nul 2>&1 || (echo   [X] cargo not found. Install Rust: https://rustup.rs & pause & exit /b 1)

cargo build --release --locked
if errorlevel 1 (echo. & echo   [X] Build FAILED. & pause & exit /b 1)

taskkill /IM ZapretGUI.exe /F >nul 2>&1
if exist "..\ZapretGUI.exe" if not exist "..\_backup" mkdir "..\_backup"
copy /Y "target\release\ZapretGUI.exe" "..\ZapretGUI.exe" >nul
echo.
echo   [OK] Built and copied to ..\ZapretGUI.exe
pause
