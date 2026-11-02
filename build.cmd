@echo off
REM ezSsh build helper for Windows cmd.exe.
REM
REM Usage:
REM   build            cargo run (debug), the real database
REM   build run        same as above
REM   build build      cargo build (debug)
REM   build release    cargo build --profile dist (target\dist\ezssh.exe)
REM   build package    NSIS + MSI installers via cargo-packager
REM   build test       cargo test + clippy
REM   build clean      remove target\
REM   build help

setlocal EnableExtensions EnableDelayedExpansion
cd /d "%~dp0"

set "CMD=%~1"
if "%CMD%"=="" set "CMD=run"

if /i "%CMD%"=="help"    goto :help
if /i "%CMD%"=="-h"      goto :help
if /i "%CMD%"=="--help"  goto :help
if /i "%CMD%"=="run"     goto :run
if /i "%CMD%"=="build"   goto :build
if /i "%CMD%"=="release" goto :release
if /i "%CMD%"=="package" goto :package
if /i "%CMD%"=="test"    goto :test
if /i "%CMD%"=="clean"   goto :clean

echo [!!!] Unknown command: %CMD%. Run `build help`.
exit /b 1

:run
echo [^>^>^>] cargo run -p ezssh-app
cargo run -p ezssh-app
exit /b %errorlevel%

:build
echo [^>^>^>] cargo build ^(debug^)
cargo build --workspace
exit /b %errorlevel%

:release
echo [^>^>^>] cargo build --profile dist ^(fat LTO, a few minutes^)
cargo build -p ezssh-app --profile dist --locked
exit /b %errorlevel%

:package
where cargo-packager >nul 2>&1
if errorlevel 1 (
    echo [!!!] cargo-packager not found. Install it: cargo install cargo-packager --locked
    exit /b 1
)
echo [^>^>^>] cargo packager -f nsis,wix
cargo packager -p ezssh-app --release -f nsis,wix
exit /b %errorlevel%

:test
echo [^>^>^>] cargo test
cargo test --workspace || exit /b 1
echo [^>^>^>] cargo clippy
cargo clippy --workspace --all-targets -- -D warnings || exit /b 1
echo [^>^>^>] All checks passed.
exit /b 0

:clean
echo [^>^>^>] Removing target\
if exist "target" rmdir /s /q "target"
echo [^>^>^>] Clean.
exit /b 0

:help
echo Usage:
echo   build            cargo run (debug), the real database
echo   build run        same as above
echo   build build      cargo build (debug)
echo   build release    cargo build --profile dist
echo   build package    NSIS + MSI installers via cargo-packager
echo   build test       cargo test + clippy
echo   build clean      remove target\
exit /b 0
