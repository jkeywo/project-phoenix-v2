@echo off
REM =====================================================
REM deploy-local.bat — Build server + client, serve dist/
REM =====================================================
setlocal enabledelayedexpansion

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

echo === Starting dev server on http://localhost:3000 ===
echo Press Ctrl+C to stop.
npx serve dist -p 3000 --no-clipboard