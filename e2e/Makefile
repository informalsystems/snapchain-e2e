NUM_NODES ?= 3
NODES ?= $(shell (seq -f'"node%g"' -s',' 1 $(NUM_NODES) | sed 's/,$$//'))

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
	rm -rdf grafana/graphite
	rm -rdf grafana/grafana/data

perturb:
	docker compose down node25
	sleep 30
	docker compose up -d

# Start node25 one minute after all other nodes.
sync-debug:
	docker compose up -d --scale node25=0
	sleep 60
	docker compose up -d node25

infra-create:
	cd terraform && terraform apply -parallelism=200 -var='testnet_dir=../nodes' -var='node_names=[$(NODES)]'

infra-destroy:
	cd terraform && terraform destroy -parallelism=12 -var='testnet_dir=../nodes' -var='node_names=[$(NODES)]'

.PHONY: build setup start stop clean perturb sync-debug
