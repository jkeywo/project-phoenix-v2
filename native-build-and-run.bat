@echo off
REM Build the native host and client, then launch with the same modes and args
REM as native-run.bat (default: lobby; delivery or raw are also supported).
setlocal
cd /d "%~dp0"

REM Build the host and client bundle before launching. build.rs stages the
REM Ultralight SDK DLLs beside the exe, so a fresh build starts cleanly.
echo [native-build-and-run] building phoenix-host (release, --features ultralight)...
cargo build --release --features ultralight --bin phoenix-host || (echo [ERROR] cargo build failed & exit /b 1)
echo [native-build-and-run] building the phone-client and shared native-lobby modules...
node scripts\build-client.mjs || (echo [ERROR] client build failed & exit /b 1)

REM Backstop for the build.rs staging. `ul-next-sys` declares no `links` key, so
REM Cargo is free to run its SDK *download* alongside our build script instead of
REM before it; on a cold checkout build.rs can look for the DLLs a moment too
REM early and warn instead of copying. By here the build is finished, so the SDK
REM is certainly on disk - copy anything still missing beside the exe, or the
REM host dies at load with a silent STATUS_DLL_NOT_FOUND.
for /d %%d in ("target\release\build\ul-next-sys-*") do (
    if exist "%%~fd\out\ul-sdk\bin\Ultralight.dll" (
        for %%f in ("%%~fd\out\ul-sdk\bin\*.dll") do (
            if not exist "target\release\%%~nxf" (
                echo [native-build-and-run] staging %%~nxf beside the exe
                copy /y "%%f" "target\release\" >nul
            )
        )
    )
)

call "%~dp0native-run.bat" %*
exit /b %errorlevel%
