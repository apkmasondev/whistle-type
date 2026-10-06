<#
.SYNOPSIS
  Generates the speech test set with the Polish voices built into Windows (no network), then composes the
  derived cases (lengths 1-90 s, silence, noise, noisy speech, clicks) with scripts/make-test-set.py.
  Output: tests/audio/generated/  (git-ignored)

  Uses Windows PowerShell 5.1 for the WinRT OneCore voices ("Microsoft Paulina", "Microsoft Adam") and
  System.Speech for the legacy desktop voice.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'tests\audio\generated\tts'
New-Item -ItemType Directory -Force $out | Out-Null

$sentences = [ordered]@{
    'pl_basic'      = 'To jest test rozpoznawania mowy.'
    'pl_tech'       = 'Uruchom Gradle i sprawdź aplikację Kotlin Compose.'
    'pl_mixed'      = 'Sprawdź komponent React i popraw useEffect.'
    'pl_mixed_long' = 'Sprawdź komponent React i zobacz, czy useEffect nie powoduje ponownego renderowania.'
    'pl_gradle'     = 'Uruchom Gradle i sprawdź build release.'
    'pl_diacritics' = 'Zażółć gęślą jaźń. Źdźbło trawy, łódź i chrząszcz brzmią w trzcinie.'
    'pl_claude'     = 'Otwórz Claude Code i poproś Codex o przegląd kodu w TypeScript.'
    'pl_web'        = 'Zbuduj projekt w Vite, potem sprawdź shader WebGL i model Three.js w Blenderze.'
    'pl_github'     = 'Wypchnij zmiany na GitHub i opublikuj stronę na GitHub Pages.'
    'pl_story'      = 'Wczoraj wieczorem pracowałem nad nową wersją aplikacji. Najpierw poprawiłem błędy w interfejsie, potem dodałem testy, a na końcu przygotowałem notatki do wydania. Dzisiaj chcę jeszcze sprawdzić wydajność i zużycie pamięci na starszym laptopie.'
}

$worker = @'
param($out, $json)
$ErrorActionPreference = 'Stop'
$sentences = $json | ConvertFrom-Json
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.DataReader, Windows.Storage.Streams, ContentType = WindowsRuntime]
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, [Type]$t) { $task = $asTask.MakeGenericMethod($t).Invoke($null, @($op)); $task.Wait(-1) | Out-Null; $task.Result }
$synth = New-Object Windows.Media.SpeechSynthesis.SpeechSynthesizer
$voices = [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices | Where-Object { $_.Language -eq 'pl-PL' }
foreach ($v in $voices) {
    $synth.Voice = $v
    $tag = ($v.DisplayName -replace 'Microsoft ', '').ToLower()
    foreach ($rate in @(@('normal', 1.0), @('fast', 1.6), @('slow', 0.7))) {
        $synth.Options.SpeakingRate = $rate[1]
        foreach ($p in $sentences.PSObject.Properties) {
            $stream = Await ($synth.SynthesizeTextToStreamAsync($p.Value)) ([Windows.Media.SpeechSynthesis.SpeechSynthesisStream])
            $size = [uint32]$stream.Size
            $reader = New-Object Windows.Storage.Streams.DataReader($stream.GetInputStreamAt(0))
            $null = Await ($reader.LoadAsync($size)) ([uint32])
            $bytes = New-Object byte[] $size
            $reader.ReadBytes($bytes)
            [IO.File]::WriteAllBytes((Join-Path $out ("{0}__{1}_{2}.wav" -f $p.Name, $tag, $rate[0])), $bytes)
        }
    }
}
'@
$workerPath = Join-Path $env:TEMP 'wt-tts-worker.ps1'
Set-Content -Path $workerPath -Value $worker -Encoding UTF8
$json = $sentences | ConvertTo-Json -Compress
& "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File $workerPath $out $json
Remove-Item $workerPath -Force

# Legacy SAPI desktop voice (different timbre)
Add-Type -AssemblyName System.Speech
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$desk = (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -eq 'pl-PL' } | Select-Object -First 1
if ($desk) {
    foreach ($k in $sentences.Keys) {
        $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
        $s.SelectVoice($desk.VoiceInfo.Name)
        $s.SetOutputToWaveFile((Join-Path $out "${k}__desktop_normal.wav"), $fmt)
        $s.Speak($sentences[$k]); $s.Dispose()
    }
}
Get-ChildItem $out | Measure-Object | ForEach-Object { "TTS clips: $($_.Count)" }
python (Join-Path $PSScriptRoot 'make-test-set.py')
