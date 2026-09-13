@echo off
setlocal
cd /d "%~dp0"

where node >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Node.js is required to open the Workshop.
    goto :failed
)

echo [run-workshop] Building Workshop Authoring...
where trunk >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Trunk and the Rust WASM target are required for runtime validation.
    goto :failed
)
trunk build
if errorlevel 1 (
    echo [ERROR] Workshop build failed. See the build diagnostics above.
    goto :failed
)

node scripts\serve-workshop.mjs %*
if errorlevel 1 goto :failed
exit /b 0

:failed
if /i not "%~1"=="--no-open" pause
exit /b 1
