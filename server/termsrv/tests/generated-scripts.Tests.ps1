Describe 'Generated PowerShell syntax' {
    It 'emits parseable PowerShell in <Name>' -ForEach @(
        @{ Name = 'capture-irdp-session-screenshot' }
        @{ Name = 'deploy-testvm-psremoting' }
    ) {
        # oracle: reference PowerShell language parser.
        $termsrvRoot = Split-Path -Parent $PSScriptRoot
        $path = Join-Path $termsrvRoot "crates\ironrdp-termsrv\scripts\$Name.ps1"
        $tokens = $null
        $parseErrors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile(
            $path, [ref]$tokens, [ref]$parseErrors)
        $parseErrors | Should -BeNullOrEmpty

        # Parse the generated script, not just its enclosing string literal.
        $payloads = $ast.FindAll({
            param($node)
            $node -is [System.Management.Automation.Language.StringConstantExpressionAst] -and
                $node.StringConstantType -eq [System.Management.Automation.Language.StringConstantType]::SingleQuotedHereString
        }, $true)
        $payloads | Should -Not -BeNullOrEmpty
        foreach ($payload in $payloads) {
            $null = [System.Management.Automation.Language.Parser]::ParseInput(
                $payload.Value, [ref]$tokens, [ref]$parseErrors)
            $parseErrors | Should -BeNullOrEmpty
        }
    }
}
