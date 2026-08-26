param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRoot,

    [Parameter(Mandatory = $true)]
    [string]$OutputRoot,

    [string]$KnowledgeBaseUrl = "https://www.yuque.com/sgyxy/cangyun"
)

$ErrorActionPreference = "Stop"

function ConvertTo-SafeName {
    param([string]$Name)

    $safe = $Name.Trim()
    $safe = $safe -replace '^🔗\s*', ''
    $safe = $safe.Replace('/', '／').Replace('\', '＼').Replace('|', '｜')
    $safe = $safe.Replace(':', '：').Replace('*', '＊').Replace('?', '？')
    $safe = $safe.Replace('"', '＂').Replace('<', '＜').Replace('>', '＞')
    return $safe.TrimEnd('.', ' ')
}

function ConvertTo-YamlString {
    param([AllowEmptyString()][string]$Value)
    return ($Value | ConvertTo-Json -Compress)
}

function Get-SourceSite {
    param([string]$Url)
    try { return ([Uri]$Url).DnsSafeHost.ToLowerInvariant() } catch { return '' }
}

function Get-RelativeUri {
    param(
        [string]$FromDirectory,
        [string]$TargetPath
    )

    $from = [IO.Path]::GetFullPath($FromDirectory).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    $target = [IO.Path]::GetFullPath($TargetPath)
    return ([Uri]$from).MakeRelativeUri([Uri]$target).ToString()
}

function Rewrite-AssetLinks {
    param(
        [string]$Content,
        [string]$SourceDocument,
        [string]$OutputDocument
    )

    $sourceDirectory = Split-Path -Parent $SourceDocument
    $outputDirectory = Split-Path -Parent $OutputDocument

    $markdownPattern = '(?<prefix>!?\[[^\]]*\]\()(?<url>(?:\./)?(?:img|attachments)/[^)]+)(?<suffix>\))'
    $Content = [regex]::Replace($Content, $markdownPattern, {
        param($match)
        $rawUrl = [Uri]::UnescapeDataString($match.Groups['url'].Value)
        $target = Join-Path $sourceDirectory ($rawUrl -replace '/', [IO.Path]::DirectorySeparatorChar)
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { return $match.Value }
        $relative = Get-RelativeUri -FromDirectory $outputDirectory -TargetPath $target
        return $match.Groups['prefix'].Value + $relative + $match.Groups['suffix'].Value
    }, [Text.RegularExpressions.RegexOptions]::IgnoreCase)

    $htmlPattern = '(?<prefix>(?:src|href)=["''])(?<url>(?:\./)?(?:img|attachments)/[^"'']+)(?<suffix>["''])'
    return [regex]::Replace($Content, $htmlPattern, {
        param($match)
        $rawUrl = [Uri]::UnescapeDataString($match.Groups['url'].Value)
        $target = Join-Path $sourceDirectory ($rawUrl -replace '/', [IO.Path]::DirectorySeparatorChar)
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { return $match.Value }
        $relative = Get-RelativeUri -FromDirectory $outputDirectory -TargetPath $target
        return $match.Groups['prefix'].Value + $relative + $match.Groups['suffix'].Value
    }, [Text.RegularExpressions.RegexOptions]::IgnoreCase)
}

$source = (Resolve-Path -LiteralPath $SourceRoot).Path
$indexPath = Join-Path $source 'index.md'
$progressPath = Join-Path $source 'progress.json'
if (-not (Test-Path -LiteralPath $indexPath -PathType Leaf)) { throw "Missing source index: $indexPath" }
if (-not (Test-Path -LiteralPath $progressPath -PathType Leaf)) { throw "Missing source progress metadata: $progressPath" }

if (Test-Path -LiteralPath $OutputRoot) {
    $existing = @(Get-ChildItem -LiteralPath $OutputRoot -Force)
    if ($existing.Count -gt 0) { throw "Output directory is not empty: $OutputRoot" }
} else {
    New-Item -ItemType Directory -Path $OutputRoot | Out-Null
}
$output = (Resolve-Path -LiteralPath $OutputRoot).Path

$progress = @(Get-Content -LiteralPath $progressPath -Raw | ConvertFrom-Json)
$metadataByPath = @{}
foreach ($item in $progress) {
    if ($item.toc.type -eq 'DOC' -and $item.path) {
        $metadataByPath[($item.path -replace '\\', '/')] = $item
    }
}

$season = $null
$category = '未分类'
$entries = New-Object System.Collections.Generic.List[object]
$localCount = 0
$externalCount = 0
$lines = Get-Content -LiteralPath $indexPath

foreach ($line in $lines) {
    if ($line -match '^##\s+📂\s*(?<name>.+?)\s*$') {
        $season = ConvertTo-SafeName $Matches['name']
        $category = '未分类'
        continue
    }
    if ($line -match '^#{2,3}\s+⬇️\s*(?<name>.+?)\s*$') {
        $category = ConvertTo-SafeName $Matches['name']
        continue
    }
    if ($line -notmatch '^(?:#{2,3}\s+|-\s+)\[(?<title>[^\]]+)\]\((?<target>.+)\)\s*$') { continue }
    if ([string]::IsNullOrWhiteSpace($season)) { throw "Found entry before a season heading: $line" }

    $title = $Matches['title'].Trim()
    $target = $Matches['target'].Trim()
    $safeSeason = ConvertTo-SafeName $season
    $safeCategory = ConvertTo-SafeName $category
    $safeTitle = ConvertTo-SafeName $title
    $entryDirectory = Join-Path (Join-Path $output $safeSeason) $safeCategory
    New-Item -ItemType Directory -Path $entryDirectory -Force | Out-Null
    $outputDocument = Join-Path $entryDirectory ($safeTitle + '.md')

    if (Test-Path -LiteralPath $outputDocument) {
        $suffixBytes = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($target))
        $suffix = [Convert]::ToHexString($suffixBytes).Substring(0, 8).ToLowerInvariant()
        $outputDocument = Join-Path $entryDirectory ($safeTitle + '_' + $suffix + '.md')
    }

    if ($target -match '^https?://') {
        $catalogPath = "$season / $category / $title"
        $frontmatter = @(
            '---',
            ('title: ' + (ConvertTo-YamlString $title)),
            'kind: external_link',
            ('season: ' + (ConvertTo-YamlString $season)),
            ('category: ' + (ConvertTo-YamlString $category)),
            ('source_url: ' + (ConvertTo-YamlString $target)),
            ('source_site: ' + (ConvertTo-YamlString (Get-SourceSite $target))),
            ('yuque_book_url: ' + (ConvertTo-YamlString $KnowledgeBaseUrl)),
            ('yuque_catalog_path: ' + (ConvertTo-YamlString $catalogPath)),
            'yuque_entry_type: external_link',
            'mirror_status: pending',
            ('captured_at: ' + (ConvertTo-YamlString ([DateTimeOffset]::Now.ToString('o')))),
            '---',
            '',
            '# ' + $title,
            '',
            '> 此条目在原语雀知识库中是站外链接，尚未镜像正文。',
            '',
            '[打开原始内容](' + $target + ')',
            ''
        ) -join "`n"
        [IO.File]::WriteAllText($outputDocument, $frontmatter, [Text.UTF8Encoding]::new($false))
        $externalCount++
        $entries.Add([pscustomobject]@{ title=$title; season=$season; category=$category; kind='external_link'; source=$target; source_site=(Get-SourceSite $target); yuque_book_url=$KnowledgeBaseUrl; yuque_catalog_path=$catalogPath; output=$outputDocument.Substring($output.Length + 1) })
        continue
    }

    $decodedTarget = [Uri]::UnescapeDataString(($target -split '#', 2)[0]) -replace '\\', '/'
    $sourceDocument = Join-Path $source ($decodedTarget -replace '/', [IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $sourceDocument -PathType Leaf)) { throw "Missing downloaded document: $decodedTarget" }
    $meta = $metadataByPath[$decodedTarget]
    $slug = if ($meta) { [string]$meta.toc.url } else { '' }
    $uuid = if ($meta) { [string]$meta.toc.uuid } else { '' }
    $updatedAt = if ($meta) { [string]$meta.contentUpdatedAt } else { '' }
    $createdAt = if ($meta) { [string]$meta.createAt } else { '' }
    $publishedAt = if ($meta) { [string]$meta.publishedAt } else { '' }
    $firstPublishedAt = if ($meta) { [string]$meta.firstPublishedAt } else { '' }
    $docId = if ($meta) { [string]$meta.toc.doc_id } else { '' }
    $sourceUrl = if ($slug) { $KnowledgeBaseUrl.TrimEnd('/') + '/' + $slug } else { $KnowledgeBaseUrl }
    $content = Get-Content -LiteralPath $sourceDocument -Raw
    $content = Rewrite-AssetLinks -Content $content -SourceDocument $sourceDocument -OutputDocument $outputDocument
    $frontmatter = @(
        '---',
        ('title: ' + (ConvertTo-YamlString $title)),
        'kind: yuque_document',
        ('season: ' + (ConvertTo-YamlString $season)),
        ('category: ' + (ConvertTo-YamlString $category)),
        ('source_url: ' + (ConvertTo-YamlString $sourceUrl)),
        'source_site: yuque.com',
        ('yuque_book_url: ' + (ConvertTo-YamlString $KnowledgeBaseUrl)),
        ('yuque_catalog_path: ' + (ConvertTo-YamlString "$season / $category / $title")),
        ('yuque_slug: ' + (ConvertTo-YamlString $slug)),
        ('yuque_uuid: ' + (ConvertTo-YamlString $uuid)),
        ('yuque_doc_id: ' + (ConvertTo-YamlString $docId)),
        ('source_created_at: ' + (ConvertTo-YamlString $createdAt)),
        ('source_first_published_at: ' + (ConvertTo-YamlString $firstPublishedAt)),
        ('source_published_at: ' + (ConvertTo-YamlString $publishedAt)),
        ('source_updated_at: ' + (ConvertTo-YamlString $updatedAt)),
        ('captured_at: ' + (ConvertTo-YamlString ([DateTimeOffset]::Now.ToString('o')))),
        '---',
        ''
    ) -join "`n"
    [IO.File]::WriteAllText($outputDocument, $frontmatter + $content, [Text.UTF8Encoding]::new($false))
    $localCount++
    $entries.Add([pscustomobject]@{ title=$title; season=$season; category=$category; kind='yuque_document'; source=$sourceUrl; source_site='yuque.com'; yuque_book_url=$KnowledgeBaseUrl; yuque_catalog_path="$season / $category / $title"; output=$outputDocument.Substring($output.Length + 1); source_file=$decodedTarget; yuque_slug=$slug; yuque_uuid=$uuid; yuque_doc_id=$docId; created_at=$createdAt; first_published_at=$firstPublishedAt; published_at=$publishedAt; updated_at=$updatedAt })
}

$manifest = [ordered]@{
    schema_version = 2
    generated_at = [DateTimeOffset]::Now.ToString('o')
    knowledge_base_url = $KnowledgeBaseUrl
    source_root = $source
    output_root = $output
    document_count = $localCount
    external_link_count = $externalCount
    entry_count = $entries.Count
    entries = $entries
}
$manifestPath = Join-Path $output '_migration-manifest.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))

$readme = @"
# 朔风卷雪

此目录按语雀站点的语义结构整理为“赛季 → 分类 → 条目”。

- 语雀原生文档：$localCount 篇，正文已保存为本地 Markdown。
- 站外链接条目：$externalCount 个，当前保存为带来源地址的 Markdown 占位页。
- 原始导出、图片和附件保存在 Vault 根目录的 `_source/朔风卷雪`，用于追溯与增量同步。
- 机器可读迁移清单：`_migration-manifest.json`。

当前赛季“暗影千机（2026）”在语雀目录中采用展开式布局；本地库已按实际含义将它的通用、白皮书、计算器、实战、基础与宏归入该赛季目录。
"@
[IO.File]::WriteAllText((Join-Path $output 'README.md'), $readme, [Text.UTF8Encoding]::new($false))

[pscustomobject]@{
    output_root = $output
    document_count = $localCount
    external_link_count = $externalCount
    entry_count = $entries.Count
    manifest = $manifestPath
} | ConvertTo-Json
