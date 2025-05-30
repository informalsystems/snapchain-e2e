#!/usr/bin/env bash

cd "$(dirname "$0")"
cd ../..

./clean.sh

# Start all nodes except node25
docker compose up -d --scale node25=0

sleep 60 # Wait 1 minute

# Start node25
docker compose up -d

sleep 60 # Wait 1 minute

# Take a snapshot of the logs for node25 and its peers
./snapshot.sh 1 5 6 7 25

# Stop all nodes
docker compose down
