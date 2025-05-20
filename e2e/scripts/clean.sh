#!/bin/bash

cd "$(dirname "$0")"
cd ..

rm -rf monitoring/grafana/data monitoring/graphite/storage monitoring/prometheus/data
