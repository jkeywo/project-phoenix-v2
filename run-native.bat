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

REM `dist` is trunk's output. The host version-pins it against the manifest it
REM serves at startup, so a stale bundle refuses to start before the port is
REM taken rather than misbehaving later.
if not exist "dist\index.html" (
    echo [ERROR] dist\index.html not found - build the bundle first:
    echo           trunk build --release
    echo           node scripts/build-client.mjs
    exit /b 1
)

REM `goto` rather than an if-block, and this is not style: cmd expands %1..%9
REM for a whole parenthesised block BEFORE running any of it, so a `shift`
REM inside one has no effect on the very line that needs it. Jumping out is what
REM makes the shift take.
if /i "%~1"=="lobby" goto :lobby

echo === Native host: delivery only ===
"%HOST%" --client-dir dist %*
exit /b %errorlevel%

:lobby
REM The default branch above forwards `%*` - every argument, however many. This
REM branch cannot simply copy that line, because cmd's `shift` renumbers %1..%9
REM and leaves `%*` naming the WHOLE original command line, mode word included.
REM So the arguments are re-accumulated one at a time, which is the forwarding
REM `%*` would have given: no nine-argument ceiling (`--rendezvous URL --origin
REM URL --addr HOST:PORT --manifest M` is already eight), and each one carried
REM with the quoting the caller typed, because %1 is expanded rather than %~1.
set "LOBBY_ARGS="
shift
:lobby_args
if "%~1"=="" goto :lobby_run
set "LOBBY_ARGS=%LOBBY_ARGS% %1"
shift
goto :lobby_args

:lobby_run
echo === Native host: lobby (pick the scenario on the viewscreen) ===
"%HOST%" --client-dir dist --lobby%LOBBY_ARGS%
exit /b %errorlevel%
