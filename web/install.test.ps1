# Mock network and process launch; never downloads or installs software.
$ErrorActionPreference = 'Stop'
$global:sonoJotInstallerTest = @{
  checksum = '9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08'
  version = '1.2.3'
  launched = 0
}
function Invoke-RestMethod {
  param($Uri, $MaximumRedirection)
  if ($MaximumRedirection -ne 0 -or $Uri -notlike 'https://releases.sonojot.com/*') { throw 'Unsafe request' }
  if ($Uri -like '*/latest.json') { return @{ version = $global:sonoJotInstallerTest.version; platforms = @{ 'windows-x86_64' = @{ url = 'verified-by-test' } } } }
  if ($Uri -ne 'https://releases.sonojot.com/v1.2.3/SonoJot_1.2.3_x64-setup.exe.sha256') { throw 'Wrong checksum URL' }
  return $global:sonoJotInstallerTest.checksum
}
function Invoke-WebRequest {
  param($Uri, $OutFile, [switch]$UseBasicParsing, $MaximumRedirection)
  if ($MaximumRedirection -ne 0 -or $Uri -ne 'https://releases.sonojot.com/v1.2.3/SonoJot_1.2.3_x64-setup.exe') { throw 'Unsafe download' }
  [IO.File]::WriteAllBytes($OutFile, [Text.Encoding]::ASCII.GetBytes('test'))
}
function Start-Process {
  param($FilePath, $ArgumentList, $WindowStyle, [switch]$Wait, [switch]$PassThru)
  if ($ArgumentList -ne '/S' -or $WindowStyle -ne 'Hidden' -or -not $Wait) { throw 'Wrong installer arguments' }
  $global:sonoJotInstallerTest.launched++
  return @{ ExitCode = 0 }
}
& "$PSScriptRoot/public/install.ps1"
if ($global:sonoJotInstallerTest.launched -ne 1) { throw 'Verified installer did not run' }
$global:sonoJotInstallerTest.checksum = '0' * 64
try { & "$PSScriptRoot/public/install.ps1"; throw 'Expected checksum failure' }
catch { if ($_.Exception.Message -notlike '*checksum does not match*') { throw } }
if ($global:sonoJotInstallerTest.launched -ne 1) { throw 'Unverified installer ran' }
$global:sonoJotInstallerTest.version = '../bad'
try { & "$PSScriptRoot/public/install.ps1"; throw 'Expected metadata failure' }
catch { if ($_.Exception.Message -notlike '*no valid version*') { throw } }
# `irm | iex` runs in the caller's scope; the installer must not change its preferences.
$global:sonoJotInstallerTest.version = '1.2.3'
$global:sonoJotInstallerTest.checksum = '9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08'
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'Continue'
Invoke-Expression (Get-Content -Raw -LiteralPath "$PSScriptRoot/public/install.ps1")
if ($ErrorActionPreference -ne 'Continue' -or $ProgressPreference -ne 'Continue' -or (Test-Path variable:checksum)) {
  throw 'The installer leaked state into the calling session'
}
if ($global:sonoJotInstallerTest.launched -ne 2) { throw 'Verified installer did not run under iex' }
Write-Host 'Installer checks passed: verified launch, checksum mismatch, invalid metadata, caller scope.'
