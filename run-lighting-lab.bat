@echo off
setlocal
cd /d "%~dp0"
if exist target\debug\phoenix-lighting-lab.exe (
    target\debug\phoenix-lighting-lab.exe %*
) else (
    cargo run --manifest-path prototypes\native-lighting\Cargo.toml --target-dir target -- %*
)
if errorlevel 1 pause
