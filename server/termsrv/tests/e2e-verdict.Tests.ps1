BeforeAll {
    $verdictScript = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\assert-e2e-session-proof.ps1'
    . (Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\e2e-test-screenshot.helpers.ps1')
}

Describe 'E2E verdict' {
    BeforeEach {
        $evidence = @{
            Mode = 'Standalone'
            preflightBlockedConnectionAttempt = $false
            hardPreflightIssues = @()
            screenshotExists = $true
            screenshotFileInfo = @{ Length = 2000 }
            analysisSucceeded = $true
            hasMeaningfulContent = $true
            remoteLogCollectionSucceeded = $true
        }
    }

    It 'rejects missing remote evidence despite a good screenshot' {
        # oracle: server-side evidence is mandatory and a client screenshot is not equivalent.
        $evidence.remoteLogCollectionSucceeded = $false
        { & $verdictScript -Evidence $evidence } | Should -Throw '*strict session proof failed*'
    }

    It 'rejects a blank frame without strict mode' {
        # oracle: blank or uniform images are failed graphics observations.
        $evidence.hasMeaningfulContent = $false
        { Assert-E2EOutput -Evidence $evidence -SkipScreenshot $false } | Should -Throw '*blank or uniform*'
    }

    It 'allows explicitly skipped screenshot collection without requiring an image' {
        # oracle: diagnostic-only collection must remain independently callable.
        $evidence.screenshotExists = $false
        { Assert-E2EOutput -Evidence $evidence -SkipScreenshot $true } | Should -Not -Throw
    }

    It 'accepts meaningful standalone output and successful evidence collection' {
        # oracle: standalone success requires current graphics and server evidence, not provider-only callbacks.
        { & $verdictScript -Evidence $evidence } | Should -Not -Throw
    }

    It 'does not accept otherwise complete provider proof without current-connection correlation' {
        # oracle: mutant strict proof combines unrelated logon and graphics signals.
        $provider = @{
            Mode = 'Provider'; evidenceCorrelated = $true; evidenceCorrelationReason = 'fixture'
            RdpUsername = 'fixture'; securityLogonType10Count = 1; termsrvFallbackMarkerCount = 0
            providerSessionProofMarkerCount = 1; notifyCommandProcessCreatedCount = 1; termsrvSessionProofMarkerCount = 1
            guiTargetSessionResolved = $true; guiTargetSessionProcessProof = $true; guiTargetSessionId = 4
            providerHardwareIdCustom = $true; providerGetHardwareIdCount = 1; providerOnDriverLoadCount = 1
            providerCustomVideoSourceSelected = $true; providerVideoHandleDriverLoadCount = 1
            providerManualVideoSourceSelected = $false; providerFallbackVideoSourceSelected = $false; providerAssumePresentCount = 0
            iddMonitorArrivalMarkerCount = 1; iddDisplayConfigUpdateRequestCount = 1; iddDisplayConfigUpdateSuccessCount = 1
            iddCommitModesMarkerCount = 1; iddCommitModesActivePathCount = 1; iddSwapchainAssignedCount = 1
            iddSessionReadyMarkerCount = 1; iddFirstFrameMarkerCount = 1; iddDumpCount = 1
            iddTargetSessionHasGraphics = $true; type10IddGraphicsSessionConfirmed = $true
            providerVideoHandleMarkerCount = 1; termsrvIddMarkerCount = 1; providerLogonPolicyMarkerCount = 1
            iddDriverLoadedNotified = $true; iddWddmEnabledSignalCount = 1
        }
        foreach ($key in $provider.Keys) { $evidence[$key] = $provider[$key] }
        { & $verdictScript -Evidence $evidence } | Should -Not -Throw
        $evidence.evidenceCorrelated = $false
        { & $verdictScript -Evidence $evidence } | Should -Throw '*connection evidence is not correlated*'
    }
}
