#!/usr/bin/env bash

set -ex

CC_ADDR=$(cat ./nodes/.cc-ip)

# Run command from CC
ssh_cc() {
    ssh -o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null root@$CC_ADDR -t "$@"
}

ssh_cc "$@"
