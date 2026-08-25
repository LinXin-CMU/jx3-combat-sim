@echo off
chcp 65001 >nul
title JX3 FenShanJin Calculator

echo [1/2] Starting backend (Rust, port 3005)...
cd /d "%~dp0backend"
start "Backend - Rust" cmd /k "cargo run 2>&1"

echo [2/2] Starting frontend (port 8081)...
cd /d "%~dp0frontend"
start "Frontend" cmd /k "python -m http.server 8081 2>&1"

timeout /t 3 /nobreak >nul
echo.
echo ========================================
echo   Backend : http://localhost:3005
echo   Frontend: http://localhost:8080
echo ========================================
echo.
start http://localhost:8080
