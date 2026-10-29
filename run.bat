@echo off
cd /d "%~dp0"
rem Map viewer. First build downloads egui and takes a minute.
cargo run --release --manifest-path apps\viewer\Cargo.toml
pause
