param(
    [string]$Folder = "C:\Users\Nathan.Peter\jp-immersion\media\example-japanese-stories",
    [int]$Workers = 2,
    [int]$ThreadsPerWorker = 4,
    [switch]$Force
)

$ErrorActionPreference = "Continue"
$root = "C:\Users\Nathan.Peter\jp-immersion"
$exe = "$root\tools\whisper\whisper-cli.exe"
$model = "$root\tools\whisper\ggml-small-q5_1.bin"
$logFile = "$root\data\whisper-batch.log"
New-Item -ItemType Directory -Force -Path "$root\data" | Out-Null

function Write-Log([string]$msg) {
    $line = "[$(Get-Date -Format 'HH:mm:ss')] $msg"
    Add-Content -Path $logFile -Value $line -Encoding UTF8
    Write-Output $line
}

# Re-do tracks 02 and 03 with -ml 42 if they were generated earlier without -ml 42
$mp3s = Get-ChildItem -Path $Folder -Filter "*.mp3" | Sort-Object Name
$todo = @()
foreach ($f in $mp3s) {
    $srt = [System.IO.Path]::ChangeExtension($f.FullName, ".srt")
    if ($Force -or -not (Test-Path $srt)) {
        $todo += $f.FullName
    }
}

Write-Log "Starting batch Whisper sync: $($todo.Count) tracks queued ($Workers workers x $ThreadsPerWorker threads, BelowNormal priority)"
if ($todo.Count -eq 0) {
    Write-Log "All tracks already have synced .srt subtitles!"
    exit 0
}

$pool = [RunspaceFactory]::CreateRunspacePool(1, $Workers)
$pool.Open()
$jobs = @()

foreach ($mp3 in $todo) {
    $ps = [PowerShell]::Create()
    $ps.RunspacePool = $pool
    $null = $ps.AddScript({
        param($exe, $model, $mp3, $threads)
        $name = [System.IO.Path]::GetFileName($mp3)
        $outBase = [System.IO.Path]::ChangeExtension($mp3, $null).TrimEnd('.')
        $srt = "$outBase.srt"
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        try {
            $psi = New-Object System.Diagnostics.ProcessStartInfo
            $psi.FileName = $exe
            $psi.Arguments = "-m `"$model`" -l ja -t $threads -ml 42 -osrt -of `"$outBase`" `"$mp3`""
            $psi.UseShellExecute = $false
            $psi.CreateNoWindow = $true
            $psi.RedirectStandardOutput = $true
            $psi.RedirectStandardError = $true
            $proc = [System.Diagnostics.Process]::Start($psi)
            try { $proc.PriorityClass = [System.Diagnostics.ProcessPriorityClass]::BelowNormal } catch {}
            $null = $proc.StandardOutput.ReadToEndAsync()
            $null = $proc.StandardError.ReadToEndAsync()
            $proc.WaitForExit()
            $sw.Stop()
            if ($proc.ExitCode -eq 0 -and (Test-Path $srt)) {
                try { Invoke-RestMethod -Uri "http://127.0.0.1:41937/api/rescan" -Method Post -TimeoutSec 5 | Out-Null } catch {}
                return "OK ($($([Math]::Round($sw.Elapsed.TotalSeconds, 1)))s): $name"
            } else {
                return "ERR (exit $($proc.ExitCode)): $name"
            }
        } catch {
            return "ERR: $name -> $($_.Exception.Message)"
        }
    }).AddArgument($exe).AddArgument($model).AddArgument($mp3).AddArgument($ThreadsPerWorker)
    $jobs += [PSCustomObject]@{ PS = $ps; Handle = $ps.BeginInvoke(); Name = [System.IO.Path]::GetFileName($mp3) }
}

$done = 0; $ok = 0; $err = 0
foreach ($j in $jobs) {
    $res = $j.PS.EndInvoke($j.Handle)
    $j.PS.Dispose()
    $done++
    if ($res -like "OK*") { $ok++ } else { $err++ }
    Write-Log "[$done/$($todo.Count)] $res"
}

$pool.Close(); $pool.Dispose()
try { Invoke-RestMethod -Uri "http://127.0.0.1:41937/api/rescan" -Method Post -TimeoutSec 5 | Out-Null } catch {}
Write-Log "Batch complete: $ok synced, $err failed."
