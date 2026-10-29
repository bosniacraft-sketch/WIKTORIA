@echo off
cd /d "%~dp0"
rem Command-line tools: summary (default), goods, render OUT.bmp
cargo run --release -p wiktoria-cli -- %*
