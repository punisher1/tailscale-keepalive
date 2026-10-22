# Install tailscale-keepalive from GitHub Releases.
#
#   irm https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.ps1 | iex
#
# Optional environment variables:
#   TS_KEEPALIVE_VERSION          release tag to install (default: latest)
#   TS_KEEPALIVE_INSTALL_DIR      where to put the binary (default: %LOCALAPPDATA%\Programs\tailscale-keepalive)
#   TS_KEEPALIVE_TARGET           peer IP; required when installing the service
#   TS_KEEPALIVE_INSTALL_SERVICE  set to 1 to also register the Windows service (needs Administrator)

$ErrorActionPreference = 'Stop'

$Repo    = 'punisher1/tailscale-keepalive'
$BinName = 'tailscale-keepalive'
$Version = $env:TS_KEEPALIVE_VERSION

$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64' }
    'ARM64' { 'aarch64' }
    default { throw "unsupported architecture: $($env:PROCESSOR_ARCHITECTURE)" }
}
$target = "$arch-pc-windows-msvc"

if ($Version) {
    $base = "https://github.com/$Repo/releases/download/$Version"
} else {
    $base = "https://github.com/$Repo/releases/latest/download"
}
$url = "$base/$BinName-$target.zip"

Write-Host "downloading $url"
$tmp = Join-Path $env:TEMP ("tska-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
try {
    $zip = Join-Path $tmp 'pkg.zip'
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $exe = Get-ChildItem -Path $tmp -Recurse -Filter "$BinName.exe" |
        Select-Object -First 1 -ExpandProperty FullName
    if (-not $exe) { throw "archive did not contain $BinName.exe" }

    if ($env:TS_KEEPALIVE_INSTALL_DIR) {
        $installDir = $env:TS_KEEPALIVE_INSTALL_DIR
    } else {
        $installDir = Join-Path $env:LOCALAPPDATA "Programs\$BinName"
    }
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null

    $dest = Join-Path $installDir "$BinName.exe"
    Copy-Item -Path $exe -Destination $dest -Force
    Write-Host "installed $dest"

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $installDir) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $installDir), 'User')
        Write-Host "added $installDir to the user PATH (open a new shell to use it)"
    }
    $env:Path = "$env:Path;$installDir"

    if ($env:TS_KEEPALIVE_INSTALL_SERVICE -eq '1') {
        if (-not $env:TS_KEEPALIVE_TARGET) {
            throw "set `$env:TS_KEEPALIVE_TARGET to the peer's Tailscale IP before installing the service"
        }
        Write-Host 'registering the Windows service'
        & $dest -t $env:TS_KEEPALIVE_TARGET install
        Write-Host 'check it with: tailscale-keepalive status'
    } else {
        Write-Host ''
        Write-Host 'next steps:'
        Write-Host "  $BinName -t 100.x.y.z               # try it in the foreground"
        Write-Host "  $BinName -t 100.x.y.z install       # register the service (Administrator)"
    }
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
