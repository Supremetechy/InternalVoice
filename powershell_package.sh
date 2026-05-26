# powershell_package.sh - InternalVoice Windows Installation & Wrapper Script
# Note: This is a PowerShell script. Run it as: powershell -ExecutionPolicy Bypass -File powershell_package.sh

$ErrorActionPreference = "Stop"

$BINARY_NAME = "internalvoice"
$TARGET_PATH = ".\target\release\$BINARY_NAME.exe"
$WRAPPER_NAME = "iv.bat"

function Check-Rust {
    if (!(Get-Command "rustc" -ErrorAction SilentlyContinue)) {
        Write-Host "Error: Rust compiler (rustc) not found." -ForegroundColor Red
        $choice = Read-Host "Would you like to install Rust now? (y/n)"
        if ($choice -eq 'y') {
            Write-Host "Downloading Rust installer (rustup)..." -ForegroundColor Cyan
            $installerPath = "$env:TEMP\rustup-init.exe"
            Invoke-WebRequest -Uri "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" -OutFile $installerPath
            Write-Host "Running installer..." -ForegroundColor Cyan
            Start-Process -FilePath $installerPath -ArgumentList "-y" -Wait
            Remove-Item $installerPath
            $env:Path += ";$HOME\.cargo\bin"
            Write-Host "Rust installed successfully." -ForegroundColor Green
        } else {
            Write-Host "Rust is required to build InternalVoice. Please install it and try again." -ForegroundColor Red
            exit 1
        }
    }
}

function Build-App {
    Write-Host "Building $BINARY_NAME in release mode..." -ForegroundColor Cyan
    cargo build --release
}

function Run-Setup {
    Write-Host "Running configuration wizard..." -ForegroundColor Cyan
    & $TARGET_PATH --setup
}

function Create-Wrapper {
    Write-Host "Creating '$WRAPPER_NAME' command wrapper..." -ForegroundColor Cyan
    $wrapperContent = @"
@echo off
set DIR=%~dp0
pushd %DIR%
target\release\$BINARY_NAME.exe %*
popd
"@
    $wrapperContent | Out-File -FilePath "$WRAPPER_NAME" -Encoding ASCII
    Write-Host "  [✓] Wrapper created: .\$WRAPPER_NAME" -ForegroundColor Green
}

# --- Main Execution ---

Write-Host "── InternalVoice Packaging & Setup ──" -ForegroundColor Cyan
Write-Host ""

Check-Rust
Build-App
Run-Setup
Create-Wrapper

Write-Host ""
Write-Host "Successfully structured packaging for InternalVoice." -ForegroundColor Green
Write-Host "You can now run the program using: .\$WRAPPER_NAME" -ForegroundColor Green
