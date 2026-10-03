@echo off
setlocal

echo ============================================
echo  Deploy Tool Build
echo ============================================

where pyinstaller >nul 2>&1
if errorlevel 1 (
    echo [ERROR] pyinstaller not found
    echo Run: pip install pyinstaller
    pause
    exit /b 1
)

if not exist config.template.yaml (
    echo [ERROR] config.template.yaml not found
    pause
    exit /b 1
)

if not exist assets\app.ico (
    echo [WARN] assets\app.ico not found, exe will use default icon
)

if exist DeployTool.spec del DeployTool.spec
if exist build rmdir /s /q build
if exist dist rmdir /s /q dist

echo [1/2] Building...
pyinstaller --noconfirm --clean --onefile --windowed ^
  --name TA_DEPLOY_TOOL ^
  --icon assets\app.ico ^
  --add-data "config.template.yaml;." ^
  --add-data "assets/app.ico;assets" ^
  --collect-submodules asyncssh ^
  --collect-submodules qasync ^
  --hidden-import PySide6.QtCore ^
  --hidden-import PySide6.QtGui ^
  --hidden-import PySide6.QtWidgets ^
  --exclude-module tkinter ^
  --exclude-module matplotlib ^
  --exclude-module numpy ^
  --exclude-module pandas ^
  --exclude-module scipy ^
  --exclude-module PySide6.QtWebEngineCore ^
  --exclude-module PySide6.QtWebEngineWidgets ^
  --exclude-module PySide6.QtQml ^
  --exclude-module PySide6.QtQuick ^
  --exclude-module PySide6.QtMultimedia ^
  --exclude-module PySide6.QtCharts ^
  --exclude-module PySide6.QtSql ^
  --exclude-module PySide6.QtTest ^
  --exclude-module PySide6.QtNetworkAuth ^
  --exclude-module PySide6.QtPdf ^
  --exclude-module PySide6.QtBluetooth ^
  --exclude-module PySide6.QtDesigner ^
  --exclude-module PySide6.QtHelp ^
  --exclude-module PySide6.QtNfc ^
  --exclude-module PySide6.QtPositioning ^
  --exclude-module PySide6.QtRemoteObjects ^
  --exclude-module PySide6.QtSensors ^
  --exclude-module PySide6.QtSerialPort ^
  --exclude-module PySide6.QtSpatialAudio ^
  --exclude-module PySide6.QtStateMachine ^
  --exclude-module PySide6.QtSvgWidgets ^
  --exclude-module PySide6.QtTextToSpeech ^
  --exclude-module PySide6.QtUiTools ^
  --exclude-module PySide6.QtWebChannel ^
  --exclude-module PySide6.QtWebSockets ^
  --exclude-module PySide6.Qt3DCore ^
  --exclude-module PySide6.QtDataVisualization ^
  main.py

if errorlevel 1 (
    echo [ERROR] Build failed
    pause
    exit /b 1
)

echo [2/2] Build success
echo Output: dist\DeployTool.exe
pause