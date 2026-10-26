# Check that every relative Markdown link in the repository resolves.
#
# Run from CI and locally; links are resolved against the file that contains them, so a link
# written for the repository root fails when the file lives in docs/ (that happened once).
# External http(s) links are not fetched - the network is not the thing being checked.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$files = Get-ChildItem -Path $root -Recurse -Filter *.md |
    Where-Object { $_.FullName -notmatch '\\(target|dist|scrap|decomp)\\' }
$missing = New-Object System.Collections.Generic.List[string]
$checked = 0

foreach ($file in $files) {
    $text = Get-Content -LiteralPath $file.FullName -Raw
    foreach ($match in [regex]::Matches($text, '\]\(([^)#]+?)\)')) {
        $link = $match.Groups[1].Value
        if ($link -match '^(?:https?:|mailto:)') { continue }
        if ($link -match '^#') { continue }
        $checked++
        $target = Join-Path $file.DirectoryName $link
        if (-not (Test-Path -LiteralPath $target)) {
            $missing.Add("$($file.FullName.Substring($root.Length + 1)): $link")
        }
    }
}

if ($missing.Count -gt 0) {
    foreach ($item in $missing) { Write-Host "::error::missing relative link $item" }
    Write-Host "$($missing.Count) of $checked link(s) do not resolve."
    exit 1
}
Write-Host "All $checked relative link(s) across $($files.Count) Markdown file(s) resolve."
