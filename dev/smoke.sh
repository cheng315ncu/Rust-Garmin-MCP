#!/usr/bin/env bash
# Smoke test: send MCP initialize + tools/list + tools/call for all new tools.
# Credentials: either put GARMIN_EMAIL=… and GARMIN_PASSWORD=… in
# garmin_mcp_rust/.env (auto-loaded by the binary via dotenvy), or pass
# them inline:  GARMIN_EMAIL=… GARMIN_PASSWORD=… bash dev/smoke.sh
# Outputs: one JSON-RPC response per line on stdout; server stderr in /tmp/garmin-mcp.err

set -u

DATE="${DATE:-2026-04-24}"   # yesterday — today's data often not fully processed yet
cd "$(dirname "$0")/.."

# Build first so cargo run doesn't emit compile noise on stderr during the test.
cargo build --quiet 2>&1

req() {
  local id="$1" method="$2" params="${3:-null}"
  if [[ "$params" == "null" ]]; then
    printf '{"jsonrpc":"2.0","id":%s,"method":"%s"}\n' "$id" "$method"
  else
    printf '{"jsonrpc":"2.0","id":%s,"method":"%s","params":%s}\n' "$id" "$method" "$params"
  fi
}

notif() {
  printf '{"jsonrpc":"2.0","method":"%s"}\n' "$1"
}

call() {
  local id="$1" name="$2" args="$3"
  req "$id" "tools/call" "$(printf '{"name":"%s","arguments":%s}' "$name" "$args")"
}

{
  req 1 "initialize" '{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}'
  notif "notifications/initialized"
  req 2 "tools/list"
  call 10 "get_recent_activities" '{"limit":3}'
  call 11 "get_sleep_summary" "{\"date\":\"$DATE\"}"
  call 12 "get_daily_heart_rate" "{\"date\":\"$DATE\"}"
  call 13 "get_stress_summary" "{\"date\":\"$DATE\"}"
  call 14 "get_body_battery_summary" "{\"date\":\"$DATE\"}"
  call 15 "get_training_status" "{\"date\":\"$DATE\"}"
} | ./target/debug/garmin-mcp 2>/tmp/garmin-mcp.err
