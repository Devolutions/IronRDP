[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateNotNullOrEmpty()]
    [string[]] $SkillDirectory
)

$ErrorActionPreference = 'Stop'
$Tessl = (Get-Command tessl -ErrorAction Stop).Source
$Failed = $false

foreach ($Directory in $SkillDirectory) {
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) {
        Write-Error "Skill directory not found: $Directory" -ErrorAction Continue
        $Failed = $true
        continue
    }

    $ResolvedDirectory = (Resolve-Path -LiteralPath $Directory).Path
    $SkillFile = Join-Path $ResolvedDirectory 'SKILL.md'
    if (-not (Test-Path -LiteralPath $SkillFile -PathType Leaf)) {
        Write-Error "SKILL.md not found in $ResolvedDirectory" -ErrorAction Continue
        $Failed = $true
        continue
    }

    $SkillsRoot = Split-Path $ResolvedDirectory -Parent
    $StagingRoot = Split-Path $SkillsRoot -Parent
    $TemporaryRoot = Join-Path $StagingRoot ".tessl-skill-lint-$([guid]::NewGuid())"
    $StagedDirectory = Join-Path $TemporaryRoot (Split-Path $ResolvedDirectory -Leaf)

    try {
        New-Item -ItemType Directory -Path $StagedDirectory -Force | Out-Null
        Get-ChildItem -LiteralPath $ResolvedDirectory -Force |
            Where-Object Name -ne '.tessl-plugin' |
            Copy-Item -Destination $StagedDirectory -Recurse -Force

        & $Tessl skill import --workspace local $StagedDirectory | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Error "Tessl local import failed for $ResolvedDirectory" -ErrorAction Continue
            $Failed = $true
            continue
        }

        & $Tessl skill lint $StagedDirectory
        if ($LASTEXITCODE -ne 0) {
            $Failed = $true
        }
    } finally {
        if (Test-Path -LiteralPath $TemporaryRoot) {
            Remove-Item -LiteralPath $TemporaryRoot -Recurse -Force
        }
    }
}

if ($Failed) {
    exit 1
}
