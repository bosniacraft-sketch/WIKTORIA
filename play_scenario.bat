@echo off
rem Wiktoria in the browser with a test scenario: play_scenario.bat siege_forts
rem (a name from the scenarios folder, or a path to a scenario file). Lists them when run without one.
rem Close this window (or press Ctrl+C in it) to stop the game.
setlocal
cd /d "%~dp0"
if not exist vic2_path.txt (
  echo vic2_path.txt is missing: it holds the path of your Victoria 2 folder.
  pause
  exit /b 1
)
call :find_cargo
set "EXE=target\release\wiktoria-cli.exe"
if defined CARGO (
  echo Building Wiktoria with "%CARGO%" ^(quick when nothing changed^)...
  "%CARGO%" build --release
  if errorlevel 1 (
    pause
    exit /b 1
  )
) else (
  echo Rust ^(cargo^) was not found. Install it from https://rustup.rs and run this again.
  pause
  exit /b 1
)
set "WIKTORIA_ROOT=%CD%"
if "%~1"=="" (
  "%EXE%" scenario --list
  echo.
  echo Usage: play_scenario.bat ^<scenario^>   for example: play_scenario.bat siege_forts
  pause
  exit /b 1
)
rem The game loads for a while before it answers: open the page only once it does.
echo Loading the game, the page opens by itself when it is ready (this can take a minute)...
start "" /b powershell -NoProfile -WindowStyle Hidden -Command "for ($i = 0; $i -lt 600; $i++) { try { (New-Object Net.Sockets.TcpClient('127.0.0.1', 8036)).Close(); Start-Process 'http://127.0.0.1:8036/'; break } catch { Start-Sleep 1 } }"
"%EXE%" serve --scenario "%~1" --port 8036 2>&1 | powershell -NoProfile -Command "$input | Tee-Object -FilePath wiktoria_web.log"
echo.
echo The game stopped. What it printed is also in %CD%\wiktoria_web.log
pause
exit /b

rem Sets CARGO to a working cargo.exe: PATH first, then the usual rustup places.
:find_cargo
set "CARGO="
for /f "delims=" %%c in ('where cargo 2^>nul') do if not defined CARGO set "CARGO=%%c"
if not defined CARGO if defined CARGO_HOME if exist "%CARGO_HOME%\bin\cargo.exe" set "CARGO=%CARGO_HOME%\bin\cargo.exe"
if not defined CARGO if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
exit /b
