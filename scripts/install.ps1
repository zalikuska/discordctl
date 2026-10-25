# discordctl installer. Usage:
#   irm https://raw.githubusercontent.com/zalikuska/discordctl/main/scripts/install.ps1 | iex
#
# What it does: downloads the latest release archive, checks its SHA-256 against SHA256SUMS.txt from the same
# release, unpacks it to a temporary folder and runs `discordctl setup` (which asks before changing anything).
# Nothing is installed silently. Optional environment variables:
#   DISCORDCTL_REPO     owner/name of the repository (default below)
#   DISCORDCTL_VERSION  a tag such as v0.1.0 (default: latest)
& {
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = if ($env:DISCORDCTL_REPO) { $env:DISCORDCTL_REPO } else { 'zalikuska/discordctl' }
$tag = $env:DISCORDCTL_VERSION

function Step($msg) { Write-Host "  $msg" -ForegroundColor Cyan }

if ($env:OS -ne 'Windows_NT') { throw 'discordctl runs on Windows only.' }

Write-Host "`ndiscordctl installer" -ForegroundColor White
$api = if ($tag) { "https://api.github.com/repos/$repo/releases/tags/$tag" } else { "https://api.github.com/repos/$repo/releases/latest" }
Step "looking up the release ($repo, $(if ($tag) { $tag } else { 'latest' }))"
$release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'discordctl-installer' }
$zipAsset = $release.assets | Where-Object { $_.name -like 'discordctl-*-windows-x64.zip' } | Select-Object -First 1
$sumAsset = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' } | Select-Object -First 1
if (-not $zipAsset -or -not $sumAsset) { throw "release $($release.tag_name) has no Windows archive or checksum file" }

$tmp = Join-Path $env:TEMP ("discordctl-install-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    $zip = Join-Path $tmp $zipAsset.name
    Step "downloading $($zipAsset.name) ($([math]::Round($zipAsset.size / 1MB, 1)) MB)"
    Invoke-WebRequest -Uri $zipAsset.browser_download_url -OutFile $zip -Headers @{ 'User-Agent' = 'discordctl-installer' }
    $sums = (Invoke-WebRequest -Uri $sumAsset.browser_download_url -Headers @{ 'User-Agent' = 'discordctl-installer' }).Content
    $expected = ($sums -split "`n" | Where-Object { $_ -match [regex]::Escape($zipAsset.name) } | Select-Object -First 1) -replace '\s.*', ''
    $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected.Trim().ToLower() -ne $actual) { throw "checksum mismatch for $($zipAsset.name): expected '$expected', got '$actual'. Not installing." }
    Step "checksum ok ($($actual.Substring(0, 12))...)"

    $dir = Join-Path $tmp 'pkg'
    Expand-Archive -Path $zip -DestinationPath $dir
    Step 'starting the setup wizard'
    Write-Host ''
    & (Join-Path $dir 'discordctl.exe') setup
    $code = $LASTEXITCODE
    if ($code -ne 0) { throw "setup exited with code $code" }
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
}
