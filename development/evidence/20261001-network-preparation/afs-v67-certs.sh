#!/usr/bin/env bash
set -euo pipefail
run=/var/lib/afs-acceptance/network-v67
[ "$(uname -m)" = aarch64 ]
[ "$(findmnt -T /var/lib -n -o FSTYPE)" = ext4 ]
[ ! -e "$run" ]
umask 077
mkdir -p "$run"/tls "$run"/logs
cd "$run"/tls
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=afs-env-v67-ca -keyout ca.key -out ca.pem > ../logs/ca.log 2>&1
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=afs-env-v67-untrusted -keyout untrusted.key -out untrusted.pem > ../logs/untrusted.log 2>&1
for row in 'ctl 192.168.109.11' 'a 192.168.109.12' 'b 192.168.109.13' 'c 192.168.109.14'; do
 read -r node ip <<< "$row"
 openssl req -new -newkey rsa:2048 -nodes -subj "/CN=afs-env-$node" -keyout "$node.key" -out "$node.csr" > "../logs/key-$node.log" 2>&1
 printf '%s\n' "subjectAltName=DNS:afs-env-$node,IP:$ip" 'extendedKeyUsage=serverAuth,clientAuth' > "$node.ext"
 openssl x509 -req -in "$node.csr" -CA ca.pem -CAkey ca.key -CAcreateserial -days 2 -extfile "$node.ext" -out "$node.pem" > "../logs/cert-$node.log" 2>&1
 tar -czf "$run/$node-tls.tgz" ca.pem untrusted.pem "$node.pem" "$node.key"
done
openssl version > ../logs/openssl-version.txt
sha256sum ca.pem untrusted.pem ctl.pem a.pem b.pem c.pem > ../logs/public-cert-inputs.sha256
