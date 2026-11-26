[CmdletBinding()]
param(
    [ValidateSet('Both', 'Backend', 'Panel')]
    [string]$Component = 'Both',
    [string]$OutputPath = '',
    [string]$SigningKey = '',
    [string]$PublicKey = '',
    [string]$ReleaseTool = '',
    # Portable exe built elsewhere (e.g. on app01); skips the local Portable build
    [string]$PortableExe = ''
)

$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$Frontend = Join-Path $ProjectRoot 'frontend'
$Backend = Join-Path $ProjectRoot 'backend'
if (!$OutputPath) { $OutputPath = Join-Path $ProjectRoot 'dist/release' }
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
if ($SigningKey) {
    $SigningKey = (Resolve-Path -LiteralPath $SigningKey).Path
    if ($PublicKey -notmatch '^[0-9a-fA-F]{64}$' -or $PublicKey -eq ('0' * 64)) {
        throw 'A signing key requires the matching nonzero Ed25519 public key (-PublicKey).'
    }
}
if ($PublicKey -and ($PublicKey -notmatch '^[0-9a-fA-F]{64}$' -or $PublicKey -eq ('0' * 64))) {
    throw 'Invalid Ed25519 public key.'
}
if ($ReleaseTool) { $ReleaseTool = (Resolve-Path -LiteralPath $ReleaseTool).Path }
if ($PortableExe) { $PortableExe = (Resolve-Path -LiteralPath $PortableExe).Path }

function Invoke-Checked {
    param([string]$Executable, [string[]]$Arguments)
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Executable exited with code $LASTEXITCODE" }
}

function Copy-Notices([string]$Destination) {
    $notice = Join-Path $Destination 'THIRD_PARTY_NOTICES.txt'
    Copy-Item -LiteralPath (Join-Path $ProjectRoot 'docs/THIRD_PARTY_NOTICES.txt') -Destination $notice -Force
    foreach ($name in 'react', 'react-dom', 'scheduler', 'lucide-react', 'tailwindcss') {
        $license = Get-Content -LiteralPath (Join-Path $Frontend "node_modules/$name/LICENSE") -Raw
        Add-Content -LiteralPath $notice -Encoding UTF8 -Value ("`r`n=== " + $name + " ===`r`n" + $license)
    }
    foreach ($name in 'motion', 'framer-motion', 'motion-dom', 'motion-utils') {
        $license = Get-Content -LiteralPath (Join-Path $Frontend "node_modules/$name/LICENSE.md") -Raw
        Add-Content -LiteralPath $notice -Encoding UTF8 -Value ("`r`n=== " + $name + " ===`r`n" + $license)
    }
    foreach ($file in 'LICENSE-MIT', 'LICENSE-APACHE-2.0') {
        $license = Get-Content -LiteralPath (Join-Path $Frontend "node_modules/@tauri-apps/api/$file") -Raw
        Add-Content -LiteralPath $notice -Encoding UTF8 -Value ("`r`n=== @tauri-apps/api " + $file + " ===`r`n" + $license)
    }
    foreach ($name in 'LICENSE-Anuphan', 'LICENSE-Chakra') {
        $license = Get-Content -LiteralPath (Join-Path $Frontend "assets/fonts/$name") -Raw -Encoding UTF8
        Add-Content -LiteralPath $notice -Encoding UTF8 -Value ("`r`n=== " + $name + " ===`r`n" + $license)
    }
    # winpty.dll / winpty-agent.exe are embedded in the agent for Windows without ConPTY (Server 2016)
    $license = Get-Content -LiteralPath (Join-Path $Backend 'vendor/winpty/LICENSE') -Raw -Encoding UTF8
    Add-Content -LiteralPath $notice -Encoding UTF8 -Value ("`r`n=== winpty 0.4.4-dev (binaries from node-pty 1.1.0) ===`r`n" + $license)
}

$PanelVersion = (Get-Content -LiteralPath (Join-Path $Frontend 'package.json') -Raw | ConvertFrom-Json).version
$BackendVersion = [regex]::Match((Get-Content -LiteralPath (Join-Path $Backend 'Cargo.toml') -Raw), '(?m)^version = "([0-9]+[.][0-9]+[.][0-9]+)"').Groups[1].Value
if ($PanelVersion -notmatch '^[0-9]+[.][0-9]+[.][0-9]+$' -or !$BackendVersion) { throw 'Missing component version.' }
$BuildBackend = $Component -ne 'Panel'
$BuildPanel = $Component -ne 'Backend'
$Target = Join-Path $Backend 'target/release'
$PreviousPublicKey = $env:SYNAPTA_RELEASE_PUBKEY
try {
    Invoke-Checked 'npm.cmd' @('--prefix', $Frontend, 'ci', '--no-audit', '--no-fund')
    Invoke-Checked 'npm.cmd' @('--prefix', $Frontend, 'run', 'build')
    if ($PublicKey) { $env:SYNAPTA_RELEASE_PUBKEY = $PublicKey }
    Push-Location $Backend
    try {
        if ($BuildBackend) {
            if ((& rustc -vV | Out-String) -notmatch 'host: x86_64-pc-windows-msvc') { throw 'Backend release requires the Windows x64 MSVC Rust toolchain.' }
            Invoke-Checked 'npm.cmd' @('--prefix', $Frontend, 'run', 'build:desktop')
            Invoke-Checked 'cargo' @('build', '--release', '--locked', '--features', 'desktop', '--bin', 'synapta-connect', '--bin', 'synapta-release')
            # Portable builds without "desktop": tauri-build links its icon resource into every binary,
            # which collides with Portable's own icon (CVT1100 duplicate resource) at link time.
            if (!$PortableExe) { Invoke-Checked 'cargo' @('build', '--release', '--locked', '--bin', 'synapta-connect-portable') }
        } elseif ($SigningKey -and !$ReleaseTool) {
            Invoke-Checked 'cargo' @('build', '--release', '--locked', '--bin', 'synapta-release')
        }
    } finally {
        Pop-Location
    }
    if (!$ReleaseTool) { $ReleaseTool = Join-Path $Target 'synapta-release.exe' }

    if ($BuildBackend) {
        $destination = Join-Path $OutputPath "backend/v$BackendVersion"
        New-Item -ItemType Directory -Path $destination -Force | Out-Null
        $exe = Join-Path $Target 'synapta-connect.exe'
        if ($SigningKey) {
            Invoke-Checked $ReleaseTool @('package', '--exe', $exe, '--out', $destination, '--version', $BackendVersion, '--key', $SigningKey)
            Invoke-Checked $ReleaseTool @('verify', '--dir', $destination, '--pubkey', $PublicKey)
        } else {
            Copy-Item -LiteralPath $exe -Destination (Join-Path $destination "SynaptaConnect-v$BackendVersion.exe") -Force
        }
        Copy-Notices $destination
        if (!$PortableExe) { $PortableExe = Join-Path $Target 'synapta-connect-portable.exe' }
        Copy-Item -LiteralPath $PortableExe -Destination (Join-Path $destination "SynaptaConnectPortable-v$BackendVersion.exe") -Force
        # Windowed installer for first-time installs; updates keep using the bare agent exe above
        $installer = Join-Path $destination "SynaptaConnectSetup-v$BackendVersion.exe"
        & (Join-Path $ProjectRoot 'installer/windows/build-installer.ps1') -Agent $exe -Notices (Join-Path $destination 'THIRD_PARTY_NOTICES.txt') -Version $BackendVersion -Output $installer | Out-Null
        Write-Output $destination
    }
    if ($BuildPanel) {
        $destination = Join-Path $OutputPath "panel/v$PanelVersion"
        New-Item -ItemType Directory -Path $destination -Force | Out-Null
        $builtPanel = Join-Path $Frontend 'dist'
        if ($SigningKey) {
            Invoke-Checked $ReleaseTool @('package-panel', '--web', $builtPanel, '--out', $destination, '--version', $PanelVersion, '--key', $SigningKey)
            Invoke-Checked $ReleaseTool @('verify-panel', '--dir', $destination, '--pubkey', $PublicKey)
        } else {
            $webOutput = Join-Path $destination 'web'
            New-Item -ItemType Directory -Path $webOutput -Force | Out-Null
            Get-ChildItem -LiteralPath $builtPanel | Copy-Item -Destination $webOutput -Recurse -Force
        }
        Copy-Notices $destination
        Write-Output $destination
    }
    if (!$SigningKey) { Write-Output 'Local build. Supply -SigningKey and -PublicKey to produce verified update files.' }
} finally {
    $env:SYNAPTA_RELEASE_PUBKEY = $PreviousPublicKey
}
