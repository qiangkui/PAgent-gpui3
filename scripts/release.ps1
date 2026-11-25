param(
    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

<#
Move a running program aside so this build can replace it.

Windows locks the executable of a running process, so the compiler fails with
"access denied" and the release stops half way. Renaming a locked file is
allowed and the process keeps running from the new name, which frees the old
name for the build. Displaced copies are deleted on the next run, once the
program using them has quit.
#>
function Clear-RunningBinary {
    $release = Join-Path $root 'target\release'
    Get-ChildItem -LiteralPath $release -Filter 'dsh-flow-ui.running-*.exe' -ErrorAction SilentlyContinue |
        ForEach-Object {
            try {
                Remove-Item -LiteralPath $_.FullName -Force -ErrorAction Stop
            } catch {
                # Still the program someone is using; next run will take it.
            }
        }
    $exe = Join-Path $release 'dsh-flow-ui.exe'
    if (-not (Test-Path -LiteralPath $exe)) { return }
    $locked = $false
    try {
        # Opening for write with no sharing is the cheapest lock test and leaves
        # the file untouched.
        $probe = [System.IO.File]::Open($exe, 'Open', 'ReadWrite', 'None')
        $probe.Close()
    } catch [System.UnauthorizedAccessException] {
        # Either a running program or the read-only attribute; only the former
        # is fixed by renaming.
        $locked = -not (Get-Item -LiteralPath $exe).IsReadOnly
    } catch {
        $locked = $true
    }
    if (-not $locked) { return }
    $held = Join-Path $release ("dsh-flow-ui.running-{0}.exe" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
    try {
        Move-Item -LiteralPath $exe -Destination $held -Force -ErrorAction Stop
    } catch {
        # Some other lock (an editor, a scanner) forbids the rename too; only a
        # person closing the program can free it, so say so instead of failing
        # in the linker with a wall of text.
        throw "程序仍在占用 $exe 且无法改名，请先关闭 PAgent 再运行发布脚本。"
    }
    Write-Host "== 程序仍在运行，旧文件已改名为 $(Split-Path -Leaf $held) ==" -ForegroundColor Yellow
}

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
Clear-RunningBinary
cargo build --release
if ($LASTEXITCODE -ne 0) {
    # A lock that appeared mid-build (an editor, a scanner) gets one more
    # chance once the file has been moved aside.
    Clear-RunningBinary
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release 失败' }
}
$exe = Join-Path $root 'target\release\dsh-flow-ui.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw '没有生成 target\release\dsh-flow-ui.exe' }
$helper = Join-Path $root 'target\release\dsh-sandbox.exe'
if (-not (Test-Path -LiteralPath $helper)) { throw '没有生成 target\release\dsh-sandbox.exe' }

New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -LiteralPath $exe -Destination $stage
Copy-Item -LiteralPath $helper -Destination $stage
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

<#
The staging folder exists only to be zipped: once the archive holds it, keeping
it doubles the space a release takes. Older packages go too, because a folder
full of near-identical zips is a backup nobody asked for -- one release folder,
one package, and the hash printed below to identify it.
#>
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
$kept = Join-Path $root 'dist'
$older = Get-ChildItem -LiteralPath $kept -Filter 'PAgent-*-*.zip' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -ne $zip }
if ($older) {
    $older | Remove-Item -Force -ErrorAction SilentlyContinue
    Write-Host ("== 已删除旧的发布包 {0} 个 ==" -f $older.Count) -ForegroundColor Yellow
}

Write-Host "Release: $zip" -ForegroundColor Green
Write-Host "SHA256:  $hash" -ForegroundColor Green
Write-Host "Size:    $([math]::Round((Get-Item -LiteralPath $zip).Length / 1MB, 2)) MB"