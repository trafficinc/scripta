# Windows PowerShell 5.1+; use a native x86-64 host and default SQLite build.
[CmdletBinding()]
param(
    [string]$Scripta = '',
    [switch]$Build,
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$utf8 = [Text.UTF8Encoding]::new($false, $true)

function Invoke-Captured([string]$Executable, [string[]]$Arguments, [string]$OutputFile, [string]$ErrorFile, [hashtable]$Variables) {
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    # Windows CRT quoting, including quotes and trailing backslashes. Do not
    # use a shell or Start-Process's unquoted ArgumentList string joining.
    $quoted = foreach ($argument in $Arguments) {
        '"' + ($argument -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
    }
    $info.Arguments = $quoted -join ' '
    $info.WorkingDirectory = $repository
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = $utf8
    $info.StandardErrorEncoding = $utf8
    foreach ($key in $Variables.Keys) { $info.EnvironmentVariables[$key] = $Variables[$key] }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $info
    try {
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        # Explicit UTF-8 avoids PowerShell 5.1's UTF-16 redirection and legacy
        # code-page conversions corrupting Unicode JSON or executable paths.
        [IO.File]::WriteAllText($OutputFile, $stdout.Result, $utf8)
        [IO.File]::WriteAllText($ErrorFile, $stderr.Result, $utf8)
        return $process.ExitCode
    } finally { $process.Dispose() }
}

try {
    if ($env:OS -ne 'Windows_NT') { throw 'Use the Linux or macOS launcher on those hosts.' }
    $architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    if ($architecture -ne 'X64') { throw 'This launcher requires a native Windows x86-64 host.' }
    if ($Build -and $Scripta) { throw '-Build and -Scripta are mutually exclusive.' }
    $target = 'x86_64-pc-windows-msvc'
    if (-not $OutputDirectory) {
        $OutputDirectory = Join-Path $repository ('target/platform-tests/windows-' + [Guid]::NewGuid().ToString('N'))
    }
    if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
        $OutputDirectory = Join-Path (Get-Location).ProviderPath $OutputDirectory
    }
    $results = [IO.Path]::GetFullPath($OutputDirectory)
    if (Test-Path -LiteralPath $results) { throw 'Choose a new output directory to preserve earlier evidence.' }
    $null = [IO.Directory]::CreateDirectory($results)
    Write-Host "Results: $results"
    [IO.File]::WriteAllText((Join-Path $results 'host.txt'), ([Environment]::OSVersion.VersionString + "`nOS architecture: $architecture`n"), $utf8)
    if ($Build) {
        Push-Location $repository
        try {
            $compiler = & rustc -vV
            if ($LASTEXITCODE -ne 0 -or -not ($compiler -contains "host: $target")) { throw "Compiler host must match native target $target." }
            [IO.File]::WriteAllText((Join-Path $results 'compiler.txt'), ($compiler -join "`n"), $utf8)
            & cargo build -p scr-cli --release --locked --target $target
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        } finally { Pop-Location }
        $Scripta = Join-Path $repository "target/$target/release/scripta.exe"
    }
    if (-not $Scripta) {
        foreach ($candidate in @("target/$target/release/scripta.exe", "target/$target/dist/scripta.exe", "target/$target/debug/scripta.exe", 'target/release/scripta.exe', 'target/debug/scripta.exe')) {
            $candidate = Join-Path $repository $candidate
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { $Scripta = $candidate; break }
        }
        if (-not $Scripta) {
            $command = Get-Command scripta.exe -CommandType Application -ErrorAction SilentlyContinue
            if ($command) { $Scripta = $command.Source }
        }
    }
    if (-not $Scripta) { throw 'No Scripta executable found. Pass -Scripta PATH or -Build.' }
    if (-not (Test-Path -LiteralPath $Scripta -PathType Leaf)) {
        $command = Get-Command $Scripta -CommandType Application -ErrorAction SilentlyContinue
        if (-not $command) { throw "Executable not found: $Scripta" }
        $Scripta = $command.Source
    }
    $binary = (Resolve-Path -LiteralPath $Scripta).ProviderPath
    Write-Host "Executable: $binary"
    [IO.File]::WriteAllText((Join-Path $results 'executable.txt'), ($binary + "`n"), $utf8)
    [IO.File]::WriteAllText((Join-Path $results 'binary.sha256'), ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() + "`n"), $utf8)
    $variables = @{
        SCRIPTA_PLATFORM_BINARY = $binary
        SCRIPTA_PLATFORM_OS = 'windows'
        SCRIPTA_PLATFORM_ARCH = 'x86_64'
        SCRIPTA_PLATFORM_FIXTURES = (Join-Path $repository 'tests/platform/fixtures')
    }
    $versionCode = Invoke-Captured $binary @('version', '--verbose') (Join-Path $results 'version.txt') (Join-Path $results 'version-stderr.txt') $variables
    if ($versionCode -ne 0) { throw 'Scripta version command failed; see version-stderr.txt.' }
    Write-Host ([IO.File]::ReadAllText((Join-Path $results 'version.txt'), $utf8))
    $report = Join-Path $results 'report.json'
    $testCode = Invoke-Captured $binary @('test', 'tests/platform', '--format', 'json', '--show-output') $report (Join-Path $results 'stderr.txt') $variables
    $summary = Join-Path $results 'summary.txt'
    $summaryCode = Invoke-Captured $binary @('run', 'tests/platform/fixtures/report.scripta', $report) $summary (Join-Path $results 'summary-stderr.txt') $variables
    Write-Host ([IO.File]::ReadAllText($summary, $utf8))
    foreach ($name in @('stderr.txt', 'summary-stderr.txt')) {
        $text = [IO.File]::ReadAllText((Join-Path $results $name), $utf8)
        if ($text) { Write-Host $text }
    }
    Write-Host "Results: $results"
    if ($testCode -ne 0) { exit $testCode }
    exit $summaryCode
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 2
}
