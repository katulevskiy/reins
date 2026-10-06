# Builds the Reins installer for Windows: a per-user .msi (WiX v4) with reins-app.exe (the app) and reins.exe.
#
#   ./scripts/package/windows.ps1 [-Version X.Y.Z] [-Target x86_64-pc-windows-msvc]
#
# Writes target\package\Reins-<version>-Windows-x64.msi and Reins-Windows-x64.msi (the same file, under the name the
# "latest" download links use). Needs the WiX v4 .NET tool (`dotnet tool install --global wix --version 4.0.6`; this
# script installs it and the Util extension when they are missing) and, for GPUI's release shaders, the Windows SDK's
# fxc.exe (found automatically, or GPUI_FXC_PATH).
#
# Signing happens only when REINS_WINDOWS_CERT (a .pfx path) and REINS_WINDOWS_CERT_PASSWORD are set (signtool from
# the Windows SDK); otherwise the installer is unsigned and SmartScreen warns once.
param(
    [string]$Version = '',
    [string]$Target = 'x86_64-pc-windows-msvc'
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $root

if (-not $Version) {
    $Version = (Select-String -Path 'crates\reins-desktop\Cargo.toml' -Pattern '^version = "([^"]+)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "not a version: $Version" }
$arch = switch -Wildcard ($Target) { 'x86_64-*' { 'x64' } 'aarch64-*' { 'arm64' } default { throw "unknown target $Target" } }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$out = Join-Path $targetDir 'package'
New-Item -ItemType Directory -Force -Path $out | Out-Null

if (-not $env:GPUI_FXC_PATH) {
    $fxc = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\fxc.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1
    if ($fxc) { $env:GPUI_FXC_PATH = $fxc.FullName }
}

Write-Host "==> building Reins $Version for $Target"
cargo build --locked --profile release-app -p reins-desktop-app -p reins-desktop --bin reins-app --bin reins --target $Target
if ($LASTEXITCODE -ne 0) { throw 'building Reins failed' }

$stage = Join-Path $out 'windows'
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item (Join-Path $targetDir "$Target\release-app\reins-app.exe") (Join-Path $stage 'reins-app.exe')
Copy-Item (Join-Path $targetDir "$Target\release-app\reins.exe") (Join-Path $stage 'reins.exe')

$pfx = $env:REINS_WINDOWS_CERT
function Sign([string]$file) {
    if (-not $pfx) { return }
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1
    & $signtool.FullName sign /f $pfx /p $env:REINS_WINDOWS_CERT_PASSWORD /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $file
    if ($LASTEXITCODE -ne 0) { throw "signing $file failed" }
}
if (-not $pfx) { Write-Host '==> REINS_WINDOWS_CERT is not set: the installer is not signed' }
Sign (Join-Path $stage 'reins-app.exe')
Sign (Join-Path $stage 'reins.exe')

# WiX v4, as a .NET tool.
$wixVersion = '4.0.6'
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    dotnet tool install --global wix --version $wixVersion
    $env:PATH = "$env:PATH;$env:USERPROFILE\.dotnet\tools"
}
wix extension add -g "WixToolset.Util.wixext/$wixVersion"
if ($LASTEXITCODE -ne 0) { throw 'cannot add the WiX Util extension' }

$wxs = Join-Path $root 'crates\reins-desktop-app\packaging\windows\Reins.wxs'
$msi = Join-Path $out "Reins-$Version-Windows-$arch.msi"
wix build -arch $arch -ext WixToolset.Util.wixext `
    -d "Version=$Version" `
    -d "AppExe=$(Join-Path $stage 'reins-app.exe')" `
    -d "CliExe=$(Join-Path $stage 'reins.exe')" `
    -d "IconFile=$(Join-Path $root 'crates\reins-desktop-app\packaging\windows\Reins.ico')" `
    -o $msi $wxs
if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }
Sign $msi
Copy-Item -Force $msi (Join-Path $out "Reins-Windows-$arch.msi")
Get-Item $msi, (Join-Path $out "Reins-Windows-$arch.msi") | ForEach-Object { Write-Host ("    {0}  {1:N0} bytes" -f $_.Name, $_.Length) }
