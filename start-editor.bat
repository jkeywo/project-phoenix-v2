@echo off
echo Starting editor server on http://localhost:3000
cd /d "%~dp0"
start "" http://localhost:3000/editor.html
start /B npx --yes serve . -l 3000 > NUL 2>&1
echo Press Ctrl+C to stop the server.
pause