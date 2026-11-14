#!/bin/bash
# Run once as root on the build host: sudo bash automation/ci/host-firewall.sh
# CI containers that have network (dependency download) live in the Docker network "staminator-ci".
# They may reach the internet, but not the host, the home network, the VMs or other containers.
# The script installs itself with a systemd unit that puts the rules back after a reboot or a Docker restart.
set -euo pipefail

SUBNET="${CI_SUBNET:-172.31.250.0/24}"
NAME=staminator-ci-firewall
CHAIN=STAMINATOR-CI

apply() {
    # Through the host: home network, VM networks, other containers, published ports.
    iptables -w -N "$CHAIN" 2>/dev/null || iptables -w -F "$CHAIN"
    for net in 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16; do
        iptables -w -A "$CHAIN" -d "$net" -j DROP
    done
    iptables -w -N DOCKER-USER 2>/dev/null || true
    iptables -w -C DOCKER-USER -s "$SUBNET" -j "$CHAIN" 2>/dev/null \
        || iptables -w -I DOCKER-USER -s "$SUBNET" -j "$CHAIN"
    # To services of the host itself: proxy, SSH, VNC, anything else.
    iptables -w -C INPUT -s "$SUBNET" -j DROP 2>/dev/null || iptables -w -I INPUT -s "$SUBNET" -j DROP
}

if [ "${1:-}" != apply ]; then
    install -m 755 "$0" "/usr/local/sbin/$NAME"
    cat > "/etc/systemd/system/$NAME.service" <<EOF
[Unit]
Description=Cut the Staminator CI Docker network off from the host and local networks
After=docker.service
PartOf=docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
Environment=CI_SUBNET=$SUBNET
ExecStart=/usr/local/sbin/$NAME apply

[Install]
WantedBy=docker.service
EOF
    systemctl daemon-reload
    systemctl enable "$NAME.service"
    systemctl restart "$NAME.service"
    echo "Installed and applied: systemctl status $NAME"
else
    apply
fi
