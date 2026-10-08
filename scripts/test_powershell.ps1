# Noninteractive tests for the actual generated Windows PowerShell integration.
# The native interactive console still requires separate manual/ConPTY testing.
param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/ii.exe'))

$ErrorActionPreference = 'Stop'
$Binary = (Resolve-Path -LiteralPath $Binary).ProviderPath
$originalPath = $env:PATH
$initialDirectory = (Get-Location).ProviderPath
$temporary = $null
$originalTarget = $env:II_POWERSHELL_TEST_TARGET
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
    # Exercise the actual function against a native protocol fixture. Interactive
    # Windows console acceptance remains separate from these dispatch tests.
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ("ii-shell-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $temporary | Out-Null
    $target = Join-Path $temporary 'init'
    New-Item -ItemType Directory -Path $target | Out-Null
    $fixture = @'
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|argument| argument == "init") {
        print!("II_TEST_SETUP");
    } else {
        print!("{}", std::env::var("II_POWERSHELL_TEST_TARGET").unwrap());
    }
}
'@
    $source = Join-Path $temporary 'fixture.rs'
    [IO.File]::WriteAllText($source, $fixture)
    & rustc --edition=2024 $source -o (Join-Path $temporary 'ii.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Could not build shell protocol fixture' }
    $env:PATH = "$temporary$([IO.Path]::PathSeparator)$env:PATH"
    $env:II_POWERSHELL_TEST_TARGET = $target
    foreach ($arguments in @(@('--hidden', 'init'), @('--', 'init'))) {
        $output = ii @arguments
        if ($null -ne $output) { throw 'Navigation leaked the selected path instead of changing directory' }
        if ((Get-Location).ProviderPath -ne $target) { throw 'A directory named init did not change location' }
        Set-Location -LiteralPath $initialDirectory
    }
    if ((ii init powershell) -ne 'II_TEST_SETUP') { throw 'Initialization stopped passing through' }
    if ((Get-Location).ProviderPath -ne $initialDirectory) { throw 'Initialization changed directory' }
    Write-Output 'PowerShell alias resolution, setup, and informational passthrough: OK'
    $global:LASTEXITCODE = 0
} finally {
    Set-Location -LiteralPath $initialDirectory
    $env:II_POWERSHELL_TEST_TARGET = $originalTarget
    $env:PATH = $originalPath
    if ($null -ne $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
