# E2E testnet

## Local testnet

Run the testnet with:
```sh
make build
make setup
make start
make load
make perturb
make stop
make clean
```

## Remote testnet

The testnet will be deployed to DigitalOcean.

### Setup DigitalOcean in Terraform

1. Set up your [personal access token for DO](https://docs.digitalocean.com/reference/api/create-personal-access-token/)
    ```bash
    doctl auth init
    ```
    If you have executed this and the following steps before, you may be able to skip to step 5.
    And if your token expired, you may need to force the use of the one you just generated here by using `doctl auth init -t <new token>` instead.
    ```bash
    doctl auth init -t dop_v1_0123456789abcdef...
    ```
2. Get the fingerprint of the SSH key you want to be associated with the root user on the created VMs
    ```bash
    doctl compute ssh-key list
    ```
3. Set up your Digital Ocean credentials as Terraform variables. Be sure to write them to `./tf/terraform.tfvars` as this file is ignored in `.gitignore`.
    ```bash
    cat <<EOF > ./tf/terraform.tfvars
    do_token = "dop_v1_0123456789abcdef..."
    ssh_keys = ["ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89"]
    EOF
    ```
4. Initialize Terraform (only needed once)
    ```bash
    make terraform-init
    ```

### Preparation required for deploying nodes

5. Before creating the remote nodes, the Docker image should exist locally. Build it with:
    ```sh
    make remote-build
    ```
    This command creates an image with the tag `snapchain-node` and it may take a rather long time to compile.

### Run the testnet

6. Define the size of the testnet by setting the variables `NUM_VALIDATORS` and `NUM_FULL_NODES` in
   `Makefile`.

7. Deploy the nodes, including their config files:
    ```sh
    make remote-create
    ```

    When you finish with the tests, don't forget to destroy the nodes!
    ```sh
    make remote-destroy
    ```

8. Start/stop all nodes in the testnet:
    ```sh
    make remote-start
    make remote-stop
    ```

### Optionally 

9. See the logs of a node:
    ```sh
    ./scripts/ssh-node.sh val1 docker logs -f node
    ```
    Validator nodes are named `val1`, `val2`, ... and full nodes are named `full1`, `full2`, ...

10. Re-upload all config files:
    ```sh
    ./scripts/upload-config.sh
    ```

11. Take down and restart multiple full nodes simultaneously:
    ```sh
    ./scripts/perturb.sh
    ```

12. Temporarily disable a port in a given node:
    ```sh
    ./scripts/port-disable.sh val1 3381
    sleep 60
    ./scripts/port-enable.sh val1 3381
    ```
    Port 3381 is for RPC and port 3383 is for HTTP.

13. Reset the app state in all nodes, to be able to run multiple experiments without destroying the nodes.
    ```sh
    make remote-reset-states
    ```

### Finally

99. Don't forget to destroy the testnet:
    ```
    make remote-destroy
    ```
