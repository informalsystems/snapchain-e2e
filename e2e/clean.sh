#!/bin/bash

cd "$(dirname "$0")"
cd ..

rm -rf grafana/grafana/data grafana/graphite/storage grafana/prometheus/data
