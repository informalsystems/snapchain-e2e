build:
	docker compose build
	cargo build --bin setup_e2e_testnet

setup:
	../target/debug/setup_e2e_testnet

start:
	docker compose up -d

stop:
	docker compose down --remove-orphans --volumes

clean: stop
	rm -rdf nodes
	rm -rdf monitoring/graphite/storage
	rm -rdf monitoring/grafana/data
	rm -rdf monitoring/prometheus/data

perturb:
	docker compose down node25
	sleep 30
	docker compose up -d

# Start node25 one minute after all other nodes.
sync-debug:
	docker compose up -d --scale node25=0
	sleep 60
	docker compose up -d node25

.PHONY: build setup start stop clean perturb sync-debug
