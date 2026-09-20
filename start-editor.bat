@echo off
setlocal
cd /d "%~dp0"
echo The World Editor has moved to the native Workshop project workspace.
rem Set PHOENIX_WORKSHOP_OPEN=file=assets/worlds/name.toml for a bounded
rem selection. It is read directly by Node and never expanded by cmd.exe.
node scripts\dev-workshop.mjs
