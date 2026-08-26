@echo off
setlocal
chcp 65001 >nul
title Cangyun Qiling - Local

set "PROJECT_ROOT=%~dp0"
if not defined JX3_PORT set "JX3_PORT=3005"
if not defined JX3_BIND set "JX3_BIND=127.0.0.1"
if not defined JX3_AGENT_CONFIG if exist "%PROJECT_ROOT%agent.providers.toml" set "JX3_AGENT_CONFIG=%PROJECT_ROOT%agent.providers.toml"
if not defined JX3_KNOWLEDGE_ROOT if exist "%USERPROFILE%\Documents\苍云策划知识库\朔风卷雪\_migration-manifest.json" set "JX3_KNOWLEDGE_ROOT=%USERPROFILE%\Documents\苍云策划知识库\朔风卷雪"

where cargo >nul 2>nul
if errorlevel 1 (
  echo [X] Rust/Cargo was not found in PATH.
  echo     Install the Rust toolchain, reopen this terminal, and try again.
  pause
  exit /b 1
)

echo [*] Starting Cangyun Qiling...
echo     URL: http://localhost:%JX3_PORT%
if defined JX3_KNOWLEDGE_ROOT echo     Knowledge: %JX3_KNOWLEDGE_ROOT%
echo     The Rust backend also serves the frontend; no second web server is needed.
echo     Press Ctrl+C to stop.
echo.

cd /d "%PROJECT_ROOT%backend"
cargo run
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" (
  echo.
  echo [X] Application exited with code %EXIT_CODE%.
  pause
)
exit /b %EXIT_CODE%
