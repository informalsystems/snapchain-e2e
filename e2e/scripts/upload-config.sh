#!/usr/bin/env bash

set -ex

cd "$(dirname "$0")"
cd ..

NODE_NAMES=($(cat ./nodes/infra-data.json | jq -r '.instances | keys | join(" ")' ))
SSH_OPTS="-o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null"

echo NODE_NAMES=$NODE_NAMES

for NODE in "${NODE_NAMES[@]}"
do
    NODE_IP=$(cat ./nodes/infra-data.json | jq -r .instances.${NODE}.public_ip)
    scp -rpC $SSH_OPTS ./nodes/$NODE/* root@${NODE_IP}:/app/config/ &
done

# Remove app data
./scripts/pssh.sh rm -rdf /app/data/.rocks
