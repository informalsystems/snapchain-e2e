#!/usr/bin/env bash

set -ex 

for i in {1..50}; do
	# ./scripts/ssh-node.sh full$i docker stop node && rm -rdf /app/data/.rocks/ &
	./scripts/ssh-node.sh full$i docker stop node && echo "Node full$i stopping..." &
done

echo Waiting...
sleep 60

for i in {1..50}; do
	./scripts/ssh-node.sh full$i /app/start.sh && echo "Node full$i restarting" &
done
