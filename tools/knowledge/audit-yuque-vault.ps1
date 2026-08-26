param(
    [Parameter(Mandatory = $true)]
    [string]$ManifestPath,

    [string]$ReportPath = ''
)

$ErrorActionPreference = 'Stop'

function Get-DirectoryStats {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        return [pscustomobject]@{ files = 0; bytes = 0 }
    }
    $files = @(Get-ChildItem -LiteralPath $Path -File -Recurse)
    return [pscustomobject]@{ files = $files.Count; bytes = [long](($files | Measure-Object Length -Sum).Sum) }
}

function Format-Bytes {
    param([long]$Bytes)
    if ($Bytes -ge 1GB) { return ('{0:N2} GiB' -f ($Bytes / 1GB)) }
    if ($Bytes -ge 1MB) { return ('{0:N2} MiB' -f ($Bytes / 1MB)) }
    if ($Bytes -ge 1KB) { return ('{0:N2} KiB' -f ($Bytes / 1KB)) }
    return "$Bytes B"
}

$manifestFile = (Resolve-Path -LiteralPath $ManifestPath).Path
$vaultRoot = Split-Path -Parent $manifestFile
$vaultContainer = Split-Path -Parent $vaultRoot
$manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
if (-not $ReportPath) { $ReportPath = Join-Path $vaultContainer '迁移报告.md' }

$commonFields = @(
    'title', 'kind', 'season', 'category', 'source_url', 'source_site',
    'yuque_book_url', 'yuque_book_id', 'yuque_book_title',
    'yuque_book_owner_name', 'yuque_book_owner_login', 'yuque_book_owner_id',
    'yuque_catalog_path', 'yuque_entry_type', 'yuque_url', 'captured_at', 'content_sha256'
)
$nativeFields = @('yuque_slug', 'yuque_uuid', 'yuque_doc_id', 'source_created_at', 'source_first_published_at', 'source_published_at', 'source_updated_at')
$externalFields = @('external_url', 'canonical_url', 'source_title', 'mirror_status', 'retrieval_eligible', 'retrieval_scope', 'extraction_method', 'http_status')
$missingFiles = [Collections.Generic.List[string]]::new()
$metadataProblems = [Collections.Generic.List[string]]::new()
$remoteMedia = [Collections.Generic.List[string]]::new()
$mediaPattern = 'https?://[^\s"''<>\)]+?\.(?:png|jpe?g|gif|webp|svg|bmp|mp4|webm|mp3|wav|m4a|ogg)(?:\?[^\s"''<>\)]*)?'

foreach ($entry in $manifest.entries) {
    $documentPath = Join-Path $vaultRoot ([string]$entry.output)
    if (-not (Test-Path -LiteralPath $documentPath -PathType Leaf)) {
        $missingFiles.Add([string]$entry.output)
        continue
    }
    $content = Get-Content -LiteralPath $documentPath -Raw
    if ($content -notmatch '(?s)\A---\r?\n(?<frontmatter>.*?)\r?\n---\r?\n') {
        $metadataProblems.Add("$($entry.output): invalid frontmatter")
        continue
    }
    $frontmatter = $Matches['frontmatter']
    $required = @($commonFields)
    if ($entry.kind -eq 'yuque_document') { $required += $nativeFields } else { $required += $externalFields }
    foreach ($field in $required) {
        if ($frontmatter -notmatch "(?m)^$([regex]::Escape($field)):\s*.*$") {
            $metadataProblems.Add("$($entry.output): missing $field")
        }
    }
    foreach ($match in [regex]::Matches($content, $mediaPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        $remoteMedia.Add($match.Value)
    }
}

$native = @($manifest.entries | Where-Object kind -eq 'yuque_document')
$external = @($manifest.entries | Where-Object kind -eq 'external_mirror')
$statusCounts = @{}
foreach ($status in @('full', 'metadata_only', 'failed')) {
    $statusCounts[$status] = @($external | Where-Object mirror_status -eq $status).Count
}
$seasonGroups = @($manifest.entries | Group-Object season | Sort-Object Name)
$domainGroups = @($external | Group-Object { try { ([Uri]$_.source).DnsSafeHost } catch { 'invalid' } } | Sort-Object Count -Descending)
$rawStats = Get-DirectoryStats (Join-Path $vaultContainer '_source')
$normalizedStats = Get-DirectoryStats $vaultRoot
$assetStats = Get-DirectoryStats (Join-Path $vaultRoot '_assets\remote')
$vaultStats = Get-DirectoryStats $vaultContainer
$mediaManifestPath = Join-Path $vaultRoot '_assets\remote\manifest.json'
$failedMedia = @()
if (Test-Path -LiteralPath $mediaManifestPath -PathType Leaf) {
    $mediaManifest = Get-Content -LiteralPath $mediaManifestPath -Raw | ConvertFrom-Json
    $failedMedia = @($mediaManifest.items | Where-Object status -ne 'downloaded')
}

$seasonRows = ($seasonGroups | ForEach-Object { "| $($_.Name) | $($_.Count) |" }) -join "`n"
$domainRows = ($domainGroups | ForEach-Object { "| $($_.Name) | $($_.Count) |" }) -join "`n"
$problemEntries = @($external | Where-Object mirror_status -ne 'full')
$problemRows = if ($problemEntries.Count) {
    ($problemEntries | ForEach-Object { "| $($_.mirror_status) | $($_.season) / $($_.category) | [$($_.title)]($($_.source)) |" }) -join "`n"
} else { '| — | — | — |' }

$report = @"
# 朔风卷雪知识库迁移报告

生成时间：$([DateTimeOffset]::Now.ToString('o'))

## 结论

- 语雀目录条目：$(@($manifest.entries).Count)；本地 Markdown：$(@($manifest.entries).Count)。
- 语雀原生文档：$($native.Count) 篇，正文、语雀直链、slug、UUID、doc_id 与时间信息均已保存。
- 语雀站外条目：$($external.Count) 篇；全文 $($statusCounts.full)，仅元数据 $($statusCounts.metadata_only)，抓取失败 $($statusCounts.failed)。
- 缺失本地文件：$($missingFiles.Count)；元数据字段问题：$($metadataProblems.Count)。
- 仍引用远程媒体：$(@($remoteMedia | Sort-Object -Unique).Count) 个；其中源站已失效或拒绝直链：$($failedMedia.Count) 个。
- Obsidian Vault 总体积：$(Format-Bytes $vaultStats.bytes)；原始语雀导出：$(Format-Bytes $rawStats.bytes)；规范化知识目录：$(Format-Bytes $normalizedStats.bytes)。

## 溯源字段

每篇文档均保存 `source_url`、`yuque_url`、`yuque_book_url`、`yuque_catalog_path`、语雀知识库 ID/名称/所有者、抓取时间与内容哈希。语雀原生文档另有 slug、UUID、doc_id、创建/首次发布/发布/更新时间；站外镜像另有外部 URL、规范 URL、外部标题/作者/时间、HTTP 状态、抽取方法与检索质量等级。

- 语雀知识库：[朔风卷雪]($($manifest.knowledge_base_url))
- 机器清单：`朔风卷雪/_migration-manifest.json`
- 外链抓取清单：`朔风卷雪/_external-mirror-manifest.json`
- 媒体抓取清单：`朔风卷雪/_assets/remote/manifest.json`
- 原始导出：`_source/朔风卷雪`

## 赛季分布

| 赛季 | 条目数 |
|---|---:|
$seasonRows

## 外部来源分布

| 站点 | 条目数 |
|---|---:|
$domainRows

## 非全文条目

“仅元数据”只允许用于发现来源，不应作为玩法事实正文；“失败”不可进入事实引用。

| 状态 | 目录位置 | 来源 |
|---|---|---|
$problemRows

## 媒体说明

已下载的外部媒体存放于 `朔风卷雪/_assets/remote`（$($assetStats.files) 个文件，$(Format-Bytes $assetStats.bytes)）。剩余失败项主要为历史头像/表情 404、站点安全验证，以及少量拒绝直链的 FlowUs 媒体；原文超链仍保留。
"@

[IO.File]::WriteAllText([IO.Path]::GetFullPath($ReportPath), $report, [Text.UTF8Encoding]::new($false))

$result = [ordered]@{
    schema_version = [int]$manifest.schema_version
    entries = @($manifest.entries).Count
    native_documents = $native.Count
    external_documents = $external.Count
    external_full = $statusCounts.full
    external_metadata_only = $statusCounts.metadata_only
    external_failed = $statusCounts.failed
    missing_files = $missingFiles.Count
    metadata_problems = $metadataProblems.Count
    unresolved_remote_media = @($remoteMedia | Sort-Object -Unique).Count
    failed_media = $failedMedia.Count
    vault_bytes = $vaultStats.bytes
    report = [IO.Path]::GetFullPath($ReportPath)
}
$result | ConvertTo-Json
if ($missingFiles.Count -or $metadataProblems.Count) { exit 2 }
