# Живая проверка записи: сообщение, ответ, правка, реакция, удаление. ШЛЁТ НАСТОЯЩИЕ СООБЩЕНИЯ в -Channel,
# поэтому канал обязателен и без -Yes скрипт только показывает, что собирается сделать.
param(
    [Parameter(Mandatory)][string]$Channel,
    [switch]$Yes
)
$ErrorActionPreference = 'Stop'
$d = Join-Path $PSScriptRoot '..\target\release\discordctl.exe'
$tag = "[discordctl test $(Get-Date -Format 'HH:mm:ss')]"
if (-not $Yes) {
    "Would send in ${Channel}: '$tag', a reply, an edit, a reaction, then delete both. Re-run with -Yes."
    exit 0
}
function Step($name, [scriptblock]$body) { $r = & $body; "{0,-14} ok" -f $name; $r }

$sent = Step 'send' { (& $d message send $Channel "$tag first" --json) -join "`n" | ConvertFrom-Json }
$first = $sent.message.id
if ($sent.message.content -ne "$tag first") { throw 'content mismatch' }
$reply = Step 'reply' { (& $d message send $Channel "$tag reply" --reply-to $first --json) -join "`n" | ConvertFrom-Json }
if ($reply.message.reply_to.message_id -ne $first) { throw 'reply_to mismatch' }
$edit = Step 'edit' { (& $d message edit $Channel $first "$tag first (edited)" --json) -join "`n" | ConvertFrom-Json }
if (-not $edit.message.edited_timestamp) { throw 'not marked as edited' }
Step 'react' { & $d message react $Channel $first '👍' | Out-Null }
$seen = ((& $d read $Channel -n 5 --json) -join "`n" | ConvertFrom-Json).messages | Where-Object { $_.id -eq $first }
if (-not ($seen.reactions | Where-Object { $_.me })) { throw 'own reaction not visible in history' }
Step 'unreact' { & $d message react $Channel $first '👍' --remove | Out-Null }
Step 'delete reply' { & $d message delete $Channel $reply.message.id | Out-Null }
Step 'delete first' { & $d message delete $Channel $first | Out-Null }
$left = ((& $d read $Channel -n 5 --json) -join "`n" | ConvertFrom-Json).messages | Where-Object { $_.id -in @($first, $reply.message.id) }
if ($left) { throw 'test messages are still there' }
'all steps ok; test messages removed'
