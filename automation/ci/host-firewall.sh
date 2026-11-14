#!/bin/bash
# Run once as root on the build host: sudo bash automation/ci/host-firewall.sh
# CI containers that have network (dependency download) live in the Docker network "staminator-ci".
# They may reach the internet, but not the host, the home network, the VMs or other containers.
set -euo pipefail

SUBNET="${CI_SUBNET:-172.31.250.0/24}"
RULES=/etc/ufw/after.rules
MARK="staminator-ci"

# Through the host: home network, VM networks, other containers, published ports.
# Docker leaves the DOCKER-USER chain alone; ufw restores this block on boot and on reload.
if grep -q "# BEGIN $MARK" "$RULES"; then
    echo "Forwarding rules are already in $RULES"
else
    cp "$RULES" "$RULES.before-$MARK"
    cat >> "$RULES" <<EOF

# BEGIN $MARK (automation/ci/host-firewall.sh)
*filter
:DOCKER-USER - [0:0]
-A DOCKER-USER -s $SUBNET -d 10.0.0.0/8 -j DROP
-A DOCKER-USER -s $SUBNET -d 172.16.0.0/12 -j DROP
-A DOCKER-USER -s $SUBNET -d 192.168.0.0/16 -j DROP
-A DOCKER-USER -s $SUBNET -d 169.254.0.0/16 -j DROP
COMMIT
# END $MARK
EOF
fi

# To services of the host itself: proxy, SSH, VNC, anything else.
ufw insert 1 deny from "$SUBNET" to any comment "$MARK"
ufw reload

echo "Done. From a container in the network, crates.io must answer and the host must not:"
echo "  docker run --rm --network $MARK <ci image> bash -c 'curl -sI https://crates.io | head -1; timeout 3 bash -c \"</dev/tcp/192.168.1.98/22\" || echo host closed'"
