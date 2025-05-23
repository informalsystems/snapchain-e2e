# E2E testnet

## How to run the testnet

```sh
make build
make setup
make start
make perturb
make stop
make clean
```

## Deploy to DigitalOcean


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

### How to deploy

Build Docker image (with tag `snapchain-node`):
```
make build
```

Export image as file:
```
docker image save snapchain-node -o snapchain-image.tar
```

Deploy nodes:
```
make infra-create
```

When you finish with the test, don't forget to:
```
make infra-destroy
```
