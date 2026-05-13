@echo off
REM =====================================================
REM run-tests.bat — Run Rust unit tests + smoke tests
REM =====================================================
setlocal enabledelayedexpansion
set FAILED=0

echo ====== Rust Unit Tests ======
call cargo test
if %errorlevel% neq 0 (
    echo [FAIL] Rust unit tests failed.
    set FAILED=1
) else (
    echo [PASS] Rust unit tests passed.
)

echo.
echo ====== Build dist/ for smoke tests ======
call trunk build --release
if %errorlevel% neq 0 (
    echo [FAIL] Server dist build failed.
    set FAILED=1
)
call trunk build --release --config client-trunk.toml
if %errorlevel% neq 0 (
    echo [FAIL] Client dist build failed.
    set FAILED=1
)

echo.
echo ====== Smoke Tests ======
pushd tests\smoke
call npx playwright test
if %errorlevel% neq 0 (
    echo [FAIL] Smoke tests failed.
    set FAILED=1
) else (
    echo [PASS] Smoke tests passed.
)
popd

echo.
if %FAILED% neq 0 (
    echo ====== SOME TESTS FAILED ======
    exit /b 1
) else (
    echo ====== ALL TESTS PASSED ======
    exit /b 0
)