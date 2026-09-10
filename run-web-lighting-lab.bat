@echo off
cd /d "%~dp0"
node prototypes\web-lighting\stage-assets.mjs
if errorlevel 1 exit /b %errorlevel%
set NO_COLOR=true
set CARGO_TARGET_DIR=%CD%\target
trunk serve --config prototypes\web-lighting\Trunk.toml
