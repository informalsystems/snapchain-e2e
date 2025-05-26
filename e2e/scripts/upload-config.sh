#!/usr/bin/env bash

set -ex 

NODE_NAMES=($(cat ./nodes/infra-data.json | jq -r '.instances | keys | join(" ")' ))
SSH_OPTS="-o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null"

echo NODE_NAMES=$NODE_NAMES

for NODE in "${NODE_NAMES[@]}"
do
    NODE_IP=$(cat ./nodes/infra-data.json | jq -r .instances.${NODE}.public_ip)
    scp -C $SSH_OPTS ./nodes/$NODE/config.toml root@${NODE_IP}:/app/config/config.toml
done
