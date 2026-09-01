@echo off
REM =====================================================
REM run-native.bat - launch the native host (PRD #855 /
REM issues #1121, #1326, #1328)
REM
REM   run-native.bat          delivery only, exactly as
REM                           AGENTS.md and the acceptance
REM                           kits document it. DO NOT
REM                           change what this default
REM                           passes - kits depend on it.
REM
REM   run-native.bat lobby    the same binary, authoritative,
REM                           booting onto the scenario
REM                           picker on the viewscreen
REM                           (issue #1328). No mission
REM                           flags: the world and the hull
REM                           are chosen on screen, so this
REM                           passes no --world and no
REM                           --ship.
REM
REM Anything after the mode is forwarded verbatim, so the
REM ordinary extras still work:
REM
REM   run-native.bat lobby --rendezvous <URL> --origin <URL>
REM   run-native.bat --addr 127.0.0.1:8080
REM
REM Build first (once), with the SDK feature if you want the
REM viewscreen to composite the lobby surface at all:
REM
REM   cargo build --release --features ultralight --bin phoenix-host
REM
REM Without --features ultralight the host runs and serves,
REM and the lobby surface is simply not composited - so the
REM picker has to come from a phone instead.
REM =====================================================
setlocal
cd /d "%~dp0"

set "HOST=target\release\phoenix-host.exe"
if not exist "%HOST%" (
    echo [ERROR] %HOST% not found.
    echo         Build it first:
    echo           cargo build --release --features ultralight --bin phoenix-host
    exit /b 1
)

REM `dist` is trunk's output. The host version-pins it against the manifest it
REM serves at startup, so a stale bundle refuses to start before the port is
REM taken rather than misbehaving later.
if not exist "dist\index.html" (
    echo [ERROR] dist\index.html not found — build the bundle first:
    echo           trunk build --release
    echo           node scripts/build-client.mjs
    exit /b 1
)

if /i "%~1"=="lobby" (
    shift
    echo === Native host: lobby ^(pick the scenario on the viewscreen^) ===
    "%HOST%" --client-dir dist --lobby %1 %2 %3 %4 %5 %6 %7 %8 %9
    exit /b %errorlevel%
)

echo === Native host: delivery only ===
"%HOST%" --client-dir dist %*
exit /b %errorlevel%
