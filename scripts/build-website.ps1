# Copy the release NSIS and MSI packages into website/downloads and
# refresh website/version.json (and the matching copy embedded in index.html).
# Requires a completed `npm run tauri build` so the bundle folder exists.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-website.ps1

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$confPath = Join-Path $root "src-tauri\tauri.conf.json"
$website = Join-Path $root "website"
$downloads = Join-Path $website "downloads"
$versionPath = Join-Path $website "version.json"
$indexPath = Join-Path $website "index.html"

if (-not (Test-Path $confPath)) {
    throw "Could not find $confPath"
}

$conf = Get-Content -Raw -Encoding UTF8 $confPath | ConvertFrom-Json
$version = [string]$conf.version
if ([string]::IsNullOrWhiteSpace($version)) {
    throw "tauri.conf.json has no version."
}

$bundle = Join-Path $root "src-tauri\target\release\bundle"
$nsisDir = Join-Path $bundle "nsis"
$msiDir = Join-Path $bundle "msi"

$nsisSource = Get-ChildItem -Path $nsisDir -Filter "*setup.exe" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
$msiSource = Get-ChildItem -Path $msiDir -Filter "*.msi" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if (-not $nsisSource) {
    throw "No NSIS setup.exe under $nsisDir. Build the installer first: npm run tauri build"
}
if (-not $msiSource) {
    throw "No MSI under $msiDir. Build the installer first: npm run tauri build"
}

New-Item -ItemType Directory -Force -Path $downloads | Out-Null

$nsisName = "Bolo.com_${version}_x64-setup.exe"
$msiName = "Bolo.com_${version}_x64_en-US.msi"
$nsisDest = Join-Path $downloads $nsisName
$msiDest = Join-Path $downloads $msiName

Copy-Item -LiteralPath $nsisSource.FullName -Destination $nsisDest -Force
Copy-Item -LiteralPath $msiSource.FullName -Destination $msiDest -Force

$nsisBytes = (Get-Item -LiteralPath $nsisDest).Length
$msiBytes = (Get-Item -LiteralPath $msiDest).Length

$json = @"
{
  "name": "Bolo.com",
  "version": "$version",
  "requirements": "Windows 10 or 11, 64-bit",
  "downloads": {
    "nsis": {
      "href": "downloads/$nsisName",
      "bytes": $nsisBytes
    },
    "msi": {
      "href": "downloads/$msiName",
      "bytes": $msiBytes
    }
  }
}
"@
$json = $json.Trim() + "`n"

$utf8 = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($versionPath, $json, $utf8)

$html = [System.IO.File]::ReadAllText($indexPath)
$pattern = '(?s)(<script type="application/json" id="bolo-release">).*?(</script>)'
$replacement = '${1}' + "`n" + $json.TrimEnd() + "`n" + '${2}'
$updated = [regex]::Replace($html, $pattern, $replacement, 1)
if ($updated -eq $html) {
    throw "index.html is missing the bolo-release JSON block."
}
[System.IO.File]::WriteAllText($indexPath, $updated, $utf8)

Write-Host "website/downloads/$nsisName ($nsisBytes bytes)"
Write-Host "website/downloads/$msiName ($msiBytes bytes)"
Write-Host "Updated website/version.json and the embedded copy in index.html"
