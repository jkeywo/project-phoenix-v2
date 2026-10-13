@echo off
REM Serve the existing dist/ bundle without rebuilding.
setlocal
cd /d "%~dp0"

if not exist "dist\index.html" (
    echo [ERROR] dist\index.html not found - run local-build-and-deploy.bat first.
    exit /b 1
)

echo === Starting dev server on http://localhost:3000 ===
echo Press Ctrl+C to stop.
call npx serve dist -p 3000 --no-clipboard
exit /b %errorlevel%
