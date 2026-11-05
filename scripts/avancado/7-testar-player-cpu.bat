@echo off
rem Igual ao 6-testar-player, mas decodificando pela CPU (para comparar).
cd /d "%~dp0.."
for %%f in (*-recorder.exe) do set "REC=%%~nf"
set "APP=%REC:-recorder=%.exe"
start "" "%APP%" --player-test %1 --sw
