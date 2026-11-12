function Invoke-TermSrvDiagnosticBundle {
    param(
        [string]$OutputDirectory,
        [string]$ComputerName,
        [System.Collections.IDictionary]$Collectors
    )

    New-Item -ItemType Directory -Path $OutputDirectory -ErrorAction Stop | Out-Null
    $manifest = [ordered]@{
        ComputerName = $ComputerName
        CreatedUtc = [datetime]::UtcNow.ToString('o')
        Status = 'complete'
        Sections = @()
    }
    foreach ($name in $Collectors.Keys) {
        $directory = Join-Path $OutputDirectory $name
        New-Item -ItemType Directory -Path $directory -ErrorAction Stop | Out-Null
        $section = [ordered]@{ Name = $name; Status = 'ok'; Artifacts = @(); Warnings = @(); Error = $null }
        try {
            $result = & $Collectors[$name] $directory
            $section.Warnings = @($result.Warnings)
            $files = @(Get-ChildItem -LiteralPath $directory -File -Recurse -ErrorAction Stop)
            if ($files.Count -eq 0) { throw 'Collector produced no artifacts' }
            $section.Artifacts = @($files | ForEach-Object { $_.FullName.Substring($OutputDirectory.Length + 1) })
            if ($section.Warnings.Count -gt 0) { $section.Status = 'warning' }
        }
        catch {
            $section.Status = 'error'
            $section.Error = $_.Exception.Message
        }
        if ($section.Status -ne 'ok') {
            $manifest.Status = 'partial'
            Write-Warning "$name diagnostics: $($section.Error) $($section.Warnings -join '; ')"
        }
        $manifest.Sections += [pscustomobject]$section
    }
    $manifestPath = Join-Path $OutputDirectory 'manifest.json'
    $manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    [pscustomobject]@{
        Status = $manifest.Status
        ManifestPath = $manifestPath
        OutputDirectory = $OutputDirectory
    }
}
