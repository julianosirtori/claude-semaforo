#!/usr/bin/env bash
# Claude Semáforo hook (Linux / inside a devcontainer).
#
# Wire the same script to every event. It reads the hook JSON on stdin and:
#   - PreToolUse  -> POST /permission, then print the response on stdout so
#                    Claude Code applies the allow/deny decision (long-poll).
#   - everything  -> POST /events (fire and forget: UserPromptSubmit,
#     else          Notification, Stop, SessionEnd).
#
# Reaches the host at host.docker.internal first, then 127.0.0.1.
# Configure with SEMAFORO_TOKEN (required), SEMAFORO_PORT, or SEMAFORO_URL.

set -uo pipefail

# Token: SEMAFORO_TOKEN env wins, else ~/.claude/semaforo.token (written by the
# app's "Instalar hooks" button), else a placeholder.
TOKEN="${SEMAFORO_TOKEN:-}"
if [ -z "$TOKEN" ] && [ -f "$HOME/.claude/semaforo.token" ]; then
  TOKEN="$(tr -d '\r\n' < "$HOME/.claude/semaforo.token" 2>/dev/null)"
fi
TOKEN="${TOKEN:-troque-este-token}"
PORT="${SEMAFORO_PORT:-7337}"
payload="$(cat)"

# Tag the session as containerized so the widget shows the `container` badge.
container_flag="0"
if [ -f /.dockerenv ] || grep -qaE '(docker|containerd|kubepods)' /proc/1/cgroup 2>/dev/null; then
  container_flag="1"
fi

case "$payload" in
  *'"hook_event_name":"PreToolUse"'* | *'"hook_event_name": "PreToolUse"'*)
    path="/permission"; timeout=610 ;;
  *)
    path="/events"; timeout=3 ;;
esac

if [ -n "${SEMAFORO_URL:-}" ]; then
  bases="${SEMAFORO_URL%/}"
else
  bases="http://host.docker.internal:${PORT} http://127.0.0.1:${PORT}"
fi

resp=""
for base in $bases; do
  if resp="$(curl -fsS --connect-timeout 2 -m "$timeout" -X POST "${base}${path}" \
      -H "Authorization: Bearer ${TOKEN}" \
      -H "Content-Type: application/json" \
      -H "X-Semaforo-Container: ${container_flag}" \
      --data-binary "$payload" 2>/dev/null)"; then
    break
  fi
done

# Claude Code reads our stdout on PreToolUse to get the permission decision.
if [ "$path" = "/permission" ] && [ -n "$resp" ]; then
  printf '%s' "$resp"
fi
exit 0
