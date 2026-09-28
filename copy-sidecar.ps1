# Script: copy-sidecar.ps1
# Copie le binaire mmc-batch-backend compilé dans le dossier binaries/ de Tauri
# avec le nom de triple cible correct pour Windows x64.
#
# Usage:
#   .\copy-sidecar.ps1
#   .\copy-sidecar.ps1 -Clean  # Pour nettoyer le cache Cargo avant de compiler

param(
    [switch]$Clean,
    [switch]$BuildOnly
)

$BackendDir = Join-Path $PSScriptRoot "BackendRust"
$FrontendDir = Join-Path $PSScriptRoot "Frontend"
$BinariesDir = Join-Path $FrontendDir "src-tauri\binaries"
$SidecarName = "mmc-batch-backend-x86_64-pc-windows-msvc.exe"
$SidecarDest = Join-Path $BinariesDir $SidecarName

$TargetExeCandidates = @(
    "C:\cargo-build\mmc-batch-backend\release\mmc-batch-backend.exe",
    (Join-Path $BackendDir "target\release\mmc-batch-backend.exe")
)

Write-Host "=== MMC-Batch Sidecar Copy Script ===" -ForegroundColor Cyan

# Arreter les backends encore lances : un ancien backend garde le port 8000
# (le frontend recoit alors des 404) et verrouille le .exe (la copie echoue).
$Running = Get-Process -Name "mmc-batch-backend*" -ErrorAction SilentlyContinue
if ($Running) {
    $Running | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    Write-Host "[OK] Ancien backend arrete ($($Running.Count) processus)" -ForegroundColor Green
}

# Créer le dossier binaries s'il n'existe pas
if (-not (Test-Path $BinariesDir)) {
    New-Item -ItemType Directory -Force -Path $BinariesDir | Out-Null
    Write-Host "[OK] Dossier binaries cree: $BinariesDir" -ForegroundColor Green
}

# Optionnel: cargo clean
if ($Clean) {
    Write-Host "[INFO] Nettoyage du cache Cargo..." -ForegroundColor Yellow
    Push-Location $BackendDir
    cargo clean
    Pop-Location
}

# Compiler le backend
Write-Host "[INFO] Compilation du backend Rust en release..." -ForegroundColor Yellow
Push-Location $BackendDir
$env:CARGO_INCREMENTAL = "0"
cargo build --release
$BuildResult = $LASTEXITCODE
Pop-Location

if ($BuildResult -ne 0) {
    Write-Host "[ERREUR] La compilation a echoue (code $BuildResult)." -ForegroundColor Red
    exit 1
}

# Chercher l'exécutable compilé (apres la compilation, pour prendre le plus recent)
$TargetExe = $TargetExeCandidates |
    Where-Object { Test-Path $_ } |
    Sort-Object { (Get-Item $_).LastWriteTime } -Descending |
    Select-Object -First 1

# Copier le binaire
if ($TargetExe -and (Test-Path $TargetExe)) {
    Copy-Item -Path $TargetExe -Destination $SidecarDest -Force
    $Size = (Get-Item $SidecarDest).Length / 1MB
    Write-Host "[OK] Sidecar copie: $SidecarDest ($([math]::Round($Size, 2)) MB)" -ForegroundColor Green
} else {
    Write-Host "[ERREUR] Binaire introuvable: $($TargetExeCandidates -join ', ')" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "=== Pret! Vous pouvez maintenant lancer: ===" -ForegroundColor Cyan
Write-Host "  cd Frontend" -ForegroundColor White
Write-Host "  npm run tauri dev     # Mode developpement" -ForegroundColor White
Write-Host "  npm run tauri build   # Build production (.exe)" -ForegroundColor White
