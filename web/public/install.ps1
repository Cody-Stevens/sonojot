# SonoJot installer for 64-bit Windows. Re-run to update.
# `irm | iex` runs in the caller's session, so keep every preference and variable local.
& {
$ErrorActionPreference = 'Stop'
# Windows PowerShell's progress bar slows large downloads severalfold.
$ProgressPreference = 'SilentlyContinue'
if (-not [Environment]::Is64BitOperatingSystem) { throw 'SonoJot requires 64-bit Windows.' }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$releaseHost = 'https://releases.sonojot.com'
# No redirects: every request stays on the HTTPS release host.
$release = Invoke-RestMethod -Uri "$releaseHost/latest.json" -MaximumRedirection 0
$version = $release.version
if ($version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
  throw 'The release metadata has no valid version.'
}
if (-not $release.platforms.'windows-x86_64') { throw 'This release has no Windows installer.' }
$name = "SonoJot_${version}_x64-setup.exe"
$url = "$releaseHost/v$version/$name"
$checksum = (Invoke-RestMethod -Uri "$url.sha256" -MaximumRedirection 0).Trim()
if ($checksum -notmatch '^[a-fA-F0-9]{64}$') { throw 'The release checksum is missing or invalid.' }
$work = Join-Path ([IO.Path]::GetTempPath()) ("sonojot-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$file = Join-Path $work $name
try {
  Write-Host "Downloading SonoJot $version..."
  Invoke-WebRequest -Uri $url -OutFile $file -UseBasicParsing -MaximumRedirection 0
  if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $checksum) {
    throw 'The installer checksum does not match. Nothing was installed.'
  }
  $process = Start-Process -FilePath $file -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne 0) { throw "The installer exited with code $($process.ExitCode)." }
  Write-Host "SonoJot $version is installed. Open it from the Start menu."
} finally {
  if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force }
  Remove-Item -LiteralPath $work -Force
}
}
