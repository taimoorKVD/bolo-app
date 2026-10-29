# Build a signed Bolo release for in-app updates (direct-download / NSIS build).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release.ps1 -Version 0.1.2 -Notes "Fixed X`nAdded Y"
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release.ps1 -Bump patch -NotesFile notes.txt
#
# 1. Sets the version in tauri.conf.json, Cargo.toml and package.json.
# 2. Builds the NSIS installer with createUpdaterArtifacts, signed with the
#    updater key (never stored in this repo).
# 3. Writes latest.json and copies the installer + latest.json into the website
#    repo (downloads/ and updates/). It does NOT commit, push, or deploy.
param(
  [string]$Version = "",
  [ValidateSet("", "patch", "minor", "major")][string]$Bump = "",
  [string]$Notes = "",
  [string]$NotesFile = "",
  [string]$KeyPath = "D:\taimoorKVD\my-experiments\_secrets\bolo-updater.key",
  [string]$Website = "",
  [string]$BaseUrl = "https://bolo.thebetawebsite.com",
  [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
if (-not $Website) { $Website = Join-Path (Split-Path $root -Parent) "bolo.com-website" }
$conf = Join-Path $root "src-tauri\tauri.conf.json"
$cargo = Join-Path $root "src-tauri\Cargo.toml"
$pkg = Join-Path $root "package.json"
$utf8 = New-Object System.Text.UTF8Encoding $false

$current = (Get-Content $conf -Raw -Encoding UTF8 | ConvertFrom-Json).version
if (-not $Version) {
  if (-not $Bump) { $Version = $current } else {
    $p = $current.Split(".") | ForEach-Object { [int]$_ }
    switch ($Bump) { "major" { $p = @($p[0] + 1, 0, 0) } "minor" { $p = @($p[0], $p[1] + 1, 0) } "patch" { $p = @($p[0], $p[1], $p[2] + 1) } }
    $Version = ($p -join ".")
  }
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3 (got '$Version')" }
Write-Host "Releasing Bolo $Version (was $current)"

function Set-Version($path, $pattern, $replacement) {
  $text = [IO.File]::ReadAllText($path)
  $new = [regex]::Replace($text, $pattern, $replacement, 1)
  [IO.File]::WriteAllText($path, $new, $utf8)
}
$rx = New-Object regex '"version":\s*"[^"]+"'
foreach ($f in $conf, $pkg) { $t = [IO.File]::ReadAllText($f); [IO.File]::WriteAllText($f, $rx.Replace($t, "`"version`": `"$Version`"", 1), $utf8) }
$rc = New-Object regex '(?m)^version = "[^"]+"'
$t = [IO.File]::ReadAllText($cargo); [IO.File]::WriteAllText($cargo, $rc.Replace($t, "version = `"$Version`"", 1), $utf8)

if ($NotesFile) { $Notes = Get-Content $NotesFile -Raw -Encoding UTF8 }
if (-not $Notes) { $Notes = "Bug fixes and improvements." }

$nsisDir = Join-Path $root "src-tauri\target\release\bundle\nsis"
if (-not $SkipBuild) {
  if (-not (Test-Path $KeyPath)) { throw "Updater key not found at $KeyPath" }
  # The key is passed through the environment only; never echo it.
  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $KeyPath -Raw
  if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "" }
  Push-Location $root
  try {
    npx tauri build --bundles nsis --config '{\"bundle\":{\"createUpdaterArtifacts\":true}}'
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
  } finally {
    Pop-Location
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  }
}

$setup = Get-ChildItem $nsisDir -Filter "*_${Version}_x64-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw "No NSIS installer for $Version in $nsisDir" }
$sig = "$($setup.FullName).sig"
if (-not (Test-Path $sig)) { throw "Missing signature $sig (was the build signed?)" }

$name = "Bolo_${Version}_x64-setup.exe"
$downloads = Join-Path $Website "downloads"
$updates = Join-Path $Website "updates"
New-Item -ItemType Directory -Force $downloads, $updates | Out-Null
Copy-Item $setup.FullName (Join-Path $downloads $name) -Force

$latest = [ordered]@{
  version   = $Version
  notes     = $Notes.Trim()
  pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = [ordered]@{
    "windows-x86_64" = [ordered]@{
      signature = (Get-Content $sig -Raw).Trim()
      url       = "$BaseUrl/downloads/$name"
    }
  }
}
[IO.File]::WriteAllText((Join-Path $updates "latest.json"), ($latest | ConvertTo-Json -Depth 5), $utf8)

$versionJson = Join-Path $Website "version.json"
if (Test-Path $versionJson) {
  $v = Get-Content $versionJson -Raw | ConvertFrom-Json
  $v.version = $Version
  $v.downloads.nsis.href = "downloads/$name"
  $v.downloads.nsis.bytes = (Get-Item (Join-Path $downloads $name)).Length
  [IO.File]::WriteAllText($versionJson, ($v | ConvertTo-Json -Depth 5), $utf8)
}

Write-Host ""
Write-Host "Done. Staged in $Website (not committed):"
Write-Host "  downloads\$name"
Write-Host "  updates\latest.json"
Write-Host "Next: review, commit, and push the website repo to publish. Installed apps pick it up within 4 hours (or on next launch)."
