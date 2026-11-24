@echo off
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0test-platform-windows.ps1" %*
exit /b %ERRORLEVEL%
