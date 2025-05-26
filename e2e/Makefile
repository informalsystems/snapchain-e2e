NUM_VALIDATORS := 2
NUM_FULL_NODES := 2
NUM_NODES := $(shell expr $(NUM_VALIDATORS) + $(NUM_FULL_NODES))
VALIDATORS ?= $(shell (seq -f'"val%g"' -s',' 1 $(NUM_VALIDATORS) | sed 's/,$$//'))
FULL_NODES ?= $(shell (seq -f'"full%g"' -s',' 1 $(NUM_FULL_NODES) | sed 's/,$$//'))
TESTNET_DIR := "./nodes"
CC_IP ?= $(shell cat $(TESTNET_DIR)/.cc-ip)

IMAGE_NAME := snapchain-node

build:
	time docker build -t $(IMAGE_NAME) --platform linux/amd64 --build-context "code=.." .
	
build-config:	
	cargo build --bin setup_e2e_testnet
	cargo build --bin setup_remote_testnet

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

.PHONY: build setup start stop clean perturb sync-debug 

remote-create:
	cd terraform && time terraform apply -parallelism=200 \
		-var='testnet_dir=nodes' -var='validator_names=[$(VALIDATORS)]' -var='full_node_names=[$(FULL_NODES)]'

remote-destroy:
	cd terraform && time terraform destroy -parallelism=200 \
		-var='testnet_dir=nodes' -var='validator_names=[$(VALIDATORS)]' -var='full_node_names=[$(FULL_NODES)]'

remote-start:
	./scripts/pssh.sh /app/start.sh

remote-stop:
	./scripts/pssh.sh docker stop node


.PHONY: remote-deploy remote-destroy remote-start remote-stop
