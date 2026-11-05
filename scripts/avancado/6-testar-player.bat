@echo off
rem Teste do player (Fase 3). Arraste um clipe .mp4 sobre este arquivo,
rem ou abra direto para tocar o clipe mais recente da biblioteca.
cd /d "%~dp0.."
for %%f in (*-recorder.exe) do set "REC=%%~nf"
set "APP=%REC:-recorder=%.exe"
start "" "%APP%" --player-test %1
