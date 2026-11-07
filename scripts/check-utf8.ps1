param([string] $Root)
$ErrorActionPreference = 'Stop'

$projectRoot = if ([string]::IsNullOrWhiteSpace($Root)) { Split-Path -Parent $PSScriptRoot } else { [System.IO.Path]::GetFullPath($Root) }
$excludedDirectories = @(
    (Join-Path $projectRoot 'src/stamina'),
    (Join-Path $projectRoot 'src/stamina_setup'),
    (Join-Path $projectRoot 'target'),
    (Join-Path $projectRoot '.git')
)
$textNames = @('Cargo.lock', '.gitignore', '.gitattributes', '.editorconfig')
$textExtensions = @(
    '.bat', '.c', '.cfg', '.cpp', '.css', '.csv', '.h', '.html', '.ini', '.js',
    '.json', '.md', '.ps1', '.rc', '.rs', '.sh', '.toml', '.txt', '.xml', '.yaml', '.yml'
)
$utf8 = [System.Text.UTF8Encoding]::new($false, $true)
$invalidFiles = [System.Collections.Generic.List[string]]::new()

Get-ChildItem -LiteralPath $projectRoot -File -Recurse -Force | ForEach-Object {
    $file = $_
    if ($textExtensions -notcontains $file.Extension.ToLowerInvariant() -and $textNames -notcontains $file.Name) { return }
    foreach ($directory in $excludedDirectories) {
        if ($file.FullName.StartsWith($directory + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
            return
        }
    }

    try {
        $bytes = [System.IO.File]::ReadAllBytes($file.FullName)
        if ([Array]::IndexOf($bytes, [byte]0) -ge 0) {
            $invalidFiles.Add([System.IO.Path]::GetRelativePath($projectRoot, $file.FullName))
            return
        }
        $offset = if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { 3 } else { 0 }
        $null = $utf8.GetString($bytes, $offset, $bytes.Length - $offset)
    } catch [System.Text.DecoderFallbackException] {
        $invalidFiles.Add([System.IO.Path]::GetRelativePath($projectRoot, $file.FullName))
    }
}

if ($invalidFiles.Count -gt 0) {
    Write-Error "Invalid UTF-8 in text files:`n$($invalidFiles -join "`n")" -ErrorAction Continue
    exit 1
}

Write-Host 'UTF-8 check passed.'
