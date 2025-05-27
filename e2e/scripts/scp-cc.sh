#!/usr/bin/env bash

set -ex

CC_ADDR=$(cat ./nodes/.cc-ip)
SSH_OPTS="-o LogLevel=ERROR -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null"

# Copy file to cc
scp -C $SSH_OPTS $@ root@${CC_ADDR}:/root

