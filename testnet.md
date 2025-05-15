# E2E testnet

## Steps

### Build

Build Docker images
```sh
docker compose build
```

### Setup

Create config files for all nodes:
```sh
cargo build --bin setup_local_testnet
./target/debug/setup_local_testnet
```

### Start

```sh
docker compose up -d
```

### Stop

```sh
docker compose down
```
