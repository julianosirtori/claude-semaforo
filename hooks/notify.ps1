# Claude Semáforo hook (Windows native host).
#
# Wire the same script to every lifecycle event (UserPromptSubmit, Notification,
# PostToolUse, Stop, SessionEnd). It reads the hook JSON on stdin and POSTs it to
# /events — status only, fire and forget. The widget never answers permissions.
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

if ($env:SEMAFORO_URL) {
  $bases = @($env:SEMAFORO_URL.TrimEnd('/'))
} else {
  $bases = @("http://host.docker.internal:$port", "http://127.0.0.1:$port")
}

$headers = @{ Authorization = "Bearer $token"; 'X-Semaforo-Container' = '0' }

foreach ($base in $bases) {
  try {
    Invoke-WebRequest -Method Post -Uri "$base/events" -Headers $headers `
      -ContentType 'application/json' -Body $payload -TimeoutSec 3 -UseBasicParsing | Out-Null
    break
  } catch { }
}
exit 0
