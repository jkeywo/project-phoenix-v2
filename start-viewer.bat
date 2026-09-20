@echo off
setlocal
cd /d "%~dp0"
echo The standalone Model Viewer has moved to Workshop Models and Preview.
rem Batch command lines cannot transport arbitrary selectors without cmd.exe
rem expanding metacharacters. Set PHOENIX_WORKSHOP_OPEN to a URL query such as
rem model=assets/models/ship.glb^&lighting=directional; Node reads it directly.
node scripts\dev-workshop.mjs --models
