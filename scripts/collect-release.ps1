# Collects the build outputs into release\ and writes SHA256SUMS.txt.
# Names follow the app version so several versions can sit side by side.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$conf = Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$ver = $conf.version
$rel = Join-Path $root 'release'
$exe = Join-Path $root 'src-tauri\target\release\pulsetray.exe'
$setup = Join-Path $root "src-tauri\target\release\bundle\nsis\PulseTray_${ver}_x64-setup.exe"

foreach ($f in @($exe, $setup)) {
    if (-not (Test-Path $f)) { throw "Missing build output: $f" }
}

if (Test-Path $rel) { Remove-Item $rel -Recurse -Force }
New-Item -ItemType Directory -Path $rel | Out-Null

Copy-Item $setup (Join-Path $rel "PulseTray-Setup-$ver.exe")
Copy-Item $exe (Join-Path $rel "PulseTray-$ver-portable.exe")

# The self-updater extracts "pulsetray.exe" from this zip, so the entry must have exactly that name.
$stage = Join-Path ([System.IO.Path]::GetTempPath()) "pulsetray-zip-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item $exe (Join-Path $stage 'pulsetray.exe')
Compress-Archive -Path (Join-Path $stage 'pulsetray.exe') -DestinationPath (Join-Path $rel "PulseTray-$ver-update.zip") -CompressionLevel Optimal
Remove-Item $stage -Recurse -Force

$lines = Get-ChildItem $rel -File | Where-Object { $_.Name -ne 'SHA256SUMS.txt' } | ForEach-Object {
    $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    "{0}  {1}" -f $h, $_.Name
}
$lines | Set-Content (Join-Path $rel 'SHA256SUMS.txt') -Encoding ascii
$lines
