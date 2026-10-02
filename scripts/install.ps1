# Installs the Reins desktop app (rewarden.exe) on Windows, from PowerShell:
#
#   irm <site>/install.ps1 | iex
#
# Downloads the latest GitHub release's zip for this computer (x86_64 or Arm), checks it against the release's
# SHA256SUMS, installs rewarden.exe to %LOCALAPPDATA%\Programs\Reins (REWARDEN_INSTALL_DIR to change), adds that folder
# to your user PATH, and restarts the background service if it is installed. Later updates: `rewarden update` (which
# also checks the release signature) or this script again.
#
# Settings, from the environment:
#   REWARDEN_VERSION       a release tag (v0.2.0) instead of the latest release
#   REWARDEN_INSTALL_DIR   where rewarden.exe goes
#   REWARDEN_RELEASE_DIR   a folder holding SHA256SUMS and the zip, instead of downloading them (tests, offline)
#
# Everything runs in a script block, so `irm | iex` leaves no variables behind and an error stops only the script, not
# your PowerShell window.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue' # Windows PowerShell downloads far slower while drawing progress
    # The site the next steps name. scripts/release-desktop.sh rewrites this line to the site it publishes to.
    $DefaultSite = "https://reins2fa.com"
    $Repo = 'katulevskiy/reins'

    function Fail([string] $Message) {
        throw "rewarden install: $Message"
    }

    # The processor Windows runs on (a 32-bit PowerShell on 64-bit Windows sees x86 in PROCESSOR_ARCHITECTURE).
    $cpu = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    $triple = switch ($cpu) {
        'AMD64' { 'x86_64-pc-windows-msvc' }
        'ARM64' { 'aarch64-pc-windows-msvc' }
        default { Fail "this processor ($cpu) is not supported yet" }
    }
    $installDir = if ($env:REWARDEN_INSTALL_DIR) { $env:REWARDEN_INSTALL_DIR } else {
        Join-Path $env:LOCALAPPDATA 'Programs\Reins'
    }

    # Windows PowerShell 5.1 does not offer TLS 1.2 by itself everywhere.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("reins-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        # Where SHA256SUMS and the zip come from.
        if ($env:REWARDEN_RELEASE_DIR) {
            $from = $env:REWARDEN_RELEASE_DIR
            $fetch = { param($file, $to) Copy-Item -LiteralPath (Join-Path $from $file) -Destination $to }
            $tag = '(local)'
        } else {
            $tag = $env:REWARDEN_VERSION
            if (-not $tag) {
                try {
                    $tag = (Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$Repo/releases/latest").tag_name
                } catch {
                    Fail "cannot find the latest release of $Repo ($($_.Exception.Message))"
                }
            }
            if ($tag -notmatch '^v\d+\.\d+\.\d+$') { Fail "$tag is not a release tag (vX.Y.Z)" }
            $from = "https://github.com/$Repo/releases/download/$tag"
            $fetch = { param($file, $to) Invoke-WebRequest -UseBasicParsing -Uri "$from/$file" -OutFile $to }
        }

        try { & $fetch 'SHA256SUMS' (Join-Path $tmp 'SHA256SUMS') } catch { Fail "the release $tag has no SHA256SUMS" }
        # `<64 hex digits>  reins-desktop-<version>-<target>.zip`
        $line = Get-Content (Join-Path $tmp 'SHA256SUMS') |
            Where-Object { $_ -match "^([0-9a-f]{64}) +\*?(reins-desktop-\d+\.\d+\.\d+-$([regex]::Escape($triple))\.zip)$" } |
            Select-Object -First 1
        if (-not $line) { Fail "the release $tag has no build for this computer ($triple)" }
        $null = $line -match "^([0-9a-f]{64}) +\*?(\S+)$"
        $want = $Matches[1]
        $zip = $Matches[2]
        $name = $zip -replace '\.zip$', ''

        Write-Host "Downloading $zip ($tag)..."
        $zipPath = Join-Path $tmp $zip
        try { & $fetch $zip $zipPath } catch { Fail "the download failed ($($_.Exception.Message))" }
        $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToLowerInvariant()
        if ($got -ne $want) { Fail "the download does not match the published checksum; nothing was installed" }

        Expand-Archive -LiteralPath $zipPath -DestinationPath $tmp
        $new = Join-Path (Join-Path $tmp $name) 'rewarden.exe'
        if (-not (Test-Path -LiteralPath $new)) { Fail "$zip has no $name\rewarden.exe" }
        $version = & $new --version
        if ($LASTEXITCODE -ne 0) { Fail "the downloaded program does not run on this computer" }

        # In place, even while rewarden.exe runs (an MCP bridge of a harness): Windows lets a running program be renamed
        # but not overwritten. The renamed copy goes at the next update (`rewarden update` removes rewarden.exe.*.old).
        $exe = Join-Path $installDir 'rewarden.exe'
        $wasInstalled = Test-Path -LiteralPath $exe
        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        Get-ChildItem -LiteralPath $installDir -Filter 'rewarden.exe.*.old' -ErrorAction SilentlyContinue |
            Remove-Item -Force -ErrorAction SilentlyContinue
        if ($wasInstalled) {
            $aside = "rewarden.exe.$PID-$([Guid]::NewGuid().ToString('N')).old"
            Rename-Item -LiteralPath $exe -NewName $aside
            try {
                Move-Item -LiteralPath $new -Destination $exe
            } catch {
                Rename-Item -LiteralPath (Join-Path $installDir $aside) -NewName 'rewarden.exe'
                Fail "cannot replace $exe ($($_.Exception.Message))"
            }
            Remove-Item -LiteralPath (Join-Path $installDir $aside) -Force -ErrorAction SilentlyContinue
        } else {
            Move-Item -LiteralPath $new -Destination $exe
        }
        Write-Host "Installed $version at $exe"

        # The user's PATH in the registry, for new terminals (read without expanding %VARIABLES%, and written back as
        # the same kind of value, so entries like %USERPROFILE%\bin stay as they are); and this session's.
        $envKey = Get-Item -Path 'HKCU:\Environment'
        $userPath = [string] $envKey.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
        $kind = if ($envKey.GetValueNames() -contains 'Path') { $envKey.GetValueKind('Path') } else { 'ExpandString' }
        $dirs = @($userPath -split ';' | Where-Object { $_ })
        if (($dirs | ForEach-Object { $_.TrimEnd('\') }) -notcontains $installDir.TrimEnd('\')) {
            Set-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Value ((@($dirs) + $installDir) -join ';') -Type $kind
            # Setting a variable through .NET tells running programs (Explorer) that the environment changed.
            [Environment]::SetEnvironmentVariable('REWARDEN_INSTALL_REFRESH', '1', 'User')
            [Environment]::SetEnvironmentVariable('REWARDEN_INSTALL_REFRESH', $null, 'User')
            Write-Host "Added $installDir to your PATH (open a new terminal for it to take effect)."
        }
        if (($env:Path -split ';' | ForEach-Object { $_.TrimEnd('\') }) -notcontains $installDir.TrimEnd('\')) {
            $env:Path = "$env:Path;$installDir"
        }

        # The background service runs its own copy of the program: made again from this one and restarted.
        $run = Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Reins' -ErrorAction SilentlyContinue
        if ($run) {
            & $exe service uninstall | Out-Null
            & $exe service install | Out-Null
            if ($LASTEXITCODE -eq 0) { Write-Host "Restarted the background service." }
        }

        if (-not $wasInstalled) {
            Write-Host ""
            Write-Host "Next:"
            Write-Host "  rewarden login"
            Write-Host "      sign in; your phone shows a key: approve only if it matches the one printed here"
            Write-Host "  rewarden resume"
            Write-Host "      start the background service and send GitHub git through it (rewarden pause undoes it)"
        }
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
