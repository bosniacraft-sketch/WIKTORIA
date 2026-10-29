@echo off
rem Wiktoria.
rem   wiktoria.bat          opens the main menu (New game, Load game, Settings).
rem   wiktoria.bat <days>   goes straight into the 1836 start and runs that many days first,
rem                         e.g. wiktoria.bat 365 opens on 1837 (wiktoria.bat 0 = straight in on day one).
setlocal
cd /d "%~dp0"
set "WIKTORIA_SAVE=%~dp0start\1836.v2"
set "WIKTORIA_DAYS=%~1"
if "%WIKTORIA_DAYS%"=="" (
  cargo run --release --manifest-path apps\viewer\Cargo.toml
  pause
  exit /b
)
if not exist "%WIKTORIA_SAVE%" (
  echo Start save not found: %WIKTORIA_SAVE%
  pause
  exit /b 1
)
cargo run --release --manifest-path apps\viewer\Cargo.toml -- --save "%WIKTORIA_SAVE%" --start-days %WIKTORIA_DAYS%
pause
