#!/usr/bin/env bash
# Uploaded files live in a private staging directory. No secrets printed.
set -euo pipefail
[[ $(id -u) == 0 ]] || exit 2
STAGING=/home/ubuntu/jx3-public-stage
[[ -f "$STAGING/frps.toml" && -f "$STAGING/nginx.public.conf" ]] || exit 2
[[ ! -e /opt/frp/frps.pre-public-20260906.toml ]] || { echo 'Already prepared; inspect before rerunning.'; exit 3; }
install -d -m 700 /opt/frp/tls
openssl req -x509 -newkey rsa:3072 -nodes -sha256 -days 825 \
  -subj '/CN=jx3-frp.internal' -addext 'subjectAltName=DNS:jx3-frp.internal' \
  -addext 'basicConstraints=critical,CA:TRUE' \
  -keyout /opt/frp/tls/jx3.key -out /opt/frp/tls/jx3.crt 2>/dev/null
chmod 600 /opt/frp/tls/jx3.key
install -m 644 /opt/frp/tls/jx3.crt "$STAGING/frp-trust.crt"
/opt/frp/frps verify -c "$STAGING/frps.toml"
install -m 600 /opt/frp/frps.toml /opt/frp/frps.pre-public-20260906.toml
install -m 600 "$STAGING/frps.toml" /opt/frp/frps.toml
systemctl restart frps
systemctl is-active --quiet frps
# Keep Nginx in maintenance until local authentication and the new tunnel pass checks.
echo 'New frp credentials and certificate installed; tunnel binds loopback only. HTTPS remains in maintenance.'
