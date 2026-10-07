# Noninteractive tests for the actual generated Windows PowerShell integration.
# The native interactive console still requires separate manual/ConPTY testing.
param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/ii.exe'))

$ErrorActionPreference = 'Stop'
$Binary = (Resolve-Path -LiteralPath $Binary).ProviderPath
$originalPath = $env:PATH
$initialDirectory = (Get-Location).ProviderPath
try {
    $env:PATH = "$(Split-Path -Parent $Binary)$([IO.Path]::PathSeparator)$originalPath"
    # Reproduce the built-in alias collision rather than assuming a clean session.
    Set-Alias -Name ii -Value Invoke-Item -Scope Global -Force
    Set-Alias -Name ii -Value Invoke-Item -Scope Script -Force
    $setup = & $Binary init powershell | Out-String
    if ($LASTEXITCODE -ne 0) { throw 'Native initialization failed' }
    Invoke-Expression $setup
    if ((Get-Command ii).CommandType -ne 'Function') {
        throw 'The built-in Invoke-Item alias still shadows the ii function'
    }
    $version = ii --version
    if ($LASTEXITCODE -ne 0 -or $version -notmatch '^ii \d+\.\d+\.\d+$') {
        throw "Version passthrough failed: $version"
    }
    $helpText = ii --no-preview --help | Out-String
    if ($LASTEXITCODE -ne 0 -or $helpText -notmatch 'SHELL SETUP') {
        throw 'Help passthrough failed'
    }
    if ((Get-Location).ProviderPath -ne $initialDirectory) {
        throw 'An informational command changed the current directory'
    }
    # Re-evaluating a profile must remain harmless after the alias is gone.
    Invoke-Expression $setup
    if ((ii --version) -ne $version) { throw 'Initialization is not idempotent' }
    $regenerated = ii init powershell | Out-String
    if ($regenerated -ne $setup) { throw 'Initialization passthrough changed the script' }
    Write-Output 'PowerShell alias resolution, setup, and informational passthrough: OK'
    $global:LASTEXITCODE = 0
} finally {
    $env:PATH = $originalPath
}
