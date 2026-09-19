@echo off
cd /d "%~dp0"
powershell -NoProfile -Command "Start-Process pythonw -ArgumentList 'run.py' -WorkingDirectory '%~dp0' -Verb RunAs"
