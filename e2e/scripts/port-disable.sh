#!/usr/bin/env bash

cd "$(dirname "$0")"
cd ..

NODE_NAME=$1
PORT=$2

./scripts/ssh-node.sh $NODE_NAME ufw deny $PORT