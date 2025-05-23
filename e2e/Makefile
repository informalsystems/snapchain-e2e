NUM_NODES := 2
NODES ?= $(shell (seq -f'"node%g"' -s',' 1 $(NUM_NODES) | sed 's/,$$//'))
TESTNET_DIR := "./nodes"
CC_IP ?= $(shell cat $(TESTNET_DIR)/.cc-ip)

IMAGE_NAME := snapchain-node

build:
	time docker build -t $(IMAGE_NAME) --platform linux/amd64 --build-context "code=.." .
	
config:	
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
	cd terraform && time terraform apply -parallelism=200 -var='testnet_dir=../nodes' -var='node_names=[$(NODES)]'

infra-destroy:
	cd terraform && time terraform destroy -parallelism=12 -var='testnet_dir=../nodes' -var='node_names=[$(NODES)]'

.PHONY: build setup start stop clean perturb sync-debug

image-upload:
	docker image save $(IMAGE_NAME) -o snapchain-image.tar
	./scripts/scp-cc.sh snapchain-image.tar 
	./scripts/ssh-cc.sh mv /root/snapchain-image.tar /data
	./scripts/ssh-cc.sh chown nobody:nogroup /data/snapchain-image.tar
