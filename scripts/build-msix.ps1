# Builds an unsigned MSIX of Bolo for Microsoft Store upload (the Store signs it).
# Identity values come from Partner Center > Product identity.
param(
  [string]$IdentityName = "PLACEHOLDER.Bolo",
  [string]$Publisher = "CN=00000000-0000-0000-0000-000000000000",
  [string]$PublisherDisplayName = "PLACEHOLDER",
  [string]$Version = "",
  [switch]$Build
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
if (-not $Version) { $v = (Get-Content "$root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version; $Version = "$v.0" }
if ($Build) { Push-Location $root; npx tauri build --no-bundle; Pop-Location }
$exe = "$root\src-tauri\target\release\cadence.exe"
if (-not (Test-Path $exe)) { throw "Missing $exe - run with -Build" }

$tools = "$env:LOCALAPPDATA\bolo-sdk-tools"
$makeappx = Get-ChildItem "$tools\bt" -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue | Where-Object FullName -like "*\x64\*" | Select-Object -First 1
if (-not $makeappx) {
  New-Item -ItemType Directory -Force $tools | Out-Null
  Invoke-WebRequest https://www.nuget.org/api/v2/package/Microsoft.Windows.SDK.BuildTools -OutFile "$tools\bt.zip"
  Expand-Archive "$tools\bt.zip" "$tools\bt" -Force
  $makeappx = Get-ChildItem "$tools\bt" -Recurse -Filter makeappx.exe | Where-Object FullName -like "*\x64\*" | Select-Object -First 1
}

$stage = "$root\target-msix\stage"
Remove-Item "$root\target-msix" -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
Copy-Item $exe "$stage\Bolo.exe"
$icons = "$root\src-tauri\icons"
foreach ($f in "StoreLogo.png","Square150x150Logo.png","Square44x44Logo.png","Square310x310Logo.png","Square71x71Logo.png") { Copy-Item "$icons\$f" "$stage\Assets\$f" }

@"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:desktop="http://schemas.microsoft.com/appx/manifest/desktop/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap desktop rescap">
  <Identity Name="$IdentityName" Publisher="$Publisher" Version="$Version" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>Bolo.com</DisplayName>
    <PublisherDisplayName>$PublisherDisplayName</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Resources><Resource Language="en-us" /></Resources>
  <Applications>
    <Application Id="Bolo" Executable="Bolo.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Bolo.com" Description="Push-to-talk voice dictation"
        BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile Wide310x150Logo="Assets\Wide310x150Logo.png" Square310x310Logo="Assets\Square310x310Logo.png" Square71x71Logo="Assets\Square71x71Logo.png" />
      </uap:VisualElements>
      <Extensions>
        <desktop:Extension Category="windows.startupTask" Executable="Bolo.exe" EntryPoint="Windows.FullTrustApplication">
          <desktop:StartupTask TaskId="BoloStartup" Enabled="true" DisplayName="Bolo" />
        </desktop:Extension>
      </Extensions>
    </Application>
  </Applications>
  <Capabilities>
    <Capability Name="internetClient" />
    <rescap:Capability Name="runFullTrust" />
    <DeviceCapability Name="microphone" />
  </Capabilities>
</Package>
"@ | Set-Content "$stage\AppxManifest.xml" -Encoding UTF8

Add-Type -AssemblyName System.Drawing
$src=[System.Drawing.Image]::FromFile("$icons\Square150x150Logo.png"); $bmp=New-Object System.Drawing.Bitmap 310,150; $g=[System.Drawing.Graphics]::FromImage($bmp); $g.InterpolationMode="HighQualityBicubic"; $g.DrawImage($src,80,0,150,150); $bmp.Save("$stage\Assets\Wide310x150Logo.png",[System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose(); $src.Dispose()
$out = "$root\target-msix\Bolo_${Version}_x64.msix"
& $makeappx.FullName pack /d $stage /p $out /o
if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }
Write-Host "Built $out (unsigned; upload to Partner Center, Store signs it)"


