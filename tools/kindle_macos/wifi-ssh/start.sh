#!/bin/sh
# Key-only SSH over Wi-Fi for Ignite development, reachable from ONE Mac only.
# Install on the Kindle as /var/local/ignite-ssh/start.sh (root, mode 700) after setting MAC to your
# Mac's fixed LAN address, and start it from scribe-lifecycle-host.sh (see ../README.md). Idempotent.
D=/var/local/ignite-ssh
MAC=__MAC_IP__            # e.g. 192.168.1.20; give the Mac a DHCP reservation in your router
PORT=2222
DB=/mnt/us/jobman/bin/dropbearmulti
case $MAC in __*) echo "start.sh: set MAC first" >&2; exit 1 ;; esac
# Firewall: Wi-Fi port $PORT from the Mac only (everything else stays dropped).
iptables -C INPUT -i wlan0 -p tcp -s $MAC --dport $PORT -j ACCEPT 2>/dev/null \
  || iptables -I INPUT 1 -i wlan0 -p tcp -s $MAC --dport $PORT -j ACCEPT
# Server: key login only (-s: no passwords for anyone). This jobman dropbear build reads
# /mnt/us/jobman/ssh/etc/dropbear/authorized_keys, not the -H home.
if [ -f $D/dropbear.pid ] && kill -0 "$(cat $D/dropbear.pid)" 2>/dev/null; then exit 0; fi
exec $DB dropbear -R -r $D/host_ed25519 -p $PORT -H $D -s -g -K 60 -P $D/dropbear.pid
