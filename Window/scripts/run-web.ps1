param(
    [ValidateSet('dev', 'build', 'test', 'preview', 'tauri:dev', 'tauri:build', 'package:exe')]
    [string]$Task = 'dev'
)

$ErrorActionPreference = 'Stop'
$taskWindowRoot = Split-Path -Parent $PSScriptRoot
$taskRepoRoot = Split-Path -Parent $taskWindowRoot
$taskNodeVersion = (Get-Content -LiteralPath (Join-Path $taskWindowRoot '.node-version') -Raw).Trim()
$taskPortableRoot = Join-Path $taskRepoRoot ('.tools/node-v' + $taskNodeVersion + '-win-x64')
$taskNodeExe = Join-Path $taskPortableRoot 'node.exe'
if (-not (Test-Path -LiteralPath $taskNodeExe)) {
    $taskSystemNode = Get-Command node -ErrorAction SilentlyContinue
    if (-not $taskSystemNode) { throw "Node $taskNodeVersion 환경을 준비해주세요." }
    $taskNodeExe = $taskSystemNode.Source
}
if ((& $taskNodeExe --version) -ne ('v' + $taskNodeVersion)) {
    throw "Window/.node-version의 Node $taskNodeVersion 버전이 필요합니다."
}
$taskNodeRoot = Split-Path -Parent $taskNodeExe
$taskNpmCli = Join-Path $taskNodeRoot 'node_modules/npm/bin/npm-cli.js'
if (-not (Test-Path -LiteralPath $taskNpmCli)) { throw '선택한 Node 환경의 npm을 찾지 못했습니다.' }

$taskOriginalPath = $env:PATH
$taskOriginalCache = $env:npm_config_cache
$taskOriginalCargo = $env:CARGO_HOME
$taskOriginalRustup = $env:RUSTUP_HOME
$taskOriginalDbDir = $env:STUDYAPP_DB_DIR
$taskOriginalLocation = Get-Location
try {
    $env:PATH = $taskNodeRoot + ';' + $env:PATH
    $env:npm_config_cache = Join-Path $taskRepoRoot '.tools/npm-cache'
    $taskCargoRoot = Join-Path $taskRepoRoot '.tools/cargo'
    if (Test-Path -LiteralPath (Join-Path $taskCargoRoot 'bin/cargo.exe')) {
        $env:CARGO_HOME = $taskCargoRoot
        $env:RUSTUP_HOME = Join-Path $taskRepoRoot '.tools/rustup'
        $env:PATH = (Join-Path $taskCargoRoot 'bin') + ';' + $env:PATH
    }
    if ($Task -eq 'tauri:dev' -and -not $env:STUDYAPP_DB_DIR) {
        $env:STUDYAPP_DB_DIR = Join-Path $taskRepoRoot '.tools/runtime'
    }
    Set-Location -LiteralPath $taskWindowRoot
    & $taskNodeExe $taskNpmCli run $Task
    if ($LASTEXITCODE -ne 0) { throw "'$Task' 실행 실패 (exit $LASTEXITCODE)" }
}
finally {
    $env:PATH = $taskOriginalPath
    $env:npm_config_cache = $taskOriginalCache
    $env:CARGO_HOME = $taskOriginalCargo
    $env:RUSTUP_HOME = $taskOriginalRustup
    $env:STUDYAPP_DB_DIR = $taskOriginalDbDir
    Set-Location -LiteralPath $taskOriginalLocation.Path
}
