[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingPlainTextForPassword', 'AdminPasswordPlainText', Justification = 'test-only script; non-interactive automation')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingPlainTextForPassword', 'SessionUserPasswordPlainText', Justification = 'test-only script; non-interactive automation')]
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Hostname,

    [Parameter(Mandatory = $true)]
    [string]$AdminUsername,

    [Parameter()]
    [securestring]$AdminPassword,

    [Parameter()]
    [string]$AdminPasswordPlainText = '',

    [Parameter()]
    [switch]$PromptAdminPassword,

    # The task uses this user's existing interactive token, not a password logon.
    [Parameter(Mandatory = $true)]
    [string]$SessionUserName,

    [Parameter()]
    [securestring]$SessionUserPassword,

    [Parameter()]
    [string]$SessionUserPasswordPlainText = '',

    [Parameter()]
    [switch]$PromptSessionUserPassword,

    # Where to write capture artifacts on the VM.
    [Parameter()]
    [string]$RemoteDir = $(Join-Path $(if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' }) 'session-probe'),

    # Where to copy the PNG on this machine.
    [Parameter()]
    [string]$LocalOutputDir = '.\artifacts\session-probe',

    [Parameter()]
    [int]$WaitSeconds = 60,

    # If set, uses the newest Active session whose SessionName matches /irdp-tcp/i.
    # If not set, it will search by UserName.
    [Parameter()]
    [switch]$PreferIrdpTcpSession
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force

$adminCred = if ($AdminPassword) {
    Resolve-TestVmCredential -Credential ([pscredential]::new($AdminUsername, $AdminPassword))
} else {
    Resolve-TestVmCredential -Username $AdminUsername -Password $AdminPasswordPlainText `
        -PromptPassword:$PromptAdminPassword -Hostname $Hostname
}

if ($SessionUserPassword -or $SessionUserPasswordPlainText -or $PromptSessionUserPassword.IsPresent) {
    Write-Warning 'Session-user password parameters are ignored; capture uses the existing interactive token.'
}

$captureHelperText = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'session-capture.helpers.ps1') -Raw

New-Item -ItemType Directory -Path $LocalOutputDir -Force | Out-Null

$session = $null
$session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
try {
    $remoteResult = Invoke-Command -Session $session -ArgumentList $SessionUserName, $RemoteDir, $WaitSeconds, $PreferIrdpTcpSession.IsPresent, $captureHelperText -ScriptBlock {
        param(
            [string]$SessionUserName,
            [string]$RemoteDir,
            [int]$WaitSeconds,
            [bool]$PreferIrdpTcpSession,
            [string]$CaptureHelperText
        )

        Set-StrictMode -Version Latest
        $ErrorActionPreference = 'Stop'

        . ([scriptblock]::Create($CaptureHelperText))
        $taskUserName = $SessionUserName
        $taskUserSid = ([Security.Principal.NTAccount]$taskUserName).Translate([Security.Principal.SecurityIdentifier]).Value

        function Get-QuerySessionRows {
            $text = (cmd.exe /c "query session") -join "`n"
            $lines = $text -split "`r?`n" | Where-Object { $_.Trim() -ne '' } | Select-Object -Skip 1
            $rows = @()
            foreach ($line in $lines) {
                $l = $line.TrimStart()
                if ($l.StartsWith('>')) { $l = $l.Substring(1).TrimStart() }

                $parts = $l -split '\s{2,}'
                if ($parts.Count -lt 3) { continue }

                $idIndex = -1
                for ($i = 0; $i -lt $parts.Count; $i++) {
                    $tmp = $null
                    if ([int]::TryParse($parts[$i], [ref]$tmp)) { $idIndex = $i; break }
                }
                if ($idIndex -lt 1 -or ($idIndex + 1) -ge $parts.Count) { continue }

                $id = $null
                [void][int]::TryParse($parts[$idIndex], [ref]$id)
                $state = $parts[$idIndex + 1]

                $sessionName = $parts[0]
                $userName = ''
                if ($idIndex -ge 2) { $userName = $parts[1] }

                $rows += [pscustomobject]@{ SessionName = $sessionName; UserName = $userName; Id = $id; State = $state; Raw = $l }
            }
            $rows
        }

        $rows = Get-QuerySessionRows

        $interactiveStates = @('Active', 'Conn')
        $ownedSessionIds = @(Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" -ErrorAction Stop |
            ForEach-Object {
                $owner = Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid -ErrorAction Stop
                if ($owner.ReturnValue -eq 0 -and $owner.Sid -eq $taskUserSid) { [int]$_.SessionId }
            })
        $targetRow = Select-CaptureSession -Rows $rows -OwnedSessionIds $ownedSessionIds -PreferIrdpTcpSession $PreferIrdpTcpSession

        if (-not $targetRow) {
            return [pscustomobject]@{ Ok = $false; Reason = "No connected desktop owned by '$taskUserName' was found."; QuerySession = (cmd.exe /c "query session") }
        }

        if (-not ($interactiveStates -contains $targetRow.State)) {
            return [pscustomobject]@{
                Ok = $false
                Reason = "Target session is '$($targetRow.State)'. Interactive screenshot probe usually requires a connected session (Active/Conn)."
                SessionId = $targetRow.Id
                SessionName = $targetRow.SessionName
                UserName = $targetRow.UserName
                QuerySession = (cmd.exe /c "query session")
            }
        }

        $capDir = Join-Path $RemoteDir ('captures\' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $capDir -Force | Out-Null

        $jsonPath = Join-Path $capDir 'last-capture.json'
        $logPath = Join-Path $capDir 'last-capture.log'
        Remove-Item -LiteralPath $jsonPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $logPath -Force -ErrorAction SilentlyContinue

        $capScript = Join-Path $capDir 'capture-screen.ps1'

        @'
param([string]$OutputDirectory, [int]$ExpectedSessionId, [string]$ExpectedUserSid)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$outDir = $OutputDirectory
New-Item -ItemType Directory -Path $outDir -Force | Out-Null

$sessionId = (Get-Process -Id $PID).SessionId
$userSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
if ($sessionId -ne $ExpectedSessionId -or $userSid -ne $ExpectedUserSid) {
    throw 'Task Scheduler started the capture in a different session or as a different user'
}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'

$jsonPath = Join-Path $outDir 'last-capture.json'
$logPath = Join-Path $outDir 'last-capture.log'

try {
    "[$(Get-Date -Format o)] Capture script starting (pre-UI) in session $sessionId" | Set-Content -LiteralPath $logPath -Encoding UTF8
} catch {
    # Best-effort: don't fail before we enter the main try/catch.
}

function Sample-BlackRatio([string]$pngPath) {
  try {
    $bmp = [System.Drawing.Bitmap]::new($pngPath)
    try {
      $rand = [System.Random]::new(0)
      $n = 2000
      $black = 0
      for ($i=0; $i -lt $n; $i++) {
        $x = $rand.Next(0, $bmp.Width)
        $y = $rand.Next(0, $bmp.Height)
        $c = $bmp.GetPixel($x, $y)
        if ($c.R -eq 0 -and $c.G -eq 0 -and $c.B -eq 0) { $black++ }
      }
      return [math]::Round(($black / $n), 4)
    }
    finally {
      $bmp.Dispose()
    }
  } catch {
    return $null
  }
}

try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing

    "[$(Get-Date -Format o)] Starting session probe capture in session $sessionId" | Add-Content -LiteralPath $logPath -Encoding UTF8

  $form = New-Object System.Windows.Forms.Form
  $form.Text = 'IronRDP session probe'
  $form.StartPosition = 'Manual'
  $form.Left = 50
  $form.Top = 50
  $form.Width = 900
  $form.Height = 260
  $form.TopMost = $true

  $label = New-Object System.Windows.Forms.Label
  $label.Dock = 'Fill'
  $label.TextAlign = 'MiddleCenter'
  $label.Font = New-Object System.Drawing.Font('Segoe UI', 28, [System.Drawing.FontStyle]::Bold)
  $label.Text = "Session $sessionId`r`n$stamp"
  $form.Controls.Add($label)

  $form.Show()
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.Application]::DoEvents()
  Start-Sleep -Milliseconds 700
  [System.Windows.Forms.Application]::DoEvents()

  $outPng = Join-Path $outDir ("session-{0}-{1}.png" -f $sessionId, $stamp)
  $method = $null
  $bmp = $null
  $gfx = $null

  try {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $gfx = [System.Drawing.Graphics]::FromImage($bmp)
    $gfx.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $method = 'CopyFromScreen'
  } catch {
    if ($gfx) { $gfx.Dispose(); $gfx = $null }
    if ($bmp) { $bmp.Dispose(); $bmp = $null }

    $bmp = New-Object System.Drawing.Bitmap $form.Width, $form.Height
    $rect = New-Object System.Drawing.Rectangle 0, 0, $form.Width, $form.Height
    $form.DrawToBitmap($bmp, $rect)
    $method = 'DrawToBitmap'
  }

  $bmp.Save($outPng, [System.Drawing.Imaging.ImageFormat]::Png)
  $blackRatio = Sample-BlackRatio -pngPath $outPng
  "[$(Get-Date -Format o)] Saved PNG ($method) blackRatio=${blackRatio}: $outPng" | Add-Content -LiteralPath $logPath -Encoding UTF8

  if ($gfx) { $gfx.Dispose() }
  if ($bmp) { $bmp.Dispose() }
  $form.Close(); $form.Dispose()

  @{
    Ok = $true
    SessionId = $sessionId
    UserSid = $userSid
    Screenshot = $outPng
    CaptureMethod = $method
    SampleBlackRatio = $blackRatio
    Time = (Get-Date).ToString('o')
  } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $jsonPath -Encoding UTF8

} catch {
  "[$(Get-Date -Format o)] ERROR: $($_.Exception.Message)" | Add-Content -LiteralPath $logPath -Encoding UTF8
  @{
    Ok = $false
    SessionId = $sessionId
    UserSid = $userSid
    Time = (Get-Date).ToString('o')
    Error = ($_ | Out-String)
  } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $jsonPath -Encoding UTF8
  throw
}
'@ | Set-Content -LiteralPath $capScript -Encoding UTF8

        $taskName = "IronRDP-SessionProbe-$([guid]::NewGuid().ToString('N'))"
        $arguments = @(
            '-NoProfile',
            '-STA',
            '-ExecutionPolicy', 'Bypass',
            '-File', $capScript,
            '-OutputDirectory', $capDir,
            '-ExpectedSessionId', [string]$targetRow.Id,
            '-ExpectedUserSid', $taskUserSid
        )

        $argumentString = ($arguments | ForEach-Object {
            if ($_ -match '[\s"'']') { '"' + ($_ -replace '"', '\\"') + '"' } else { $_ }
        }) -join ' '

        $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argumentString
        $principal = New-ScheduledTaskPrincipal -UserId $taskUserName -LogonType Interactive -RunLevel Highest
        $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -MultipleInstances IgnoreNew

        try {
            Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Settings $settings -Force | Out-Null
        } catch {
            return [pscustomobject]@{
                Ok = $false
                Reason = 'Failed to register InteractiveToken scheduled task.'
                TaskName = $taskName
                Error = ($_ | Out-String)
            }
        }

        try {
            # UNTESTED: Verify RunEx uses the selected session on the target Windows build before changing task hosting.
            $scheduler = New-Object -ComObject 'Schedule.Service'
            $scheduler.Connect()
            $task = $scheduler.GetFolder('\').GetTask($taskName)
            $null = $task.RunEx($null, 4, [int]$targetRow.Id, $null)

            $deadline = (Get-Date).AddSeconds($WaitSeconds)
            while ((Get-Date) -lt $deadline) {
                if (Test-Path -LiteralPath $jsonPath) { break }
                Start-Sleep -Seconds 1
            }

            if (-not (Test-Path -LiteralPath $jsonPath)) {
                $taskInfo = $null
                try {
                    $i = Get-ScheduledTaskInfo -TaskName $taskName
                    $taskInfo = ($i | Format-List * | Out-String -Width 400)
                } catch { }
                $logText = $null
                try { if (Test-Path -LiteralPath $logPath) { $logText = Get-Content -LiteralPath $logPath -Raw -ErrorAction SilentlyContinue } } catch { }
                return [pscustomobject]@{
                    Ok = $false
                    Reason = "Timed out waiting for capture output after $WaitSeconds seconds."
                    SessionId = $targetRow.Id
                    SessionName = $targetRow.SessionName
                    QuerySession = (cmd.exe /c "query session")
                    TaskInfo = $taskInfo
                    LogPath = $logPath
                    LogText = $logText
                }
            }

            $payload = Get-Content -LiteralPath $jsonPath -Raw | ConvertFrom-Json
            Assert-CaptureSession -ExpectedSessionId $targetRow.Id -ExpectedUserSid $taskUserSid -Capture $payload
            if (($payload.PSObject.Properties.Name -contains 'Ok') -and (-not $payload.Ok)) {
                $logText = $null
                try { if (Test-Path -LiteralPath $logPath) { $logText = Get-Content -LiteralPath $logPath -Raw -ErrorAction SilentlyContinue } } catch { }
                return [pscustomobject]@{ Ok = $false; Reason = 'Capture script failed.'; Json = (Get-Content -LiteralPath $jsonPath -Raw); LogPath = $logPath; LogText = $logText }
            }

            $png = [string]$payload.Screenshot
            if (-not (Test-Path -LiteralPath $png)) {
                return [pscustomobject]@{ Ok = $false; Reason = 'Capture JSON exists but PNG was not found.'; Json = (Get-Content -LiteralPath $jsonPath -Raw) }
            }

            return [pscustomobject]@{
                Ok = $true
                SessionId = [int]$payload.SessionId
                SessionName = $targetRow.SessionName
                UserName = $targetRow.UserName
                RemotePng = $png
                RemoteJson = $jsonPath
                RemoteLog = $logPath
                CaptureMethod = [string]$payload.CaptureMethod
                SampleBlackRatio = $payload.SampleBlackRatio
                QuerySession = (cmd.exe /c "query session")
            }
        }
        finally {
            Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            try { Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue | Out-Null } catch { }
        }
    }

    if (-not $remoteResult.Ok) {
        $remoteResult | Format-List *
        throw ($remoteResult.Reason | ForEach-Object { $_ })
    }

    $localOut = (Resolve-Path -LiteralPath $LocalOutputDir).Path
    $localPng = Join-Path $localOut (Split-Path -Leaf $remoteResult.RemotePng)
    Copy-Item -FromSession $session -LiteralPath $remoteResult.RemotePng -Destination $localPng -Force

    [pscustomobject]@{
        Hostname = $Hostname
        SessionId = $remoteResult.SessionId
        SessionName = $remoteResult.SessionName
        UserName = $remoteResult.UserName
        CaptureMethod = $remoteResult.CaptureMethod
        SampleBlackRatio = $remoteResult.SampleBlackRatio
        LocalPng = $localPng
    }
}
finally {
    if ($null -ne $session) {
        Remove-PSSession $session -ErrorAction SilentlyContinue
    }
}
