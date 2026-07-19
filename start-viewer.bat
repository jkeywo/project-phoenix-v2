@echo off
REM =====================================================
REM start-viewer.bat — Build and launch the model viewer
REM
REM Compiles the viewer WASM, starts the dev server, and
REM opens the page. Iterate on shaders and lighting here
REM instead of booting a whole scenario.
REM
REM Edit a .wgsl or .model.toml and the page rebuilds and
REM reloads itself; Ctrl+C stops the server.
REM =====================================================
setlocal
cd /d "%~dp0"

echo === Checking the viewer compiles ===
REM Fail fast with readable errors here rather than burying
REM them in Trunk's output after the server has started.
call cargo check --no-default-features --features viewer --target wasm32-unknown-unknown
if %errorlevel% neq 0 (
    echo [ERROR] Viewer failed to compile.
    exit /b %errorlevel%
)

echo === Starting model viewer on http://localhost:8081 ===
echo Press Ctrl+C to stop.

REM Give Trunk a head start on the first build before opening
REM the tab, otherwise the browser lands on a connection error.
REM Fully-qualified timeout.exe: a Git Bash / MSYS `timeout` earlier on PATH
REM takes GNU-style arguments and fails on /t.
start "" /B cmd /c ""%SystemRoot%\System32\timeout.exe" /t 8 /nobreak > NUL && start "" http://localhost:8081/"

REM dev-viewer.mjs runs Trunk plus the static server that /assets
REM is proxied to — see the comments in that script for why the
REM assets are proxied rather than copied.
node scripts/dev-viewer.mjs
