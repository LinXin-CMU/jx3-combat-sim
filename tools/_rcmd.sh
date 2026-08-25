echo "=== frps.toml (cat -A ?? ^M=CR) ==="
sudo cat -A /opt/frp/frps.toml
echo "=== listening ==="
sudo ss -ltnp | grep -E ":7000|:2014" || echo "(2014 ? frpc ??????)"
echo "=== frps status ==="
systemctl is-active frps
