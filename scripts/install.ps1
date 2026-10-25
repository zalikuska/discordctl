# discordctl installer. Usage:
#   irm https://raw.githubusercontent.com/zalikuska/discordctl/main/scripts/install.ps1 | iex
#
# What it does: downloads the latest release archive, checks its SHA-256 against SHA256SUMS.txt from the same
# release, unpacks it to a temporary folder and runs `discordctl setup` (which asks before changing anything).
# Nothing is installed silently. Optional environment variables:
#   DISCORDCTL_REPO     owner/name of the repository (default below)
#   DISCORDCTL_VERSION  a tag such as v0.1.0 (default: latest)
#   DISCORDCTL_ARGS     extra arguments for `discordctl setup`, e.g. "--yes --agent claude-code --preset read-only"
#
# If the Discord desktop app is missing, the script offers to download the official installer from discord.com,
# checks that it is signed by Discord Inc. and runs it.
& {
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = if ($env:DISCORDCTL_REPO) { $env:DISCORDCTL_REPO } else { 'zalikuska/discordctl' }
$tag = $env:DISCORDCTL_VERSION

function Step($msg) { Write-Host "  $msg" -ForegroundColor Cyan }

# Where the Squirrel-installed Discord lives: the usual folder, else the uninstall entry in the registry.
function Find-DiscordRoot {
    $default = Join-Path $env:LOCALAPPDATA 'Discord'
    if (Test-Path (Join-Path $default 'Update.exe')) { return $default }
    $cmd = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Discord' -ErrorAction SilentlyContinue).UninstallString
    if ($cmd -match '^"?(.*?)\\Update\.exe' -and (Test-Path (Join-Path $Matches[1] 'Update.exe'))) { return $Matches[1] }
    return $null
}

function Install-Discord($tmp) {
    $file = Join-Path $tmp 'DiscordSetup.exe'
    Step 'downloading the official Discord installer from discord.com'
    Invoke-WebRequest -Uri 'https://discord.com/api/download?platform=win' -OutFile $file -Headers @{ 'User-Agent' = 'discordctl-installer' }
    $sig = Get-AuthenticodeSignature $file
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O=Discord Inc\.') {
        throw "the downloaded installer is not signed by Discord Inc. (status: $($sig.Status)). Not running it."
    }
    Step 'signature ok (Discord Inc.), running the installer'
    Start-Process $file
    $deadline = (Get-Date).AddMinutes(5)
    while ((Get-Date) -lt $deadline -and -not (Find-DiscordRoot)) { Start-Sleep -Seconds 2 }
    $root = Find-DiscordRoot
    if (-not $root) { throw 'Discord did not finish installing in 5 minutes. Install it manually and run this command again.' }
    # The installer starts Discord itself; discordctl has to be the one that starts it, so close this first run.
    Start-Sleep -Seconds 8
    Get-Process -Name Discord -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) } |
        Stop-Process -Force -ErrorAction SilentlyContinue
}

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
    if (-not (Find-DiscordRoot)) {
        Write-Host ''
        Write-Host '  The Discord desktop app is not installed (the browser version cannot be controlled).' -ForegroundColor Yellow
        $answer = if ($env:DISCORDCTL_ARGS -match '(^|\s)(--yes|-y)(\s|$)') { 'y' } else { Read-Host '  Download and run the official Discord installer now? [Y/n]' }
        if ($answer -match '^(n|no)$') { throw 'Discord is required. Install it from discord.com and run this command again.' }
        Install-Discord $tmp
    }

    Step 'starting the setup wizard'
    Write-Host ''
    $extra = if ($env:DISCORDCTL_ARGS) { @($env:DISCORDCTL_ARGS -split '\s+' | Where-Object { $_ }) } else { @() }
    & (Join-Path $dir 'discordctl.exe') setup @extra
    $code = $LASTEXITCODE
    if ($code -ne 0) { throw "setup exited with code $code" }
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
}
