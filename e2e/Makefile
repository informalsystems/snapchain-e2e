# Testnet parameters
NUM_VALIDATORS := 2
NUM_FULL_NODES := 2
NUM_SHARDS := 2
FIRST_FULL_NODES := 50
BANDWIDTH := 1gbit

NUM_NODES := $(shell expr $(NUM_VALIDATORS) + $(NUM_FULL_NODES))
VALIDATORS ?= $(shell (seq -f'"val%g"' -s',' 1 $(NUM_VALIDATORS) | sed 's/,$$//'))
FULL_NODES ?= $(shell (seq -f'"full%g"' -s',' 1 $(NUM_FULL_NODES) | sed 's/,$$//'))
TESTNET_DIR := nodes
CC_IP ?= $(shell cat ./$(TESTNET_DIR)/.cc-ip)
TERRAFORM_VARS := -var='testnet_dir=$(TESTNET_DIR)' \
	-var='validator_names=[$(VALIDATORS)]' \
	-var='full_node_names=[$(FULL_NODES)]' \
	-var='num_shards=$(NUM_SHARDS)' \
	-var='first_full_nodes=$(FIRST_FULL_NODES)' \
	-var='bandwidth=$(BANDWIDTH)'
IMAGE_NAME := snapchain-node

build:
	time docker build -t $(IMAGE_NAME)-local ..

setup:
	cargo run --bin setup_e2e_testnet -- --num-validators=$(NUM_VALIDATORS) --num-full-nodes=$(NUM_FULL_NODES) --num-shards=$(NUM_SHARDS)

start:
	docker compose up -d
	cargo run --bin testnet_init_users

load:
	cargo run --bin testnet_spam -- --rate 500 --max-time 30

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

remote-build:
	time docker build -t $(IMAGE_NAME) --platform linux/amd64 ..

remote-create:
	cd terraform && time terraform apply -parallelism=200 $(TERRAFORM_VARS)

# Locally create the nodes' config files.
remote-setup:
	cargo run --bin setup_remote_testnet -- --topology default

remote-destroy:
	cd terraform && time terraform destroy -parallelism=200 $(TERRAFORM_VARS)

remote-start:
	./scripts/pssh.sh /app/start.sh

remote-stop:
	./scripts/pssh.sh docker stop node

remote-perturb:
	./scripts/ssh-node.sh full1 docker stop node
	sleep 60
	./scripts/ssh-node.sh full1 /app/start.sh

remote-snapshot:
	./scripts/snapshot.sh

# Reset the app state in all nodes.
remote-reset-states:
	./scripts/pssh.sh '(docker stop node || true) && rm -rdf /app/data/.rocks'

.PHONY: remote-deploy remote-destroy remote-start remote-stop remote-perturb remote-reset-states
