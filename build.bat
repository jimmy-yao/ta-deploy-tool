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

if not exist TA_DEPLOY_TOOL.spec (
    echo [ERROR] TA_DEPLOY_TOOL.spec not found
    pause
    exit /b 1
)

if exist build rmdir /s /q build
if exist dist rmdir /s /q dist

echo [1/2] Building with TA_DEPLOY_TOOL.spec...
pyinstaller --noconfirm TA_DEPLOY_TOOL.spec

if errorlevel 1 (
    echo [ERROR] Build failed
    pause
    exit /b 1
)

echo [2/2] Build success
echo Output: dist\TA_DEPLOY_TOOL.exe
pause