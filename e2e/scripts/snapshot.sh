#!/usr/bin/env bash

set -ex

cd "$(dirname "$0")"
cd ..

mkdir -p remote-logs

./scripts/pssh.sh "docker logs node > node.log && docker stop node"

NODE_NAMES=($(cat ./nodes/infra-data.json | jq -r '.instances | keys | join(" ")' ))
SSH_OPTS="-o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null"

for NODE in "${NODE_NAMES[@]}"
do
  NODE_IP=$(cat ./nodes/infra-data.json | jq -r .instances.${NODE}.public_ip)
  # Got issues when running in parallel, so we run sequentially
  scp -rpC $SSH_OPTS root@${NODE_IP}:/root/node.log remote-logs/${NODE}.log
done
