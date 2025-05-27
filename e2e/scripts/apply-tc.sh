#!/bin/bash

cd "$(dirname "$0")"
cd ..

if [ ! -f "nodes/infra-data.json" ]; then
  echo "Error: nodes/infra-data.json does not exist."
  exit 1
fi

./scripts/tc/generate-tc-scripts.py scripts/tc/latencies.csv nodes/infra-data.json ${1}

./scripts/upload-config.sh

./scripts/pssh.sh /app/config/tc-setup.sh
