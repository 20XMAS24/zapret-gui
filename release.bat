@echo off
rem Usage:
rem   release.bat                       build the installer into dist\
rem   release.bat -Version 2.0.1 -Publish   bump version, build and publish a GitHub release
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0release.ps1" %*
pause
