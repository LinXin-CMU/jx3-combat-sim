param(
    [Parameter(Mandatory = $true)]
    [string]$ManifestPath,

    [string]$BookId = '56756131',
    [string]$BookTitle = '朔风卷雪',
    [string]$OwnerName = '神光毓逍遥',
    [string]$OwnerLogin = 'sgyxy',
    [string]$OwnerId = '29452228',
    [string]$BookCreatedAt = '2024-10-19T01:12:50Z',
    [string]$BookUpdatedAt = '2026-08-26T11:53:50Z'
)

$ErrorActionPreference = 'Stop'

function ConvertTo-YamlString {
    param([AllowEmptyString()][string]$Value)
    return ($Value | ConvertTo-Json -Compress)
}

$manifestFile = (Resolve-Path -LiteralPath $ManifestPath).Path
$vaultRoot = Split-Path -Parent $manifestFile
$manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
$fields = [ordered]@{
    yuque_book_id = $BookId
    yuque_book_title = $BookTitle
    yuque_book_owner_name = $OwnerName
    yuque_book_owner_login = $OwnerLogin
    yuque_book_owner_id = $OwnerId
    yuque_book_created_at = $BookCreatedAt
    yuque_book_updated_at = $BookUpdatedAt
}

foreach ($entry in $manifest.entries) {
    $documentPath = Join-Path $vaultRoot ([string]$entry.output)
    $content = Get-Content -LiteralPath $documentPath -Raw
    if ($content -notmatch '(?s)\A---\r?\n(?<frontmatter>.*?)\r?\n---\r?\n(?<body>.*)\z') {
        throw "Invalid frontmatter: $documentPath"
    }
    $frontmatter = $Matches['frontmatter']
    $body = $Matches['body']
    foreach ($name in $fields.Keys) {
        $frontmatter = [regex]::Replace($frontmatter, "(?m)^$([regex]::Escape($name)):\s*.*\r?\n?", '')
    }
    $metadataLines = @($fields.GetEnumerator() | ForEach-Object { $_.Key + ': ' + (ConvertTo-YamlString ([string]$_.Value)) }) -join "`n"
    $frontmatter = $frontmatter -replace '(?m)^yuque_catalog_path:', ($metadataLines + "`nyuque_catalog_path:")
    $updated = "---`n$($frontmatter.TrimEnd())`n---`n$body"
    [IO.File]::WriteAllText($documentPath, $updated, [Text.UTF8Encoding]::new($false))
    foreach ($item in $fields.GetEnumerator()) {
        $entry | Add-Member -NotePropertyName $item.Key -NotePropertyValue ([string]$item.Value) -Force
    }
}

foreach ($item in $fields.GetEnumerator()) {
    $manifest | Add-Member -NotePropertyName $item.Key -NotePropertyValue ([string]$item.Value) -Force
}
$manifest.schema_version = [Math]::Max([int]$manifest.schema_version, 4)
$manifest | Add-Member -NotePropertyName metadata_enriched_at -NotePropertyValue ([DateTimeOffset]::Now.ToString('o')) -Force
[IO.File]::WriteAllText($manifestFile, ($manifest | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))

[pscustomobject]@{
    updated_documents = @($manifest.entries).Count
    manifest = $manifestFile
    schema_version = $manifest.schema_version
} | ConvertTo-Json
