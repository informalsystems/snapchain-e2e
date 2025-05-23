#!/usr/bin/env bash

set -ex

TESTNET_DIR=${1:-nodes}
CC_ADDR=$(cat $TESTNET_DIR/.cc-ip)

# Run command from CC
ssh_cc() {
    ssh -o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null root@$CC_ADDR -t "$@"
}

ssh_cc "$@"
