@echo off
cd /d "%~dp0.."
for %%f in (*-recorder.exe) do "%%f" --window VALORANT
pause
