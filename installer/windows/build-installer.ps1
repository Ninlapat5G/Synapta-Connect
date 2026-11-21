# Builds SynaptaConnectSetup-v<version>.exe: the windowed installer wrapping a built synapta-connect.exe.
# Uses the C# compiler that ships with .NET Framework 4.x, so no SDK is needed on the build machine.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Agent,
    [Parameter(Mandatory = $true)][string]$Notices,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Output
)

$ErrorActionPreference = 'Stop'
$Here = $PSScriptRoot
$ProjectRoot = Split-Path (Split-Path $Here -Parent) -Parent
$Agent = (Resolve-Path -LiteralPath $Agent).Path
$Notices = (Resolve-Path -LiteralPath $Notices).Path
if ($Version -notmatch '^[0-9]+[.][0-9]+[.][0-9]+$') { throw "Invalid version: $Version" }
$Output = [IO.Path]::GetFullPath($Output)

$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (!(Test-Path -LiteralPath $compiler)) { throw '.NET Framework 4.x C# compiler not found.' }

# Small generated resources: the agent hash checked before install, and the version shown on screen.
$work = Join-Path ([IO.Path]::GetTempPath()) ('synapta-setup-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $hashFile = Join-Path $work 'agent.sha256'
    $versionFile = Join-Path $work 'version.txt'
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($hashFile, (Get-FileHash -LiteralPath $Agent -Algorithm SHA256).Hash.ToLowerInvariant(), $utf8)
    [IO.File]::WriteAllText($versionFile, $Version, $utf8)

    $resources = @(
        "$Agent,agent.exe",
        "$hashFile,agent.sha256",
        "$versionFile,version.txt",
        "$Notices,notices.txt",
        "$(Join-Path $ProjectRoot 'frontend\assets\connect-install.jpg'),hero.jpg",
        "$(Join-Path $Here 'assets\logo.png'),logo.png",
        "$(Join-Path $Here 'assets\synapta-connect.ico'),icon.ico",
        "$(Join-Path $Here 'fonts\Anuphan-Regular.ttf'),Anuphan-Regular.ttf",
        "$(Join-Path $Here 'fonts\Anuphan-SemiBold.ttf'),Anuphan-SemiBold.ttf",
        "$(Join-Path $Here 'fonts\ChakraPetch-SemiBold.ttf'),ChakraPetch-SemiBold.ttf"
    ) | ForEach-Object { "/resource:$_" }

    $arguments = @('/nologo', '/target:winexe', '/platform:anycpu', '/optimize+', '/codepage:65001',
        '/reference:System.Windows.Forms.dll', '/reference:System.Drawing.dll',
        "/win32icon:$(Join-Path $Here 'assets\synapta-connect.ico')",
        "/win32manifest:$(Join-Path $Here 'app.manifest')",
        "/out:$Output") + $resources + @((Join-Path $Here 'Setup.cs'))
    & $compiler @arguments
    if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed: $LASTEXITCODE" }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

$check = Start-Process -FilePath $Output -ArgumentList '--verify-payload' -NoNewWindow -Wait -PassThru
if ($check.ExitCode -ne 0) { throw 'Embedded agent verification failed.' }
Write-Output $Output
