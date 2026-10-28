# Windows-side checks of the built exe. Uses only what ships with Windows: installs and downloads nothing,
# needs no admin rights, touches no servers except (optionally) the local test sshd on 127.0.0.1:2221.
#
# Run:  powershell -ExecutionPolicy Bypass -File .\dist\win-tests\run-win-tests.ps1
# Report: report-<date>.txt next to this script. Send it back as is.
$ErrorActionPreference = 'Continue'
$here   = Split-Path -Parent $MyInvocation.MyCommand.Path
$report = Join-Path $here ("report-{0}.txt" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
function Log($m) { $m | Tee-Object -FilePath $report -Append }
$script:fail = 0
$script:pass = 0
function Check($name, $ok, $info) {
  if ($ok) { Log "PASS  $name"; $script:pass++ } else { Log "FAIL  $name  $info"; $script:fail++ }
}

Log "d256Shell Windows tests  $(Get-Date -Format o)"
Log "OS: $([Environment]::OSVersion.VersionString)  PowerShell $($PSVersionTable.PSVersion)"

# ---------------------------------------------------------------- the exe itself
$exe = (Resolve-Path (Join-Path $here '..\d256Shell\d256Shell.exe') -ErrorAction SilentlyContinue)
if (-not $exe) { Log "FAIL  d256Shell.exe not found next to win-tests (expected ..\d256Shell\d256Shell.exe)"; exit 1 }
$exe = $exe.Path
$size = (Get-Item $exe).Length
Log "exe: $exe"
Log "SHA256: $((Get-FileHash $exe -Algorithm SHA256).Hash)"
Log ("size: {0:N1} MB" -f ($size / 1MB))
Check 'exe size < 25 MB' ($size -lt 25MB) "$size bytes"

$vi = (Get-Item $exe).VersionInfo
Log "VersionInfo: ProductName='$($vi.ProductName)' FileDescription='$($vi.FileDescription)' CompanyName='$($vi.CompanyName)' LegalCopyright='$($vi.LegalCopyright)' FileVersion='$($vi.FileVersion)' ProductVersion='$($vi.ProductVersion)'"
Check 'ProductName = d256 Shell' ($vi.ProductName -eq 'd256 Shell') $vi.ProductName
Check 'CompanyName = dest1256' ($vi.CompanyName -eq 'dest1256') $vi.CompanyName
Check 'LegalCopyright contains dest1256' ($vi.LegalCopyright -like '*dest1256*') $vi.LegalCopyright
Check 'FileDescription present' (-not [string]::IsNullOrWhiteSpace($vi.FileDescription)) ''

# manifest: asInvoker, PerMonitorV2, long paths
$bytes = [System.IO.File]::ReadAllBytes($exe)
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
Check 'manifest: asInvoker' ($text.Contains('asInvoker')) ''
Check 'manifest: PerMonitorV2' ($text.Contains('PerMonitorV2')) ''
Check 'manifest: longPathAware' ($text.Contains('longPathAware')) ''
Check 'no UPX packer' (-not $text.Contains('UPX!')) ''

# ---------------------------------------------------------------- Microsoft Defender
$mp = Join-Path $env:ProgramFiles 'Windows Defender\MpCmdRun.exe'
if (Test-Path $mp) {
  $o = & $mp -Scan -ScanType 3 -File $exe 2>&1 | Out-String
  Log ($o.Trim())
  Check 'Microsoft Defender: no threats found' ($LASTEXITCODE -eq 0) "MpCmdRun exit code $LASTEXITCODE"
} else {
  Log 'SKIP  Defender: MpCmdRun.exe not found (another antivirus?)'
}

# ---------------------------------------------------------------- system ssh
$ssh = "$env:SystemRoot\System32\OpenSSH\ssh.exe"
if (Test-Path $ssh) {
  $v = (& $ssh -V 2>&1 | Out-String).Trim()
  Log "ssh.exe: $v"
  $g = & $ssh -F none -G localhost 2>&1 | Out-String
  Check 'ssh -F none -G' ($g -match '(?m)^hostname localhost') $g
  $g = & $ssh -F none -G -- -oProxyCommand=x 2>&1 | Out-String
  Check 'ssh: "--" protects against option-like hosts' ($g -match '(?m)^hostname -oproxycommand=x' -and $g -notmatch '(?m)^proxycommand x') $g
  $d = Join-Path $env:TEMP 'Shell port test'
  New-Item -ItemType Directory -Force $d | Out-Null
  $kh = ($d -replace '\\','/') + '/known hosts'
  $g = & $ssh -F none -G -o "UserKnownHostsFile=`"$kh`"" localhost 2>&1 | Out-String
  Check 'UserKnownHostsFile with spaces (quoted)' ($g -match [regex]::Escape($kh)) $g
  Remove-Item -Recurse -Force $d -ErrorAction SilentlyContinue
  if ($v -match 'OpenSSH_for_Windows_(\d+)\.(\d+)') {
    $ok = ([int]$Matches[1] -gt 8) -or ([int]$Matches[1] -eq 8 -and [int]$Matches[2] -ge 4)
    Log ("askpass mode: " + $(if ($ok) { 'SSH_ASKPASS_REQUIRE supported (>= 8.4)' } else { 'old ssh: d256Shell will use the careful expect fallback' }))
  }

  # a real login through SSH_ASKPASS to the local test server (only if it is running), path with spaces
  $tcp = New-Object Net.Sockets.TcpClient
  $up = $false
  try { $iar = $tcp.BeginConnect('127.0.0.1', 2221, $null, $null); $up = $iar.AsyncWaitHandle.WaitOne(800) -and $tcp.Connected } catch {}
  $tcp.Close()
  if ($up) {
    $dir = Join-Path $env:TEMP 'Тест Shell port'
    New-Item -ItemType Directory -Force $dir | Out-Null
    $helper = Join-Path $dir 'askpass.cmd'
    Set-Content -Path $helper -Value "@echo off`r`necho testpass" -Encoding ASCII
    $env:SSH_ASKPASS = $helper
    $env:SSH_ASKPASS_REQUIRE = 'force'
    $env:DISPLAY = 'd256shell'
    $khf = (Join-Path $dir 'known_hosts') -replace '\\','/'
    $out = & $ssh -F none -o "UserKnownHostsFile=`"$khf`"" -o StrictHostKeyChecking=accept-new -o NumberOfPasswordPrompts=1 -p 2221 tester@127.0.0.1 'echo marker-$((6*7))' 2>&1 | Out-String
    Remove-Item Env:SSH_ASKPASS, Env:SSH_ASKPASS_REQUIRE, Env:DISPLAY -ErrorAction SilentlyContinue
    Check 'ssh.exe + SSH_ASKPASS (script in a folder with spaces/Cyrillic) logs in to the test server' ($out -match 'marker-42') $out
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
  } else {
    Log 'SKIP  login test: test sshd on 127.0.0.1:2221 is not running (docker compose up -d in .devcontainer)'
  }
} else {
  Check 'ssh.exe present' $false "not found at $ssh"
}

# ---------------------------------------------------------------- run the app: folder with spaces and Cyrillic, startup, memory, no traces
function Snapshot {
  $s = @{}
  foreach ($p in @($env:APPDATA, $env:LOCALAPPDATA, $env:TEMP, $env:USERPROFILE)) {
    if ($p -and (Test-Path $p)) {
      Get-ChildItem -LiteralPath $p -Force -ErrorAction SilentlyContinue | ForEach-Object { $s["$($_.FullName)"] = $_.LastWriteTimeUtc.Ticks }
    }
  }
  $reg = @()
  foreach ($k in @('HKCU:\Software', 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run')) {
    if (Test-Path $k) { $reg += (Get-ChildItem $k -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) }
  }
  return @{ files = $s; reg = ($reg | Sort-Object) }
}

$test = Join-Path $env:TEMP 'Проверка Shell port'
Remove-Item -Recurse -Force $test -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $test | Out-Null
Copy-Item -Recurse -Force (Join-Path $here '..\d256Shell') $test
$app = Join-Path $test 'd256Shell\d256Shell.exe'
$before = Snapshot

$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $app -WorkingDirectory (Split-Path $app) -PassThru
$shown = $false
for ($i = 0; $i -lt 100; $i++) {
  Start-Sleep -Milliseconds 100
  $p.Refresh()
  if ($p.MainWindowHandle -ne 0) { $shown = $true; break }
}
$sw.Stop()
Check 'window appears (folder with spaces and Cyrillic)' $shown 'no main window within 10 s'
if ($shown) {
  Log ("startup to window: {0} ms" -f $sw.ElapsedMilliseconds)
  Check 'startup < 1.5 s' ($sw.ElapsedMilliseconds -lt 1500) "$($sw.ElapsedMilliseconds) ms"
}
Start-Sleep -Seconds 6
# memory: the app plus its WebView2 child processes
$all = @($p.Id)
$queue = @($p.Id)
while ($queue.Count) {
  $cur = $queue[0]; $queue = $queue[1..($queue.Count)]
  $kids = Get-CimInstance Win32_Process -Filter "ParentProcessId=$cur" -ErrorAction SilentlyContinue
  foreach ($k in $kids) { $all += $k.ProcessId; $queue += $k.ProcessId }
}
$mem = 0
foreach ($id in $all) { $pr = Get-Process -Id $id -ErrorAction SilentlyContinue; if ($pr) { $mem += $pr.WorkingSet64 } }
Log ("idle memory (app + WebView2, no session): {0:N0} MB in {1} processes" -f ($mem / 1MB), $all.Count)
Check 'idle memory < 250 MB (app + WebView2)' (($mem / 1MB) -lt 250) ("{0:N0} MB" -f ($mem / 1MB))
Check 'data folder created next to the exe' (Test-Path (Join-Path (Split-Path $app) 'data')) ''

foreach ($id in ($all | Sort-Object -Descending)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Seconds 1
$after = Snapshot
$newFiles = @($after.files.Keys | Where-Object { -not $before.files.ContainsKey($_) -and $_ -notlike "$test*" })
$newReg = @($after.reg | Where-Object { $before.reg -notcontains $_ })
Log "new files/folders outside the app folder: $($newFiles.Count)"
$newFiles | Select-Object -First 20 | ForEach-Object { Log "   $_" }
Log "new registry keys under HKCU\Software / Run: $($newReg.Count)"
$newReg | ForEach-Object { Log "   $_" }
Check 'no autorun entry' (@($newReg | Where-Object { $_ -like '*\Run*' }).Count -eq 0) ''
Check 'no d256Shell traces outside the app folder (names)' (@($newFiles | Where-Object { $_ -match 'd256Shell|d256shell|dest1256' }).Count -eq 0 -and @($newReg | Where-Object { $_ -match 'd256Shell|d256shell|dest1256' }).Count -eq 0) ($newFiles -join '; ')

# askpass helper mode: same exe, must finish in a fraction of a second and never show a window
$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $app
$psi.Arguments = 'Password:'
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$psi.RedirectStandardOutput = $true
$psi.EnvironmentVariables['D256SHELL_ASKPASS'] = '1'
$sw2 = [Diagnostics.Stopwatch]::StartNew()
$hp = [Diagnostics.Process]::Start($psi)
$done = $hp.WaitForExit(3000)
$sw2.Stop()
Check 'askpass helper mode exits quickly' ($done -and $sw2.ElapsedMilliseconds -lt 2000) "$($sw2.ElapsedMilliseconds) ms, exited=$done"
if (-not $done) { Stop-Process -Id $hp.Id -Force -ErrorAction SilentlyContinue }
Check 'askpass helper shows no window' ($hp.MainWindowHandle -eq 0) ''

Remove-Item -Recurse -Force $test -ErrorAction SilentlyContinue

Log ""
Log "RESULT: $script:pass passed, $script:fail failed"
Log "Not automated (see README, manual checklist): drag-and-drop in/out, DPI 150%, two monitors, ConPTY with the real terminal, closing a tab kills ssh.exe."
if ($script:fail -gt 0) { exit 1 }
