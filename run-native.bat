@echo off
REM =====================================================
REM run-native.bat - launch the native host (PRD #855 /
REM issues #1121, #1326, #1328)
REM
REM   run-native.bat            DEFAULT: the on-screen game
REM                             lobby, on the viewscreen, with
REM                             the join relay pointed at THIS
REM                             machine's LAN address so a crew
REM                             on the same network can join.
REM                             Pick the scenario and hull on
REM                             screen; no --world / --ship.
REM
REM   run-native.bat delivery   the old delivery-only host
REM                             (serves the bundle, no game
REM                             window). This is the invocation
REM                             the acceptance kits document -
REM                             use it for those.
REM
REM   run-native.bat raw <args> pure passthrough to phoenix-host
REM                             with --client-dir dist plus your
REM                             own flags (nothing added).
REM
REM Extras after the mode are forwarded verbatim, e.g.
REM   run-native.bat --addr 0.0.0.0:8080
REM   run-native.bat raw --world assets\worlds\combat_test.toml --solo
REM
REM To let phones actually JOIN, the join relay has to be
REM reachable at the LAN address this script uses. Run it
REM locally in another terminal (no cloud, no account needed):
REM   cd worker-rendezvous
REM   npx wrangler dev --ip 0.0.0.0 --port 8787
REM One caveat today (issue #1336): a scanning phone still dials
REM the built-in cloud relay, so LAN-only joins need either that
REM relay deployed with this LAN origin allow-listed, or the
REM client rebuilt to dial the local relay. The lobby, the
REM viewscreen and the on-screen controls all work regardless.
REM =====================================================
setlocal EnableDelayedExpansion
cd /d "%~dp0"

set "HOST=target\release\phoenix-host.exe"

REM Build-and-run: the host build is a fast no-op when nothing changed, and the
REM client bundle rebuild is quick, so both run every time. build.rs stages the
REM Ultralight SDK DLLs beside the exe, so a fresh build starts cleanly.
echo [run-native] building phoenix-host (release, --features ultralight)...
cargo build --release --features ultralight --bin phoenix-host || (echo [ERROR] cargo build failed & exit /b 1)
echo [run-native] building the phone-client bundle...
node scripts\build-client.mjs || (echo [ERROR] client build failed & exit /b 1)

if not exist "%HOST%" (
    echo [ERROR] %HOST% not found after the build.
    exit /b 1
)
if not exist "dist\index.html" (
    echo [ERROR] dist\index.html not found - build the bundle first:
    echo           trunk build --release
    echo           node scripts/build-client.mjs
    exit /b 1
)

REM ---- Mode dispatch (goto, not an if-block: cmd expands %1..%9 for a whole
REM ---- parenthesised block before running it, so a shift inside one is inert).
if /i "%~1"=="delivery" goto :delivery
if /i "%~1"=="raw" goto :raw
if /i "%~1"=="lobby" (shift & goto :lobby)
goto :lobby

:delivery
shift
set "DEL_ARGS="
:del_loop
if "%~1"=="" goto :del_run
set "DEL_ARGS=!DEL_ARGS! %1"
shift
goto :del_loop
:del_run
echo === Native host: delivery only (no game window; serves the bundle) ===
"%HOST%" --client-dir dist!DEL_ARGS!
exit /b %errorlevel%

:raw
shift
set "RAW_ARGS="
:raw_loop
if "%~1"=="" goto :raw_run
set "RAW_ARGS=!RAW_ARGS! %1"
shift
goto :raw_loop
:raw_run
"%HOST%" --client-dir dist!RAW_ARGS!
exit /b %errorlevel%

:lobby
REM Detect this machine's LAN IPv4 (the interface that owns the default route and
REM is Up), skipping WSL/Hyper-V virtual adapters, which have no default gateway.
set "LANIP="
for /f "usebackq delims=" %%i in (`powershell -NoProfile -Command "$a=(Get-NetIPConfiguration).Where({$_.IPv4DefaultGateway -and $_.NetAdapter.Status -eq 'Up'}); if($a){$a[0].IPv4Address.IPAddress}"`) do set "LANIP=%%i"
if not defined LANIP (
    echo [run-native] could not detect a LAN IP; falling back to 127.0.0.1
    set "LANIP=127.0.0.1"
)
set "RZV=http://!LANIP!:8787"
set "ORG=http://!LANIP!:8080"

REM Re-accumulate any extra args (cmd's shift leaves %* naming the whole original
REM line, mode word included, so it cannot be copied literally).
set "EXTRA="
:lobby_loop
if "%~1"=="" goto :lobby_run
set "EXTRA=!EXTRA! %1"
shift
goto :lobby_loop

:lobby_run
echo === Native host: lobby (pick the scenario on the viewscreen) ===
echo     LAN address : !LANIP!
echo     join relay  : !RZV!   (run: cd worker-rendezvous ^&^& npx wrangler dev --ip 0.0.0.0 --port 8787)
echo     phones open : !ORG!
"%HOST%" --client-dir dist --lobby --rendezvous !RZV! --origin !ORG!!EXTRA!
exit /b %errorlevel%
