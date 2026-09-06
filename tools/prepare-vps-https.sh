#!/usr/bin/env bash
# Prepare an HTTPS maintenance endpoint. Does not expose the local application.
# Run as root with the verified public IP as the only argument.
set -euo pipefail
PUBLIC_IP="${1:?verified public IPv4 required}"
[[ "$PUBLIC_IP" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 2
[[ $(id -u) == 0 ]] || exit 2
if [[ -e /etc/nginx/sites-available/jx3-public ]]; then
  echo 'Existing jx3-public config found; inspect before changing it.' >&2
  exit 3
fi
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq nginx python3-venv
python3 -m venv /opt/jx3-certbot
/opt/jx3-certbot/bin/pip install --disable-pip-version-check 'certbot==5.4.0'
install -d -m 755 /var/lib/jx3-acme
cat > /etc/nginx/sites-available/jx3-acme <<'NGINX'
server {
    listen 80 default_server;
    server_name _;
    location /.well-known/acme-challenge/ { root /var/lib/jx3-acme; }
    location / { return 503; }
}
NGINX
if [[ -L /etc/nginx/sites-enabled/default ]]; then
  mv /etc/nginx/sites-enabled/default /etc/nginx/sites-available/jx3-default-link.backup
fi
ln -s /etc/nginx/sites-available/jx3-acme /etc/nginx/sites-enabled/jx3-acme
nginx -t
systemctl enable --now nginx
systemctl reload nginx
/opt/jx3-certbot/bin/certbot certonly --non-interactive --agree-tos \
    --register-unsafely-without-email --preferred-profile shortlived \
    --webroot -w /var/lib/jx3-acme --ip-address "$PUBLIC_IP" \
    --cert-name jx3-public --deploy-hook 'systemctl reload nginx'
cat > /etc/nginx/sites-available/jx3-public <<'NGINX'
server {
    listen 443 ssl;
    listen 2014 ssl;
    server_name _;
    ssl_certificate /etc/letsencrypt/live/jx3-public/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/jx3-public/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    server_tokens off;
    location / { return 503; }
}
NGINX
ln -s /etc/nginx/sites-available/jx3-public /etc/nginx/sites-enabled/jx3-public
cat > /etc/systemd/system/jx3-certbot-renew.service <<'UNIT'
[Unit]
Description=Renew JX3 public IP HTTPS certificate
[Service]
Type=oneshot
ExecStart=/opt/jx3-certbot/bin/certbot renew --quiet --deploy-hook "systemctl reload nginx"
UNIT
cat > /etc/systemd/system/jx3-certbot-renew.timer <<'UNIT'
[Unit]
Description=Check short-lived JX3 certificate renewal twice daily
[Timer]
OnCalendar=*-*-* 00,12:00:00
RandomizedDelaySec=1800
Persistent=true
[Install]
WantedBy=timers.target
UNIT
nginx -t
systemctl daemon-reload
systemctl enable --now jx3-certbot-renew.timer
systemctl reload nginx
echo 'HTTPS maintenance endpoint ready; application remains unpublished.'
