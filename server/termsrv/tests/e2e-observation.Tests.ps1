BeforeAll {
    $scripts = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts'
    $path = Join-Path $scripts 'e2e-test-screenshot.ps1'
    $ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$null)
    $statements = $ast.EndBlock.Statements
}

Describe 'Observation setup and resource lifetime' {
    It 'configures a fresh IDD dump destination for each skipped-deployment observation' {
        # oracle: mutant SkipDeploy leaves the driver's dump directory pointing at a prior run.
        $preflight = $statements | Where-Object {
            $_ -is [Management.Automation.Language.IfStatementAst] -and
            $_.Extent.Text.Contains('Checking custom IDD prerequisites')
        }
        $remote = [powershell]::Create()
        try {
            [void]$remote.AddScript(@'
param($Preparation, $Scripts)
$SkipDeploy = [switch]$true
$SkipScreenshot = [switch]$false
$StrictSessionProof = [switch]$true
$Mode = 'Provider'
$Hostname = 'localhost'
$adminCred = $null
$scriptRoot = $Scripts
$iddRuntimeRoot = 'C:\fixture'
$iddRuntimeStateFile = 'C:\fixture\runtime-config.txt'
$iddDebugTraceFile = 'C:\fixture\debug.log'
$preflightIssues = [Collections.Generic.List[string]]::new()
$hardPreflightIssues = [Collections.Generic.List[string]]::new()
$destinations = [Collections.Generic.List[string]]::new()
function New-TestVmSession { 'fixture-session' }
function Remove-PSSession {}
function Invoke-Command {
    param($Session, $ScriptBlock, $ArgumentList)
    $destinations.Add($ArgumentList[3])
    @{ issues = @(); runtime_state = $ArgumentList[3] }
}
foreach ($iddDumpRemoteDir in @('C:\fixture\first-observation', 'C:\fixture\second-observation')) {
    & ([scriptblock]::Create($Preparation))
}
@{ Destinations = @($destinations) }
'@).AddArgument($preflight.Extent.Text).AddArgument($scripts)
            $result = $remote.Invoke()
            $remote.Streams.Error | Should -HaveCount 0
            $result.Destinations | Should -HaveCount 2
            $result.Destinations[0] | Should -Be 'C:\fixture\first-observation'
            $result.Destinations[1] | Should -Be 'C:\fixture\second-observation'
        } finally {
            $remote.Dispose()
        }
    }

    It 'preserves the baseline failure and releases probe resources before log collection starts' {
        # oracle: mutant early failure masks its cause and leaks probe resources under strict mode.
        $initialization = ($statements | Where-Object {
            $_ -is [Management.Automation.Language.AssignmentStatementAst] -and
            $_.Left -is [Management.Automation.Language.VariableExpressionAst] -and
            $_.Left.VariablePath.UserPath -eq 'session'
        }).Extent.Text -join "`n"
        $observation = $statements | Where-Object {
            $_ -is [Management.Automation.Language.TryStatementAst] -and
            $_.Body.Extent.Text.Contains("OperationName 'pre-connect evidence baseline'")
        }
        $cleanup = $observation.Finally.Statements.Extent.Text -join "`n"
        $remote = [powershell]::Create()
        try {
            [void]$remote.AddScript(@'
param($Initialization, $Cleanup)
Set-StrictMode -Version Latest
$events = [Collections.Generic.List[string]]::new()
$iddOpenProbeJob = [pscustomobject]@{ Id = 42 }
$iddOpenProbeSession = [pscustomobject]@{ Id = 43 }
function Wait-Job {}
function Receive-Job {}
function Remove-Job { param($Job) $events.Add("job:$($Job.Id)") }
function Remove-PSSession { param($Session) $events.Add("session:$($Session.Id)") }
try {
    & ([scriptblock]::Create($Initialization + "`ntry { throw 'Baseline failed' } finally {`n" + $Cleanup + "`n}"))
} catch {
    $failure = $_.Exception.Message
}
@{ Failure = $failure; Events = @($events) }
'@).AddArgument($initialization).AddArgument($cleanup)
            $result = $remote.Invoke()
            $remote.Streams.Error | Should -HaveCount 0
            $result.Failure | Should -Be 'Baseline failed'
            ($result.Events -join ',') | Should -Be 'job:42,session:43'
        } finally {
            $remote.Dispose()
        }
    }

    It 'can evaluate optional snapshot evidence when screenshot capture was skipped' {
        # oracle: mutant diagnostic-only collection aborts on a snapshot variable initialized only during capture.
        $initialization = ($statements | Where-Object {
            $_ -is [Management.Automation.Language.AssignmentStatementAst] -and
            $_.Left -is [Management.Automation.Language.VariableExpressionAst] -and
            $_.Left.VariablePath.UserPath -eq 'preConnectSessionSnapshotArtifact'
        } | ForEach-Object { $_.Extent.Text }) -join "`n"
        $guard = $ast.Find({
            param($node)
            $node -is [Management.Automation.Language.IfStatementAst] -and
            $node.Clauses[0].Item1.Extent.Text.Contains('$preConnectSessionSnapshotArtifact')
        }, $true)
        $remote = [powershell]::Create()
        try {
            [void]$remote.AddScript("Set-StrictMode -Version Latest`n" + $initialization + "`n" + $guard.Extent.Text)
            $null = $remote.Invoke()
            $remote.Streams.Error | Should -HaveCount 0
        } finally {
            $remote.Dispose()
        }
    }
}
