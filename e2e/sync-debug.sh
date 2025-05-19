#!/bin/bash

cd "$(dirname "$0")"
cd ..

./e2e/clean.sh

# Start all nodes except node25
docker compose -f docker-compose.e2e.yml up -d --scale node25=0

sleep 60 # Wait 1 minute

# Start node25
docker compose -f docker-compose.e2e.yml up -d

sleep 60 # Wait 1 minute

# Take a snapshot of the logs for node25 and its peers
./e2e/snapshot.sh 1 5 6 7 25

# Stop all nodes
docker compose -f docker-compose.e2e.yml down
