#!/usr/bin/env bash

set -ex

NODE_INDEX=$1
NODE_IP=$(cat ./nodes/infra-data.json | jq -r .instances.node${NODE_INDEX}.ext_ip_address)

# TODO: check first argument is valid; check json file exists

# Run command from node
ssh_node() {
    ssh -o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null root@$NODE_IP -t "$@"
}

ssh_node "${@:2}"
