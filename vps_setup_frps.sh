#!/usr/bin/env bash
# Install or update frps on a Linux VPS.
#
# Run as root with a newly generated token supplied without shell-history echo:
#   read -rsp 'New frp token: ' FRP_AUTH_TOKEN && echo
#   export FRP_AUTH_TOKEN
#   bash vps_setup_frps.sh
#   unset FRP_AUTH_TOKEN
#
# A secret manager or protected environment file is preferable for repeated
# deployments. Put an HTTPS reverse proxy in front of the service port before
# sharing a public URL.

set -euo pipefail

: "${FRP_AUTH_TOKEN:?Set FRP_AUTH_TOKEN to a new high-entropy value}"

FRP_VERSION="${FRP_VERSION:-0.69.1}"
BIND_PORT="${FRP_BIND_PORT:-7000}"
SERVICE_PORT="${FRP_SERVICE_PORT:-2014}"

ARCH="amd64"
case "$(uname -m)" in
  aarch64|arm64) ARCH="arm64" ;;
esac
PKG="frp_${FRP_VERSION}_linux_${ARCH}"

cd /opt
echo "[*] Downloading ${PKG} ..."
URLS=(
  "https://github.com/fatedier/frp/releases/download/v${FRP_VERSION}/${PKG}.tar.gz"
  "https://gh-proxy.com/https://github.com/fatedier/frp/releases/download/v${FRP_VERSION}/${PKG}.tar.gz"
  "https://ghproxy.net/https://github.com/fatedier/frp/releases/download/v${FRP_VERSION}/${PKG}.tar.gz"
)

downloaded=0
for url in "${URLS[@]}"; do
  echo "    trying ${url}"
  if curl -fSL --connect-timeout 15 -o frp.tar.gz "${url}"; then
    downloaded=1
    break
  fi
done
if [[ "${downloaded}" != "1" ]]; then
  echo '[X] Unable to download frp.' >&2
  exit 1
fi

tar xzf frp.tar.gz
if [[ -d /opt/frp ]]; then
  mv /opt/frp "/opt/frp.backup.$(date +%Y%m%d%H%M%S)"
fi
mv "${PKG}" /opt/frp
rm -f /opt/frp.tar.gz

install -m 600 /dev/null /opt/frp/frps.toml
cat > /opt/frp/frps.toml <<EOF
bindPort = ${BIND_PORT}
auth.token = "${FRP_AUTH_TOKEN}"
transport.tls.force = true
transport.tcpMux = false
EOF

cat > /etc/systemd/system/frps.service <<'EOF'
[Unit]
Description=frps relay for the JX3 combat simulator
After=network.target

[Service]
ExecStart=/opt/frp/frps -c /opt/frp/frps.toml
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now frps
sleep 1
systemctl --no-pager status frps | head -n 8

if command -v ufw >/dev/null 2>&1; then
  ufw allow "${BIND_PORT}/tcp" || true
  ufw allow "${SERVICE_PORT}/tcp" || true
fi
if command -v firewall-cmd >/dev/null 2>&1; then
  firewall-cmd --permanent --add-port="${BIND_PORT}/tcp" || true
  firewall-cmd --permanent --add-port="${SERVICE_PORT}/tcp" || true
  firewall-cmd --reload || true
fi

echo '[OK] frps is running.'
echo "     Control port: ${BIND_PORT}"
echo "     Origin service port: ${SERVICE_PORT}"
echo '     Configure the cloud security group and an HTTPS reverse proxy separately.'
