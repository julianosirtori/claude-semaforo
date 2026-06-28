# Claude Semáforo hook (Windows native host).
#
# Wire the same script to every event. It reads the hook JSON on stdin and:
#   - PreToolUse  -> POST /permission, then emit the response on stdout so
#                    Claude Code applies the allow/deny decision (long-poll).
#   - everything  -> POST /events (fire and forget).
#     else
#
# Configure with SEMAFORO_TOKEN (required), SEMAFORO_PORT, or SEMAFORO_URL.

$ErrorActionPreference = 'SilentlyContinue'

$payload = [Console]::In.ReadToEnd()

# Token: SEMAFORO_TOKEN env wins, else ~/.claude/semaforo.token (written by the
# app's "Instalar hooks" button), else a placeholder.
$token = $env:SEMAFORO_TOKEN
if (-not $token) {
  $tokenFile = Join-Path $env:USERPROFILE '.claude\semaforo.token'
  if (Test-Path $tokenFile) { $token = (Get-Content $tokenFile -Raw -ErrorAction SilentlyContinue).Trim() }
}
if (-not $token) { $token = 'troque-este-token' }
$port = if ($env:SEMAFORO_PORT) { $env:SEMAFORO_PORT } else { '7337' }

$isPerm  = $payload -match '"hook_event_name"\s*:\s*"PreToolUse"'
$path    = if ($isPerm) { '/permission' } else { '/events' }
$timeout = if ($isPerm) { 610 } else { 3 }

if ($env:SEMAFORO_URL) {
  $bases = @($env:SEMAFORO_URL.TrimEnd('/'))
} else {
  $bases = @("http://host.docker.internal:$port", "http://127.0.0.1:$port")
}

$headers = @{ Authorization = "Bearer $token"; 'X-Semaforo-Container' = '0' }

$content = $null
foreach ($base in $bases) {
  try {
    $resp = Invoke-WebRequest -Method Post -Uri "$base$path" -Headers $headers `
      -ContentType 'application/json' -Body $payload -TimeoutSec $timeout -UseBasicParsing
    $content = $resp.Content
    break
  } catch { }
}

# Claude Code reads our stdout on PreToolUse to get the permission decision.
if ($isPerm -and $content) { Write-Output $content }
exit 0
