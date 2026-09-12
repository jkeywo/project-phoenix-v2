@echo off
setlocal
cd /d "%~dp0"

where node >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Node.js is required to open the Workshop.
    goto :failed
)

echo [run-workshop] Building Workshop Authoring...
node scripts\build-workshop.mjs
if errorlevel 1 (
    echo [ERROR] Workshop build failed. For missing dependencies, run npm ci first.
    goto :failed
)

node scripts\serve-workshop.mjs %*
if errorlevel 1 goto :failed
exit /b 0

:failed
if /i not "%~1"=="--no-open" pause
exit /b 1
