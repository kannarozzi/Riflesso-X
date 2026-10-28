# Installs the Riflesso X command line (`riflesso`) for the current user:
#
#   irm https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/install.ps1 | iex
#
# Downloads riflesso.exe from the latest release, checks it against the
# published SHA-256, puts it in "%LOCALAPPDATA%\Riflesso X CLI" (its data stays
# in that folder too) and adds the folder to the user's PATH.
# Remove it with uninstall.ps1.

& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is much faster without it
    $repo = 'kannarozzi/Riflesso-X'
    $dir = Join-Path $env:LOCALAPPDATA 'Riflesso X CLI'
    # For testing a build that is not published yet: a local zip.
    $localZip = $env:RIFLESSO_CLI_ZIP

    $temp = Join-Path ([IO.Path]::GetTempPath()) "riflesso-install-$PID"
    New-Item -ItemType Directory -Force $temp | Out-Null
    try {
        $zip = Join-Path $temp 'riflesso.zip'
        if ($localZip) {
            Copy-Item $localZip $zip
            $version = 'local build'
        } else {
            Write-Host 'Looking for the latest Riflesso X release...'
            $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'riflesso-install' }
            $asset = $release.assets | Where-Object { $_.name -like '*_x64-cli.zip' } | Select-Object -First 1
            $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' } | Select-Object -First 1
            if (-not $asset) { throw "The latest release ($($release.tag_name)) has no command-line build." }
            $version = $release.tag_name

            Write-Host "Downloading $($asset.name)..."
            Invoke-WebRequest $asset.browser_download_url -OutFile $zip -UseBasicParsing
            if ($sums) {
                # Saved to a file: Windows PowerShell returns downloads typed as binary as bytes.
                $sumsFile = Join-Path $temp 'SHA256SUMS.txt'
                Invoke-WebRequest $sums.browser_download_url -OutFile $sumsFile -UseBasicParsing
                $line = Get-Content $sumsFile | Where-Object { $_ -match [regex]::Escape($asset.name) } | Select-Object -First 1
                $expected = ($line -split '\s+')[0]
                $actual = (Get-FileHash $zip -Algorithm SHA256).Hash
                if (-not $expected -or $actual -ne $expected) {
                    throw "The download does not match its published SHA-256 checksum. Nothing was installed."
                }
            }
        }

        Expand-Archive $zip -DestinationPath $temp -Force
        $exe = Get-ChildItem $temp -Recurse -Filter 'riflesso.exe' | Select-Object -First 1
        if (-not $exe) { throw 'riflesso.exe is missing from the download.' }

        if (Get-Process riflesso -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dir\*" }) {
            throw 'riflesso is running: press Ctrl+C in its terminal, then install again.'
        }
        New-Item -ItemType Directory -Force $dir | Out-Null
        # Replace the program files, keep the "data" folder (identity, remembered devices).
        Get-ChildItem $exe.DirectoryName -File | Copy-Item -Destination $dir -Force

        # User PATH, read without expanding %VARIABLES% and written back the same way.
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        $path = [string]$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
        # Only appended: the rest of the PATH stays exactly as it was, so that
        # uninstall.ps1 can give it back unchanged.
        if (-not (@($path -split ';') | Where-Object { $_.TrimEnd('\') -eq $dir })) {
            $updated = if (-not $path) { $dir } elseif ($path.EndsWith(';')) { "$path$dir;" } else { "$path;$dir" }
            $key.SetValue('Path', $updated, 'ExpandString')
            # Lets Explorer, and the terminals it opens, see the new PATH.
            [Environment]::SetEnvironmentVariable('RIFLESSO_INSTALL', '1', 'User')
            [Environment]::SetEnvironmentVariable('RIFLESSO_INSTALL', $null, 'User')
        }
        $key.Close()
        if (($env:Path -split ';') -notcontains $dir) { $env:Path = "$env:Path;$dir" }

        Write-Host ''
        Write-Host "Riflesso X command line installed ($version) in $dir" -ForegroundColor Green
        Write-Host 'Run "riflesso" to start receiving, or "riflesso --help" for the options.'
    } finally {
        Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
