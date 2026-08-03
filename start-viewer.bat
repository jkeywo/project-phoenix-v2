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

REM Open the tab only once Trunk is actually listening. Trunk does not bind its
REM port until the FIRST wasm build finishes, which is minutes from cold — a
REM fixed head start (this used to wait 8 seconds) drops the browser on a dead
REM port and the tab shows a connection error nobody thinks to refresh.
start "" /B powershell -NoProfile -Command ^
  "while (-not (Test-NetConnection -ComputerName localhost -Port 8081 -InformationLevel Quiet -WarningAction SilentlyContinue)) { Start-Sleep -Seconds 2 }; Start-Process 'http://localhost:8081/'"

REM dev-viewer.mjs runs Trunk plus the static server that /assets
REM is proxied to — see the comments in that script for why the
REM assets are proxied rather than copied.
node scripts/dev-viewer.mjs
