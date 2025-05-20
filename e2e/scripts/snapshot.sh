#!/bin/bash

cd "$(dirname "$0")"
cd ..

mkdir -p logs

for id in "$@"; do
  echo "Saving logs for node $id"
  docker logs "snapchain-e2e-node$id-1" > "logs/node$id.log"
done
