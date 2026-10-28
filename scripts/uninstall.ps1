# Removes the Riflesso X command line installed by install.ps1:
#
#   irm https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/uninstall.ps1 | iex
#
# Deletes "%LOCALAPPDATA%\Riflesso X CLI" (program and data) and its PATH entry.

& {
    $ErrorActionPreference = 'Stop'
    $dir = Join-Path $env:LOCALAPPDATA 'Riflesso X CLI'

    if (Get-Process riflesso -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dir\*" }) {
        throw 'riflesso is running: press Ctrl+C in its terminal, then uninstall again.'
    }

    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    $path = [string]$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    # Empty entries (such as a trailing ';') are kept, to leave the rest as it was.
    $all = @($path -split ';')
    $kept = @($all | Where-Object { $_.TrimEnd('\') -ne $dir })
    if ($kept.Count -ne $all.Count) {
        $key.SetValue('Path', $kept -join ';', 'ExpandString')
        # Lets Explorer, and the terminals it opens, see the new PATH.
        [Environment]::SetEnvironmentVariable('RIFLESSO_INSTALL', '1', 'User')
        [Environment]::SetEnvironmentVariable('RIFLESSO_INSTALL', $null, 'User')
    }
    $key.Close()
    $env:Path = (@($env:Path -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $dir })) -join ';'

    if (Test-Path $dir) {
        Remove-Item $dir -Recurse -Force
        Write-Host "Riflesso X command line removed from $dir" -ForegroundColor Green
    } else {
        Write-Host 'The Riflesso X command line was not installed.'
    }
}
