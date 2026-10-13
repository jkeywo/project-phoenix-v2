@echo off
REM =====================================================
REM local-build-and-deploy.bat - Build server + client, serve dist/
REM =====================================================
setlocal enabledelayedexpansion
cd /d "%~dp0"

echo === Building server (Trunk) ===
call trunk build --release
if %errorlevel% neq 0 (
    echo [ERROR] Server build failed.
    exit /b %errorlevel%
)

echo === Building client ===
call node scripts/build-client.mjs
if %errorlevel% neq 0 (
    echo [ERROR] Client build failed.
    exit /b %errorlevel%
)

call "%~dp0local-deploy.bat"
exit /b %errorlevel%
