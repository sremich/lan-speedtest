#!/usr/bin/env bash
# Install and configure coturn on the speed test guest.
#
# Idempotent: safe to re-run. Reads credentials from the environment (normally
# sourced from the deploy .env) and renders turnserver.conf.template into
# /etc/turnserver.conf. No credential is ever written to the repo.
#
# Required environment:
#   TURN_USER   TURN username (must match SPEEDTEST_TURN_USER)
#   TURN_PASS   TURN password (must match SPEEDTEST_TURN_PASS)
#   TURN_REALM  realm, e.g. the guest's FQDN
#   LISTEN_IP   the guest's LAN address to bind and relay on
set -euo pipefail

TEMPLATE="$(dirname "$(readlink -f "$0")")/turnserver.conf.template"
DEST=/etc/turnserver.conf
DEFAULTS=/etc/default/coturn

[[ $EUID -eq 0 ]] || { echo "run as root" >&2; exit 1; }

for var in TURN_USER TURN_PASS TURN_REALM LISTEN_IP; do
  if [[ -z "${!var:-}" ]]; then
    echo "ERROR: $var is not set (source your .env first)" >&2
    exit 1
  fi
done

# The relay range in the template. Kept in one place so the firewall rule and
# the config cannot drift apart.
MIN_PORT=$(grep -oP '(?<=^min-port=)\d+' "$TEMPLATE")
MAX_PORT=$(grep -oP '(?<=^max-port=)\d+' "$TEMPLATE")

echo "==> installing coturn"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq coturn >/dev/null

echo "==> rendering $DEST"
umask 077
tmp=$(mktemp)
# The password can contain characters that are special to sed, so substitute
# with awk against fixed strings instead.
TURN_USER="$TURN_USER" TURN_PASS="$TURN_PASS" TURN_REALM="$TURN_REALM" \
LISTEN_IP="$LISTEN_IP" awk '
  { line = $0
    gsub(/@@TURN_USER@@/,  ENVIRON["TURN_USER"],  line)
    gsub(/@@TURN_PASS@@/,  ENVIRON["TURN_PASS"],  line)
    gsub(/@@TURN_REALM@@/, ENVIRON["TURN_REALM"], line)
    gsub(/@@LISTEN_IP@@/,  ENVIRON["LISTEN_IP"],  line)
    print line }
' "$TEMPLATE" > "$tmp"

# Only effective configuration matters. The template's own header explains the
# @@PLACEHOLDER@@ convention and would otherwise trip this check on every run.
# Report line numbers, not content: the rendered file contains the credential.
unsubstituted=$(grep -vE '^[[:space:]]*(#|$)' "$tmp" | grep -c '@@' || true)
if [ "$unsubstituted" -gt 0 ]; then
  echo "ERROR: $unsubstituted setting(s) still contain a placeholder, at line(s):" >&2
  grep -nE '^[[:space:]]*[^#[:space:]].*@@' "$tmp" | cut -d: -f1 | tr '\n' ' ' >&2
  echo >&2
  rm -f "$tmp"
  exit 1
fi

# Only touch the service if something actually changed — that is what makes
# a re-run a no-op rather than a restart.
if [[ -f "$DEST" ]] && cmp -s "$tmp" "$DEST"; then
  echo "    $DEST already current"
  rm -f "$tmp"
  CHANGED=0
else
  install -m 600 -o root -g root "$tmp" "$DEST"
  rm -f "$tmp"
  CHANGED=1
fi

# Debian ships coturn disabled until this is set.
if ! grep -q '^TURNSERVER_ENABLED=1' "$DEFAULTS" 2>/dev/null; then
  echo "==> enabling coturn in $DEFAULTS"
  sed -i '/^#\?TURNSERVER_ENABLED=/d' "$DEFAULTS" 2>/dev/null || true
  echo 'TURNSERVER_ENABLED=1' >> "$DEFAULTS"
  CHANGED=1
fi

systemctl enable coturn >/dev/null 2>&1 || true

if [[ "$CHANGED" -eq 1 ]]; then
  echo "==> restarting coturn"
  systemctl restart coturn
else
  systemctl is-active --quiet coturn || systemctl start coturn
fi

echo "==> verifying"
systemctl is-active --quiet coturn || { echo "coturn is not running" >&2; exit 1; }

# Bind to the configured address specifically. Listening on *some* address is
# not the same as listening where clients will look.
if ! ss -lnup | grep -q "${LISTEN_IP}:3478"; then
  echo "ERROR: nothing is listening on ${LISTEN_IP}:3478" >&2
  ss -lnup | grep 3478 | head -3 >&2
  exit 1
fi

# Logging that goes nowhere is worse than none, because it is discovered while
# debugging something else. Prove the relay is actually writing somewhere.
if ! journalctl -u coturn --since "-2 min" --no-pager 2>/dev/null | grep -q .; then
  echo "WARNING: coturn has logged nothing to journald — check log-file in the config" >&2
fi

cat <<SUMMARY

coturn is up.
  listening : ${LISTEN_IP}:3478/udp
  relay     : ${MIN_PORT}-${MAX_PORT}/udp
  realm     : ${TURN_REALM}
  user      : ${TURN_USER}

Confirm a relay candidate appears at:
  https://webrtc.github.io/samples/src/content/peerconnection/trickle-ice/
  turn:${LISTEN_IP}:3478?transport=udp
SUMMARY
