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
        $timestampAssignment = ($statements | Where-Object {
            $_ -is [Management.Automation.Language.AssignmentStatementAst] -and
            $_.Left -is [Management.Automation.Language.VariableExpressionAst] -and
            $_.Left.VariablePath.UserPath -eq 'timestamp'
        }).Extent.Text
        $dumpDirectoryAssignment = ($statements | Where-Object {
            $_ -is [Management.Automation.Language.AssignmentStatementAst] -and
            $_.Left -is [Management.Automation.Language.VariableExpressionAst] -and
            $_.Left.VariablePath.UserPath -eq 'iddDumpRemoteDir'
        }).Extent.Text
        $preflight.Extent.Text | Should -Match '-ArgumentList\s+\$iddRuntimeRoot,\s*\$iddRuntimeStateFile,\s*\$iddDebugTraceFile,\s*\$iddDumpRemoteDir'
        $remote = [powershell]::Create()
        try {
            [void]$remote.AddScript(@'
param($TimestampAssignment, $DumpDirectoryAssignment)
$iddRuntimeRoot = 'C:\fixture'
$destinations = [Collections.Generic.List[string]]::new()
foreach ($iteration in 1..2) {
    Invoke-Expression $TimestampAssignment
    Invoke-Expression $DumpDirectoryAssignment
    $destinations.Add($iddDumpRemoteDir)
}
@{ Destinations = @($destinations) }
'@).AddArgument($timestampAssignment).AddArgument($dumpDirectoryAssignment)
            $result = $remote.Invoke()
            $remote.Streams.Error | Should -HaveCount 0
            $result.Destinations | Should -HaveCount 2
            $result.Destinations[0] | Should -Match '^C:\\fixture\\bitmap-dumps-'
            $result.Destinations[1] | Should -Match '^C:\\fixture\\bitmap-dumps-'
            $result.Destinations[0] | Should -Not -Be $result.Destinations[1]
        } finally {
            $remote.Dispose()
        }
    }

    It 'releases probe resources when the observation fails' {
        # oracle: mutant observation failure skips probe-resource cleanup.
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
@{ Events = @($events) }
'@).AddArgument($initialization).AddArgument($cleanup)
            $result = $remote.Invoke()
            $remote.Streams.Error | Should -HaveCount 0
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
