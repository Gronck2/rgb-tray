# Builds a release and puts the installer into dist\RGBTray-Setup-<version>.exe.
#   .\scripts\release.ps1            build the installer
#   .\scripts\release.ps1 -Install   build and silently install/update right away
param([switch]$Install)
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# cargo reports progress on stderr, which Windows PowerShell would treat as errors.
cmd /c "cargo build --release 2>&1"
if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed"; exit 1 }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force dist | Out-Null
$setup = "dist\RGBTray-Setup-$version.exe"
Copy-Item target\release\rgbtray.exe $setup -Force
Write-Host "Installer: $setup"

if ($Install) {
    # Wait for the setup process only: Start-Process -Wait would also wait for the app it launches.
    $p = [System.Diagnostics.Process]::Start((Resolve-Path $setup).Path, '--quiet')
    $p.WaitForExit()
    Write-Host "Installed $version"
}
