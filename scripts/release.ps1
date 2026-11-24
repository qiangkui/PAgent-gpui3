param(
    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$versionLine = Select-String -Path 'Cargo.toml' -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if ($null -eq $versionLine) { throw 'Cargo.toml 里没有 version' }
$version = $versionLine.Matches[0].Groups[1].Value
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stage = Join-Path $root "dist\PAgent-$version-$stamp"
$zip = "$stage.zip"

if (-not $SkipTests) {
    Write-Host '== Rust tests ==' -ForegroundColor Cyan
    cargo test --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo test 失败' }

    Write-Host '== Engine check and tests ==' -ForegroundColor Cyan
    Push-Location (Join-Path $root 'dsh-flow')
    try {
        npm.cmd ci --ignore-scripts
        if ($LASTEXITCODE -ne 0) { throw 'npm ci 失败' }
        npm.cmd run check
        if ($LASTEXITCODE -ne 0) { throw 'npm run check 失败' }
        npm.cmd test
        if ($LASTEXITCODE -ne 0) { throw 'npm test 失败' }
    } finally {
        Pop-Location
    }
}

Write-Host '== Release build ==' -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build --release 失败' }

New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'target\release\dsh-flow-ui.exe') -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'CHANGELOG.md') -Destination $stage

$engine = Join-Path $stage 'dsh-flow'
New-Item -ItemType Directory -Force -Path $engine | Out-Null
Get-ChildItem -LiteralPath (Join-Path $root 'dsh-flow') -Force |
    Where-Object { $_.Name -notin @('node_modules', 'out', '.dsh-flow') } |
    Copy-Item -Destination $engine -Recurse -Force

$plugins = Join-Path $root 'plugins'
if (Test-Path -LiteralPath $plugins) {
    $stagedPlugins = Join-Path $stage 'plugins'
    New-Item -ItemType Directory -Force -Path $stagedPlugins | Out-Null
    Get-ChildItem -LiteralPath $plugins -Force |
        Where-Object { $_.Name -ne 'node_modules' } |
        Copy-Item -Destination $stagedPlugins -Recurse -Force
}

Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash

Write-Host "Release: $zip" -ForegroundColor Green
Write-Host "SHA256:  $hash" -ForegroundColor Green
Write-Host "Size:    $([math]::Round((Get-Item -LiteralPath $zip).Length / 1MB, 2)) MB"