#!/usr/bin/env bash

## This script executes a command in all nodes in parallel.

set -ex

SSH_OPTS="-O LogLevel=ERROR -O StrictHostKeyChecking=no -O UserKnownHostsFile=/dev/null -O GlobalKnownHostsFile=/dev/null"
TIMEOUT=120
NUM_NODES=$(cat ./nodes/infra-data.json | jq -r '[.instances[]] | length')

# space-separated list of all node IP addresses
HOST_IPS=$(cat ./nodes/infra-data.json | jq -r '[.instances[].public_ip] | join(" ")')

pssh -l root $SSH_OPTS -i -v -p $NUM_NODES -t $TIMEOUT -H "$HOST_IPS" "$@"
