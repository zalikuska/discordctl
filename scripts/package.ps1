# Builds a release archive: dist\discordctl-<version>-windows-x64.zip and dist\SHA256SUMS.txt
#   .\scripts\package.ps1            # build + package
#   .\scripts\package.ps1 -NoBuild   # package what is already in target\release
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not $NoBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
$version = (Select-String -Path (Join-Path $repo 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$name = "discordctl-$version-windows-x64"
$stage = Join-Path $repo "dist\$name"
Remove-Item (Join-Path $repo 'dist') -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($exe in 'discordctl', 'discordctl-shim', 'discordctl-daemon', 'discordctl-mcp') {
    $src = Join-Path $repo "target\release\$exe.exe"
    if (-not (Test-Path $src)) { throw "$src is missing" }
    Copy-Item $src $stage
}
foreach ($f in 'README.md', 'README.ru.md', 'LICENSE', 'CHANGELOG.md', 'SECURITY.md') { Copy-Item (Join-Path $repo $f) $stage }
Copy-Item (Join-Path $repo 'examples') $stage -Recurse

$zip = Join-Path $repo "dist\$name.zip"
Compress-Archive -Path "$stage\*" -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $name.zip" | Set-Content (Join-Path $repo 'dist\SHA256SUMS.txt') -Encoding ascii
Write-Host "built $zip"
Write-Host "sha256 $hash"
