[CmdletBinding()]
param(
    [ValidateSet("boot_to_fsp103", "fsp103_to_fsp104")]
    [string]$Segment = "fsp103_to_fsp104",

    [ValidateRange(1, 1000)]
    [int]$Generations = 2,

    [ValidateRange(2, 1000)]
    [int]$PopulationSize = 8,

    [ValidateRange(1, 1000)]
    [int]$Elites = 2,

    [ValidateRange(1, 64)]
    [int]$Workers = 4,

    [ValidateRange(1, 1000)]
    [int]$Repetitions = 2,

    [uint64]$RngSeed = 1,

    [string]$DvdPath,

    [string]$Preset = "windows-clang-debug",

    [string]$OutputRoot,

    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$huntctlManifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"
$populationEvaluator = Join-Path $PSScriptRoot "evaluate-population.ps1"

function Invoke-Checked {
    param(
        [Parameter(Mandatory)]
        [string]$Program,
        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Program $($Arguments -join ' ')"
    }
}

function Invoke-CheckedQuiet {
    param(
        [Parameter(Mandatory)]
        [string]$Program,
        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    & $Program @Arguments | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Program $($Arguments -join ' ')"
    }
}

function Write-Utf8Json([string]$Path, $Value) {
    [System.IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth 12),
        [System.Text.UTF8Encoding]::new($false))
}

if ($Elites -gt $PopulationSize) {
    throw "Elites ($Elites) cannot exceed population size ($PopulationSize)."
}
if (-not (Test-Path -LiteralPath $populationEvaluator -PathType Leaf)) {
    throw "Missing population evaluator: $populationEvaluator"
}

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $OutputRoot = Join-Path $repoRoot "build\search\$Segment\$stamp"
} elseif (-not [System.IO.Path]::IsPathRooted($OutputRoot)) {
    $OutputRoot = Join-Path $repoRoot $OutputRoot
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
New-Item -ItemType Directory -Force $OutputRoot | Out-Null

Push-Location $repoRoot
try {
    if (-not $SkipBuild) {
        Invoke-Checked "cmake" @(
            "--preset", $Preset,
            "-DDUSK_ENABLE_CODE_MODS=OFF"
        )
        Invoke-Checked "cmake" @(
            "--build", "--preset", $Preset,
            "--target", "dusklight", "--", "-j$([Environment]::ProcessorCount)"
        )
    }

    $population = Join-Path $OutputRoot "g000"
    Invoke-CheckedQuiet "cargo" @(
        "run", "--release", "--quiet", "--manifest-path", $huntctlManifest, "--",
        "search", "seed",
        "--segment", $Segment,
        "--output", $population,
        "--size", [string]$PopulationSize,
        "--rng-seed", [string]$RngSeed
    )

    $lastLeaderboard = $null
    $lastManifest = $null
    for ($generation = 0; $generation -lt $Generations; $generation++) {
        $generationName = "g{0:D3}" -f $generation
        $manifestPath = Join-Path $population "manifest.json"
        $resultsPath = Join-Path $population "results.json"
        $evaluationRoot = Join-Path $population "evaluations"

        Write-Host "`nSearch $Segment $generationName/$('g{0:D3}' -f ($Generations - 1))" -ForegroundColor Cyan
        $evaluateArguments = @{
            Population = $manifestPath
            Preset = $Preset
            Workers = $Workers
            Repetitions = $Repetitions
            ArtifactRoot = $evaluationRoot
            ResultsPath = $resultsPath
        }
        if (-not [string]::IsNullOrWhiteSpace($DvdPath)) {
            $evaluateArguments.DvdPath = $DvdPath
        }
        # Evaluator progress uses Write-Host; suppress its full result document.
        & $populationEvaluator @evaluateArguments | Out-Null

        $rankOutput = & cargo run --release --quiet --manifest-path $huntctlManifest -- `
            search rank --population $manifestPath --results $resultsPath
        if ($LASTEXITCODE -ne 0) { throw "huntctl search rank failed for $generationName" }
        $rankText = $rankOutput -join [Environment]::NewLine
        [System.IO.File]::WriteAllText(
            (Join-Path $population "leaderboard.json"),
            $rankText,
            [System.Text.UTF8Encoding]::new($false))
        # Windows PowerShell emits a top-level JSON array as one pipeline item;
        # enumerate it explicitly so [0] is a leaderboard row, not Object[].
        $lastLeaderboard = @(($rankText | ConvertFrom-Json) | ForEach-Object { $_ })
        $lastManifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
        if ($lastLeaderboard.Count -eq 0) {
            throw "Search produced an empty leaderboard for $generationName."
        }

        $winner = $lastLeaderboard[0]
        Write-Host (
            "  winner={0} depth={1} success={2}/{3} median={4}" -f
            $winner.candidate_id.Substring(0, 12),
            $winner.score.milestone_depth,
            $winner.score.successes,
            $winner.score.attempts,
            $winner.score.median_first_hit_tick) -ForegroundColor Green

        if ($generation + 1 -lt $Generations) {
            $nextPopulation = Join-Path $OutputRoot ("g{0:D3}" -f ($generation + 1))
            Invoke-CheckedQuiet "cargo" @(
                "run", "--release", "--quiet", "--manifest-path", $huntctlManifest, "--",
                "search", "evolve",
                "--population", $manifestPath,
                "--results", $resultsPath,
                "--output", $nextPopulation,
                "--size", [string]$PopulationSize,
                "--elites", [string]$Elites,
                "--rng-seed", [string]($RngSeed + [uint64]$generation + 1)
            )
            $population = $nextPopulation
        }
    }

    $champion = $lastLeaderboard[0]
    $championMember = @($lastManifest.members | Where-Object candidate_id -eq $champion.candidate_id)
    if ($championMember.Count -ne 1) {
        throw "Could not resolve final champion in its population manifest."
    }
    $championSource = Join-Path $population ([string]$championMember[0].tape_file)
    $championOutput = if ($Segment -eq "boot_to_fsp103") {
        Join-Path $repoRoot "build\boot-search-champion.tape"
    } else {
        Join-Path $repoRoot "build\route-search-champion.tape"
    }
    Copy-Item -LiteralPath $championSource -Destination $championOutput -Force

    $summary = [ordered]@{
        schema = "dusklight-search-run/v1"
        segment = $Segment
        generations = $Generations
        population_size = $PopulationSize
        repetitions = $Repetitions
        rng_seed = $RngSeed
        champion_id = [string]$champion.candidate_id
        champion_tape = $championOutput
        score = $champion.score
        output_root = $OutputRoot
    }
    Write-Utf8Json (Join-Path $OutputRoot "run.summary.json") $summary

    Write-Host "`nSearch complete." -ForegroundColor Green
    Write-Host "Champion: $championOutput" -ForegroundColor Green
    Write-Host "Artifacts: $OutputRoot" -ForegroundColor DarkGray
    $summary | ConvertTo-Json -Depth 8
} finally {
    Pop-Location
}
