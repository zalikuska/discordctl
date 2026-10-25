# Живая проверка чтения на настоящем Discord (нужен запущенный демон и активный хук).
# Печатает только структуру результата, без текста сообщений: чужие переписки в логах не нужны.
param(
    [Parameter(Mandatory)][string]$Channel,   # например '@alice' или 'Server/general'
    [int]$Limit = 120
)
$ErrorActionPreference = 'Stop'
$d = Join-Path $PSScriptRoot '..\target\release\discordctl.exe'
$sw = [Diagnostics.Stopwatch]::StartNew()
$j = (& $d read $Channel -n $Limit --json) -join "`n" | ConvertFrom-Json
$sw.Stop()
$ids = @($j.messages.id | ForEach-Object { [decimal]$_ })
$checks = [ordered]@{
    'got requested count'  = ($j.messages.Count -eq $Limit)
    'ascending by id'      = (($ids -join ',') -eq (($ids | Sort-Object) -join ','))
    'ids unique'           = (($ids | Sort-Object -Unique).Count -eq $ids.Count)
    'all in one channel'   = (@($j.messages.channel_id | Sort-Object -Unique).Count -eq 1)
    'has next cursor'      = [bool]$j.next
}
"elapsed ms: $($sw.ElapsedMilliseconds), messages: $($j.messages.Count), has_more: $($j.has_more), direction: $($j.direction)"
"channel: $($j.channel.kind) / $($j.channel.name)"
"range: $($j.messages[0].timestamp) .. $($j.messages[-1].timestamp)"
"attachments: $(@($j.messages | Where-Object { $_.attachments.Count }).Count), reactions: $(@($j.messages | Where-Object { $_.reactions.Count }).Count), replies: $(@($j.messages | Where-Object { $_.reply_to }).Count), embeds: $(@($j.messages | Where-Object { $_.embeds.Count }).Count)"
"message keys: $($j.messages[0].PSObject.Properties.Name -join ',')"
$checks.GetEnumerator() | ForEach-Object { "{0,-22} {1}" -f $_.Key, $(if ($_.Value) { 'ok' } else { 'FAIL' }) }

# Следующая страница назад по курсору не должна пересекаться с первой.
$j2 = (& $d read $Channel -n 20 --before $j.next --json) -join "`n" | ConvertFrom-Json
$overlap = @($j2.messages.id | Where-Object { $j.messages.id -contains $_ }).Count
"page 2: $($j2.messages.Count) messages, overlap with page 1: $overlap, newest of page 2 older than oldest of page 1: $([decimal]$j2.messages[-1].id -lt [decimal]$j.messages[0].id)"
if ($checks.Values -contains $false -or $overlap -ne 0) { exit 1 }
