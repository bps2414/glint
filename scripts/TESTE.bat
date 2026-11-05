@echo off
rem Teste guiado: abre o roteiro em PowerShell como administrador (medir FPS por ETW exige).
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','\"%~dp0teste-guiado.ps1\"'"
