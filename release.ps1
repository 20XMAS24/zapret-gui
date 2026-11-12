<#
  Builds ZapretGUI, prepares a clean staging folder (official zapret release, TgWsProxy, sing-box),
  compiles the Inno Setup installer, writes its SHA-256 and optionally publishes a GitHub release.

    .\release.ps1                     build dist\ZapretGUI-Setup-<ver>.exe (+ .sha256)
    .\release.ps1 -Version 2.0.1      bump Cargo.toml version first
    .\release.ps1 -Publish            also create GitHub release v<ver> with both files
    .\release.ps1 -Publish -Notes "What changed"
    .\release.ps1 -Publish -NotesFile notes.md

  Release notes are what users see in the "Что нового" window after the update (Markdown:
  "## heading", "- item"). Without -Notes / -NotesFile they are built from commit subjects
  since the previous tag.

  The repository comes from `git remote get-url origin` (or -Repo owner/name).
  Nothing from your local zapret folder (lists, ini, proxies) gets into the installer.
#>
param(
    [string]$Version = "",
    [switch]$Publish,
    [string]$Notes = "",
    [string]$NotesFile = "",
    [string]$Repo = "",
    [string]$SingBoxVersion = "1.14.4"
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Set-Location $PSScriptRoot

function Step($t) { Write-Host "`n==> $t" -ForegroundColor Cyan }
# Throws unless $path is a non-empty path inside $parent (guards every recursive copy).
function Assert-Inside($path, $parent) {
    if ([string]::IsNullOrWhiteSpace($path)) { throw "source not found inside $parent" }
    $full = [IO.Path]::GetFullPath($path).TrimEnd('\') + '\'
    $root = [IO.Path]::GetFullPath($parent).TrimEnd('\') + '\'
    if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw "refusing to use $path (outside $parent)" }
}
# Always extracts into a fresh folder under dist\cache, never reuses a possibly broken one.
function Expand-Fresh($zip, $name) {
    $dir = Join-Path $cache $name
    Assert-Inside $dir $cache
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    New-Item -ItemType Directory $dir | Out-Null
    & $tarExe -xf $zip -C $dir
    if ($LASTEXITCODE) { throw "cannot extract $zip" }
    if (-not (Get-ChildItem -LiteralPath $dir -Recurse -File | Select-Object -First 1)) { throw "$zip extracted to nothing" }
    return $dir
}
function Fetch($url, $dest) {
    if (-not (Test-Path $dest)) {
        Write-Host "    download $url"
        Invoke-WebRequest -Uri $url -OutFile "$dest.part" -UseBasicParsing -TimeoutSec 300 -Headers @{ "User-Agent" = "ZapretGUI-release" }
        Move-Item "$dest.part" $dest -Force
    }
}

# ---- tools
$tarExe = Join-Path $env:SystemRoot "System32\tar.exe"   # Windows bsdtar, not Git's tar
$sc =Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$sc;$env:ProgramFiles\GitHub CLI;$env:PATH"
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found (winget install JRSoftware.InnoSetup)" }

# ---- version
$toml = Get-Content Cargo.toml -Raw
if ($Version) {
    # Only the first `version = ` line (the [package] one); a Regex instance is needed for a count.
    $toml = ([regex]'(?m)^version = "[^"]+"').Replace($toml, "version = `"$Version`"", 1)
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot "Cargo.toml"), $toml)
}
$ver = [regex]::Match($toml, '(?m)^version = "([^"]+)"').Groups[1].Value
if (-not $Repo) {
    $url = (git remote get-url origin 2>$null)
    if ($url -match 'github\.com[:/]([^/]+/[^/.]+)') { $Repo = $Matches[1] }
}
if (-not $Repo) { throw "GitHub repository unknown: add git remote 'origin' or pass -Repo owner/name" }
Write-Host "ZapretGUI $ver, repository $Repo"

# ---- build
Step "cargo build"
$env:ZAPRET_GUI_REPO = $Repo
Remove-Item Env:ZAPRET_NO_MANIFEST -ErrorAction SilentlyContinue
cargo build --release
if ($LASTEXITCODE) { throw "cargo build failed" }

# ---- staging
$dist = Join-Path $PSScriptRoot "dist"; $cache = Join-Path $dist "cache"; $stage = Join-Path $dist "stage"
New-Item -ItemType Directory -Force $cache | Out-Null
Assert-Inside $stage $dist
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\bin", "$stage\zapret" | Out-Null
Copy-Item "target\release\ZapretGUI.exe" $stage

Step "zapret (official release)"
$zv = (Invoke-WebRequest "https://raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/.service/version.txt" -UseBasicParsing -TimeoutSec 60).Content.Trim()
$zzip = Join-Path $cache "zapret-$zv.zip"
Fetch "https://github.com/Flowseal/zapret-discord-youtube/releases/download/$zv/zapret-discord-youtube-$zv.zip" $zzip
$zx = Expand-Fresh $zzip "zapret-$zv"
# The release zip wraps everything in one folder: find the folder that actually holds the .bat files.
$zsrc = @($zx) + @(Get-ChildItem -LiteralPath $zx -Directory | ForEach-Object { $_.FullName }) |
    Where-Object { Get-ChildItem -LiteralPath $_ -Filter *.bat -File } | Select-Object -First 1
Assert-Inside $zsrc $zx
if (-not (Test-Path -LiteralPath (Join-Path $zsrc "bin\winws.exe"))) { throw "zapret archive has no bin\winws.exe" }
Copy-Item -Path (Join-Path $zsrc "*") -Destination "$stage\zapret" -Recurse -Force
New-Item -ItemType Directory -Force "$stage\zapret\.service" | Out-Null
Set-Content "$stage\zapret\.service\version.txt" $zv -NoNewline -Encoding ASCII
Write-Host "    zapret $zv"

Step "TgWsProxy (latest release)"
$tg = Invoke-RestMethod "https://api.github.com/repos/Flowseal/tg-ws-proxy/releases/latest" -TimeoutSec 60 -Headers @{ "User-Agent" = "ZapretGUI-release" }
$tgUrl = ($tg.assets | Where-Object { $_.name -eq "TgWsProxy_windows.exe" } | Select-Object -First 1).browser_download_url
if (-not $tgUrl) { throw "TgWsProxy release has no TgWsProxy_windows.exe" }
$tgFile = Join-Path $cache "TgWsProxy_windows-$($tg.tag_name).exe"
Fetch $tgUrl $tgFile
Copy-Item $tgFile "$stage\TgWsProxy_windows.exe"
Write-Host "    TgWsProxy $($tg.tag_name)"

Step "sing-box $SingBoxVersion"
$sbzip = Join-Path $cache "sing-box-$SingBoxVersion.zip"
Fetch "https://github.com/SagerNet/sing-box/releases/download/v$SingBoxVersion/sing-box-$SingBoxVersion-windows-amd64.zip" $sbzip
$sbx = Expand-Fresh $sbzip "sing-box-$SingBoxVersion"
$sbExe = (Get-ChildItem -LiteralPath $sbx -Recurse -Filter sing-box.exe -File | Select-Object -First 1).FullName
Assert-Inside $sbExe $sbx
Copy-Item -LiteralPath $sbExe -Destination "$stage\bin\sing-box.exe"

$mb = (Get-ChildItem -LiteralPath $stage -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
if ($mb -gt 300) { throw ("staging is {0:N0} MB - something went wrong, aborting" -f $mb) }
Write-Host ("    staging: {0:N1} MB" -f $mb)

# ---- installer
Step "Inno Setup"
& $iscc /Q "/DAppVersion=$ver" "/DStage=$stage" "/DOutDir=$dist" "/DRepo=$Repo" "installer\setup.iss"
if ($LASTEXITCODE) { throw "ISCC failed" }
$setupName = "ZapretGUI-Setup-$ver.exe"
$setup = Join-Path $dist $setupName
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText("$setup.sha256", "$hash  $setupName`n")
Write-Host "    $setup"
Write-Host "    sha256 $hash"

# ---- publish
if ($Publish) {
    Step "GitHub release v$ver"
    # PS 5.1 turns redirected native stderr into an error under ErrorActionPreference=Stop.
    $exists = $false
    try { $ErrorActionPreference = "Continue"; gh release view "v$ver" --repo $Repo 2>&1 | Out-Null; $exists = ($LASTEXITCODE -eq 0) }
    finally { $ErrorActionPreference = "Stop" }
    if ($exists) { throw "release v$ver already exists - bump the version (-Version x.y.z)" }
    if (-not $Notes -and $NotesFile) { $Notes = [IO.File]::ReadAllText((Resolve-Path $NotesFile)) }
    if (-not $Notes) {
        # Commit subjects since the previous release tag (Co-Authored-By lines are not subjects).
        $prev = ""
        try {
            $ErrorActionPreference = "Continue"
            git fetch --tags --quiet 2>$null   # release tags are created on GitHub by `gh release create`
            $prev = (git describe --tags --abbrev=0 2>$null)
        } finally { $ErrorActionPreference = "Stop" }
        $range = if ($prev) { "$prev..HEAD" } else { "HEAD" }
        $items = @(git log $range --no-merges --pretty=format:"- %s")
        $Notes = "## Изменения`n" + ($(if ($items.Count) { $items -join "`n" } else { "- Исправления и улучшения" }))
    }
    $Notes += "`n`n---`nУстановщик: $setupName. Установленные копии обновятся сами."
    gh release create "v$ver" $setup "$setup.sha256" --repo $Repo --title "ZAPRET GUI $ver" --notes $Notes
    if ($LASTEXITCODE) { throw "gh release create failed" }
    Write-Host "    https://github.com/$Repo/releases/tag/v$ver" -ForegroundColor Green
}
Write-Host "`nDone." -ForegroundColor Green
