#!/usr/bin/env bash

cd "$(dirname "$0")"
cd ../..

mkdir -p logs

for id in "$@"; do
  echo "Saving logs for node $id"
  docker compose logs "node$id" > "logs/node$id.log"
done
