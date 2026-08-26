param(
    [Parameter(Mandatory = $true)]
    [string]$ManifestPath,

    [Parameter(Mandatory = $true)]
    [string]$ProgressPath
)

$ErrorActionPreference = 'Stop'

function ConvertTo-YamlString {
    param([AllowEmptyString()][string]$Value)
    return ($Value | ConvertTo-Json -Compress)
}

function Get-SourceSite {
    param([string]$Url)
    try { return ([Uri]$Url).DnsSafeHost.ToLowerInvariant() } catch { return '' }
}

function Get-MarkdownBody {
    param([string]$Text)
    if ($Text -match '(?s)\A---\s*\r?\n.*?\r?\n---\s*\r?\n?(?<body>.*)\z') {
        return $Matches['body']
    }
    return $Text
}

function Get-Sha256 {
    param([string]$Text)
    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
}

function ConvertTo-IsoDate {
    param($Value)
    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) { return '' }
    if ($Value -is [DateTime]) { return $Value.ToUniversalTime().ToString('o') }
    if ($Value -is [DateTimeOffset]) { return $Value.ToUniversalTime().ToString('o') }
    try { return ([DateTimeOffset]::Parse([string]$Value)).ToUniversalTime().ToString('o') } catch { return [string]$Value }
}

$manifestFile = (Resolve-Path -LiteralPath $ManifestPath).Path
$progressFile = (Resolve-Path -LiteralPath $ProgressPath).Path
$manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
$vaultRoot = Split-Path -Parent $manifestFile
$knowledgeBaseUrl = [string]$manifest.knowledge_base_url
$progress = @(Get-Content -LiteralPath $progressFile -Raw | ConvertFrom-Json)
$metadataByUuid = @{}
$metadataByPath = @{}
foreach ($item in $progress) {
    if ($item.toc.type -eq 'DOC' -and $item.toc.uuid) {
        $metadataByUuid[[string]$item.toc.uuid] = $item
        $metadataByPath[([string]$item.path -replace '\\', '/')] = $item
    }
}

$capturedAt = [DateTimeOffset]::Now.ToString('o')
$rewritten = 0
foreach ($entry in $manifest.entries) {
    $path = Join-Path $vaultRoot ([string]$entry.output)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing document: $path" }
    $text = Get-Content -LiteralPath $path -Raw
    $body = (Get-MarkdownBody $text).TrimStart("`r", "`n")
    $catalogPath = "$($entry.season) / $($entry.category) / $($entry.title)"
    $sourceUrl = [string]$entry.source
    $sourceSite = Get-SourceSite $sourceUrl

    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add('---')
    $lines.Add('title: ' + (ConvertTo-YamlString ([string]$entry.title)))
    $lines.Add('kind: ' + [string]$entry.kind)
    $lines.Add('season: ' + (ConvertTo-YamlString ([string]$entry.season)))
    $lines.Add('category: ' + (ConvertTo-YamlString ([string]$entry.category)))
    $lines.Add('source_url: ' + (ConvertTo-YamlString $sourceUrl))
    $lines.Add('source_site: ' + (ConvertTo-YamlString $sourceSite))
    $lines.Add('yuque_book_url: ' + (ConvertTo-YamlString $knowledgeBaseUrl))
    $lines.Add('yuque_catalog_path: ' + (ConvertTo-YamlString $catalogPath))

    if ($entry.kind -eq 'yuque_document') {
        $uuid = [string]$entry.yuque_uuid
        $sourceFile = [string]$entry.source_file -replace '\\', '/'
        $meta = if ($uuid) { $metadataByUuid[$uuid] } else { $metadataByPath[$sourceFile] }
        if (-not $meta) { throw "Missing Yuque metadata for: $sourceFile" }
        $lines.Add('yuque_entry_type: document')
        $lines.Add('yuque_url: ' + (ConvertTo-YamlString $sourceUrl))
        $lines.Add('yuque_slug: ' + (ConvertTo-YamlString ([string]$meta.toc.url)))
        $lines.Add('yuque_uuid: ' + (ConvertTo-YamlString ([string]$meta.toc.uuid)))
        $lines.Add('yuque_doc_id: ' + (ConvertTo-YamlString ([string]$meta.toc.doc_id)))
        $lines.Add('source_created_at: ' + (ConvertTo-YamlString (ConvertTo-IsoDate $meta.createAt)))
        $lines.Add('source_first_published_at: ' + (ConvertTo-YamlString (ConvertTo-IsoDate $meta.firstPublishedAt)))
        $lines.Add('source_published_at: ' + (ConvertTo-YamlString (ConvertTo-IsoDate $meta.publishedAt)))
        $lines.Add('source_updated_at: ' + (ConvertTo-YamlString (ConvertTo-IsoDate $meta.contentUpdatedAt)))
        $entry | Add-Member -NotePropertyName yuque_slug -NotePropertyValue ([string]$meta.toc.url) -Force
        $entry | Add-Member -NotePropertyName yuque_uuid -NotePropertyValue ([string]$meta.toc.uuid) -Force
        $entry | Add-Member -NotePropertyName yuque_doc_id -NotePropertyValue ([string]$meta.toc.doc_id) -Force
        $entry | Add-Member -NotePropertyName yuque_url -NotePropertyValue $sourceUrl -Force
        $entry | Add-Member -NotePropertyName created_at -NotePropertyValue (ConvertTo-IsoDate $meta.createAt) -Force
        $entry | Add-Member -NotePropertyName first_published_at -NotePropertyValue (ConvertTo-IsoDate $meta.firstPublishedAt) -Force
        $entry | Add-Member -NotePropertyName published_at -NotePropertyValue (ConvertTo-IsoDate $meta.publishedAt) -Force
        $entry | Add-Member -NotePropertyName updated_at -NotePropertyValue (ConvertTo-IsoDate $meta.contentUpdatedAt) -Force
    } else {
        $lines.Add('yuque_entry_type: external_link')
        $lines.Add('yuque_url: ' + (ConvertTo-YamlString $knowledgeBaseUrl))
        $lines.Add('external_url: ' + (ConvertTo-YamlString $sourceUrl))
        $lines.Add('mirror_status: pending')
        $body = "# $($entry.title)`n`n> 此条目在原语雀知识库中是站外链接，尚未镜像正文。`n`n- [打开外部原文]($sourceUrl)`n- [打开语雀知识库]($knowledgeBaseUrl)`n"
        $entry | Add-Member -NotePropertyName yuque_url -NotePropertyValue $knowledgeBaseUrl -Force
        $entry | Add-Member -NotePropertyName external_url -NotePropertyValue $sourceUrl -Force
    }

    $bodyHash = Get-Sha256 $body
    $lines.Add('captured_at: ' + (ConvertTo-YamlString $capturedAt))
    $lines.Add('content_sha256: ' + (ConvertTo-YamlString $bodyHash))
    $lines.Add('---')
    $normalized = ($lines -join "`n") + "`n`n" + $body.TrimEnd() + "`n"
    [IO.File]::WriteAllText($path, $normalized, [Text.UTF8Encoding]::new($false))

    $entry | Add-Member -NotePropertyName source_site -NotePropertyValue $sourceSite -Force
    $entry | Add-Member -NotePropertyName yuque_book_url -NotePropertyValue $knowledgeBaseUrl -Force
    $entry | Add-Member -NotePropertyName yuque_catalog_path -NotePropertyValue $catalogPath -Force
    $entry | Add-Member -NotePropertyName captured_at -NotePropertyValue $capturedAt -Force
    $entry | Add-Member -NotePropertyName content_sha256 -NotePropertyValue $bodyHash -Force
    $rewritten++
}

$manifest.schema_version = 2
$manifest.generated_at = $capturedAt
[IO.File]::WriteAllText($manifestFile, ($manifest | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

[pscustomobject]@{
    rewritten = $rewritten
    manifest = $manifestFile
    schema_version = $manifest.schema_version
} | ConvertTo-Json
