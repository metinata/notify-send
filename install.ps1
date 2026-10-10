# Installs notify-send for Windows for the current user:
#
#     irm https://github.com/metinata/notify-send/releases/latest/download/install.ps1 | iex
#
# Downloads the latest release for this machine (x64 or ARM64), checks its
# SHA-256 checksum, puts it in %LOCALAPPDATA%\Programs\notify-send and adds that
# folder to the user's PATH. No administrator rights are needed. Run it again to
# update.

# A script block of its own, so that nothing leaks into the session that runs `iex`.
& {
    $ErrorActionPreference = 'Stop'
    # Windows PowerShell downloads much faster without a progress bar.
    $ProgressPreference = 'SilentlyContinue'
    # Older Windows PowerShell setups do not turn on TLS 1.2, which GitHub requires.
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $release = 'https://github.com/metinata/notify-send/releases/latest/download'
    $dir = Join-Path $env:LOCALAPPDATA 'Programs\notify-send'

    # The machine's architecture. An x64 PowerShell on ARM64 runs under emulation
    # and sees AMD64 in the environment, so Windows is asked; IsWow64Process2
    # exists from Windows 10 1709 on. Before that, a 32-bit PowerShell sees the
    # machine's architecture in PROCESSOR_ARCHITEW6432.
    $machine = try {
        if (-not ('NotifySendInstaller.Machine' -as [type])) {
            Add-Type -Namespace NotifySendInstaller -Name Machine -MemberDefinition @'
[DllImport("kernel32.dll")]
public static extern bool IsWow64Process2(IntPtr process, out ushort processMachine, out ushort nativeMachine);
'@
        }
        $processMachine = [uint16]0; $nativeMachine = [uint16]0
        # -1 stands for the current process.
        if (-not [NotifySendInstaller.Machine]::IsWow64Process2([IntPtr]-1, [ref]$processMachine, [ref]$nativeMachine)) {
            throw 'IsWow64Process2 failed'
        }
        switch ($nativeMachine) {
            0x8664 { 'AMD64' }
            0xAA64 { 'ARM64' }
            default { 'machine type 0x{0:X4}' -f $nativeMachine }
        }
    } catch {
        if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    }
    $arch = switch ($machine) {
        'AMD64' { 'x86_64' }
        'ARM64' { 'aarch64' }
        default { throw "notify-send needs 64-bit Windows on x64 or ARM64, not $machine." }
    }
    $name = "notify-send-$arch-pc-windows-msvc.zip"

    $temp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        Write-Host "Downloading $name"
        $zip = Join-Path $temp $name
        Invoke-WebRequest -Uri "$release/$name" -OutFile $zip -UseBasicParsing
        Invoke-WebRequest -Uri "$release/$name.sha256" -OutFile "$zip.sha256" -UseBasicParsing
        $expected = (Get-Content -Path "$zip.sha256" -Raw).Split(' ')[0].Trim()
        $actual = (Get-FileHash -Path $zip -Algorithm SHA256).Hash
        if ($actual -ne $expected) {
            throw "The download is damaged: its SHA-256 is $actual, but the release says $expected."
        }

        Expand-Archive -Path $zip -DestinationPath (Join-Path $temp 'files')
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $exe = Join-Path $dir 'notify-send.exe'
        # Copies that earlier updates moved aside, once nothing runs them any more.
        Get-ChildItem -Path $dir -Filter 'notify-send.exe.*.old' |
            Remove-Item -Force -ErrorAction SilentlyContinue
        # A notify-send that waits for a notification keeps its exe in use, but
        # Windows lets a running exe be renamed, so it is moved aside first.
        $old = $null
        try {
            if (Test-Path $exe) {
                $old = "$exe.$([Guid]::NewGuid().ToString('N')).old"
                Rename-Item -Path $exe -NewName (Split-Path $old -Leaf)
            }
            Copy-Item -Path (Join-Path $temp 'files\*') -Destination $dir -Force
        } catch {
            if ($old -and (Test-Path $old) -and -not (Test-Path $exe)) {
                Rename-Item -Path $old -NewName 'notify-send.exe'
            }
            throw "Cannot replace the files in ${dir}: $($_.Exception.Message)"
        }
        if ($old) {
            Remove-Item -Path $old -Force -ErrorAction SilentlyContinue
        }
    } finally {
        Remove-Item -Path $temp -Recurse -Force -ErrorAction SilentlyContinue
    }

    # Add the folder to the user's PATH. The registry value is read and written
    # as is, so that entries such as %USERPROFILE%\... keep their variables.
    $environment = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
    try {
        $path = $environment.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
        # Entries are compared as Windows reads them: with their variables
        # expanded, and without quotes or a trailing backslash.
        $present = ($path -split ';') | Where-Object {
            [Environment]::ExpandEnvironmentVariables($_).Trim().Trim('"').TrimEnd('\') -eq $dir
        }
        $added = -not $present
        if ($added) {
            $environment.SetValue('Path', ($path.TrimEnd(';') + ";$dir").TrimStart(';'), 'ExpandString')
        }
    } finally {
        $environment.Close()
    }
    if (($env:Path -split ';') -notcontains $dir) {
        $env:Path = "$($env:Path.TrimEnd(';'));$dir"
    }
    if ($added) {
        # Tell Windows that the environment changed, so that terminals opened
        # from now on find notify-send too.
        try {
            if (-not ('NotifySendInstaller.Win32' -as [type])) {
                Add-Type -Namespace NotifySendInstaller -Name Win32 -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint msg, UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result);
'@
            }
            $result = [UIntPtr]::Zero
            $HWND_BROADCAST = [IntPtr]0xffff; $WM_SETTINGCHANGE = 0x1A; $SMTO_ABORTIFHUNG = 2
            [void][NotifySendInstaller.Win32]::SendMessageTimeout($HWND_BROADCAST, $WM_SETTINGCHANGE, [UIntPtr]::Zero, 'Environment', $SMTO_ABORTIFHUNG, 5000, [ref]$result)
            $told = $true
        } catch {
            $told = $false
        }
    }

    $version = & (Join-Path $dir 'notify-send.exe') --version
    Write-Host "Installed $version in $dir"
    if ($added) {
        Write-Host "Added $dir to your PATH."
        if (-not $told) {
            Write-Host 'Other terminals find notify-send after you sign out and in again.'
        }
    }
    Write-Host 'Run "notify-send Hello" to see a notification, or "notify-send --help" for the options.'
}
