param(
    [Parameter(Mandatory = $true)]
    [string]$DocumentRoot,

    [Parameter(Mandatory = $true)]
    [string]$AssetRoot
)

$ErrorActionPreference = "Stop"

function Get-RelativeUri {
    param([string]$FromDirectory, [string]$TargetPath)
    $from = [IO.Path]::GetFullPath($FromDirectory).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    return ([Uri]$from).MakeRelativeUri([Uri][IO.Path]::GetFullPath($TargetPath)).ToString()
}

$documents = @(Get-ChildItem -LiteralPath $DocumentRoot -Recurse -Filter '*.md' -File)
if ($documents.Count -eq 0) { throw "No Markdown documents found under $DocumentRoot" }
if (-not (Test-Path -LiteralPath $AssetRoot)) { New-Item -ItemType Directory -Path $AssetRoot -Force | Out-Null }
$assetDirectory = (Resolve-Path -LiteralPath $AssetRoot).Path

$mediaPattern = 'https?://[^\s"''<>\)]+?\.(?:png|jpe?g|gif|webp|svg|bmp|mp4|webm|mp3|wav|m4a|ogg)(?:\?[^\s"''<>\)]*)?'
$urlDocuments = @{}
foreach ($document in $documents) {
    $content = Get-Content -LiteralPath $document.FullName -Raw
    foreach ($match in [regex]::Matches($content, $mediaPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        $url = $match.Value
        if (-not $urlDocuments.ContainsKey($url)) { $urlDocuments[$url] = New-Object System.Collections.Generic.List[string] }
        $urlDocuments[$url].Add($document.FullName)
    }
}

$handler = [Net.Http.HttpClientHandler]::new()
$client = [Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromSeconds(45)
$client.DefaultRequestHeaders.UserAgent.ParseAdd('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131 Safari/537.36')

$results = New-Object System.Collections.Generic.List[object]
$completed = 0
foreach ($url in @($urlDocuments.Keys | Sort-Object)) {
    $completed++
    $requestUrl = $url.Replace('&amp;', '&')
    $urlHashBytes = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($url))
    $urlHash = [Convert]::ToHexString($urlHashBytes).ToLowerInvariant()
    try {
        $uri = [Uri]$requestUrl
        $extension = [IO.Path]::GetExtension($uri.AbsolutePath).ToLowerInvariant()
        if ($extension -notmatch '^\.(png|jpe?g|gif|webp|svg|bmp|mp4|webm|mp3|wav|m4a|ogg)$') { $extension = '.bin' }
        $target = Join-Path $assetDirectory ($urlHash.Substring(0, 24) + $extension)
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
            $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::Get, $requestUrl)
            $referer = if ($uri.DnsSafeHost -eq 'cdn2.flowus.cn') {
                'https://flowus.cn/'
            } elseif ($uri.DnsSafeHost -like '*.docs.qq.com') {
                'https://docs.qq.com/'
            } elseif ($uri.DnsSafeHost -like '*.jx3box.com') {
                'https://www.jx3box.com/'
            } else {
                'https://www.yuque.com/'
            }
            $request.Headers.Referrer = [Uri]$referer
            $response = $client.SendAsync($request).GetAwaiter().GetResult()
            [void]$response.EnsureSuccessStatusCode()
            $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
            $response.Dispose()
            $request.Dispose()
            if ($bytes.Length -eq 0) { throw 'Downloaded media is empty.' }
            [IO.File]::WriteAllBytes($target, $bytes)
        }
        $fileBytes = [IO.File]::ReadAllBytes($target)
        $contentHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($fileBytes)).ToLowerInvariant()
        foreach ($documentPath in $urlDocuments[$url]) {
            $content = Get-Content -LiteralPath $documentPath -Raw
            $relative = Get-RelativeUri -FromDirectory (Split-Path -Parent $documentPath) -TargetPath $target
            $updated = $content.Replace($url, $relative)
            if ($updated -ne $content) { [IO.File]::WriteAllText($documentPath, $updated, [Text.UTF8Encoding]::new($false)) }
        }
        $results.Add([pscustomobject]@{
            source_url = $url
            local_file = $target.Substring($assetDirectory.Length + 1)
            bytes = $fileBytes.Length
            sha256 = $contentHash
            status = 'downloaded'
            referenced_by = @($urlDocuments[$url] | ForEach-Object { [IO.Path]::GetRelativePath($DocumentRoot, $_) })
        })
    } catch {
        $results.Add([pscustomobject]@{
            source_url = $url
            local_file = $null
            bytes = 0
            sha256 = $null
            status = 'failed'
            error = $_.Exception.Message
            referenced_by = @($urlDocuments[$url] | ForEach-Object { [IO.Path]::GetRelativePath($DocumentRoot, $_) })
        })
    }
    if (($completed % 10) -eq 0 -or $completed -eq $urlDocuments.Count) {
        Write-Host "Localized $completed / $($urlDocuments.Count) remote media URLs"
    }
}

$client.Dispose()
$handler.Dispose()
$manifest = [ordered]@{
    schema_version = 1
    generated_at = [DateTimeOffset]::Now.ToString('o')
    source_count = $urlDocuments.Count
    downloaded_count = @($results | Where-Object status -eq 'downloaded').Count
    failed_count = @($results | Where-Object status -eq 'failed').Count
    items = $results
}
$manifestPath = Join-Path $assetDirectory 'manifest.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))

[pscustomobject]@{
    schema_version = $manifest.schema_version
    generated_at = $manifest.generated_at
    source_count = $manifest.source_count
    downloaded_count = $manifest.downloaded_count
    failed_count = $manifest.failed_count
} | ConvertTo-Json
