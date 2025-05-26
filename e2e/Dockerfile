# syntax=docker.io/docker/dockerfile:1.7-labs
# The above line enables experimental features (namely, `COPY --exclude`)

# Add the "code" build context to access local source code during build.
#  docker build . --build-context code=$SNAPCHAIN_DIR

FROM rust:1.85 AS builder

RUN apt-get update && apt-get install -y libclang-dev git libjemalloc-dev llvm-dev make protobuf-compiler libssl-dev openssh-client cmake

WORKDIR /usr/src

ARG MALACHITE_GIT_REPO_URL=https://github.com/informalsystems/malachite.git
ENV MALACHITE_GIT_REPO_URL=$MALACHITE_GIT_REPO_URL
ARG MALACHITE_GIT_REF=13bca14cd209d985c3adf101a02924acde8723a5
RUN <<EOF
set -eu
git clone $MALACHITE_GIT_REPO_URL
cd malachite
git checkout $MALACHITE_GIT_REF
EOF

ARG ETH_SIGNATURE_VERIFIER_GIT_REPO_URL=https://github.com/CassOnMars/eth-signature-verifier.git
ENV ETH_SIGNATURE_VERIFIER_GIT_REPO_URL=$ETH_SIGNATURE_VERIFIER_GIT_REPO_URL
ARG ETH_SIGNATURE_VERIFIER_GIT_REF=8deb4a091982c345949dc66bf8684489d9f11889
RUN echo "clear cache" # Invalidate cache to pick up latest eth-signature-verifier
RUN <<EOF
set -eu
git clone $ETH_SIGNATURE_VERIFIER_GIT_REPO_URL
cd eth-signature-verifier
git checkout $ETH_SIGNATURE_VERIFIER_GIT_REF
EOF

WORKDIR /usr/src/app

# Unfortunately, we can't prefetch creates without including the source code,
# since the Cargo configuration references files in src.
# This means we'll re-fetch all crates every time the source code changes,
# which isn't ideal.
COPY --from=code ./Cargo.toml ./build.rs ./
COPY --from=code --exclude=src/bin src ./src

ENV RUST_BACKTRACE=full
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release

#################################################################################

FROM ubuntu:24.04

WORKDIR /app
COPY --from=builder /usr/src/app/src/proto /app/proto
COPY --from=builder /usr/src/app/target/release/snapchain /app/

RUN apt-get update && apt-get install -y iproute2
COPY <<'EOS' /usr/local/bin/entrypoint.sh
#!/bin/sh
set -e
tc qdisc add dev eth0 root netem delay 100ms 10ms distribution normal || true
exec "$@"
EOS
RUN chmod +x /usr/local/bin/entrypoint.sh

ENV RUSTFLAGS="-Awarnings"
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["/app/snapchain", "--config-path", "/app/config/config.toml"]

EXPOSE 3381 3382 3383 3483 50051
