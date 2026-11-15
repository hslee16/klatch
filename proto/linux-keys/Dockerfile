# Test images for Linux key custody (epic #387, DESIGN section 32 item 51). Build and run through `run.sh`, which
# picks the target. Nothing here is shipped; the images exist so the keystore code meets a real Secret Service.
#
#   base    Debian 12 with the Rust toolchain and what klatch builds with (vendored SQLCipher needs perl and make).
#   bus     base plus dbus, gnome-keyring and secret-tool: a session bus and an unlocked login collection.
#   nobus   base plus secret-tool only: the command exists, there is no session bus and no keyring daemon.
#   bare    base alone: no secret-tool at all.
FROM debian:12-slim AS base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends build-essential perl pkg-config ca-certificates curl procps \
 && rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --uid 1000 tester
USER tester
ENV HOME=/home/tester CARGO_HOME=/home/tester/.cargo RUSTUP_HOME=/home/tester/.rustup \
    CARGO_TARGET_DIR=/home/tester/target PATH=/home/tester/.cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
 && mkdir -p /home/tester/target
# mount points the run script puts cache volumes on: made here so the volumes inherit this user as owner
RUN mkdir -p /home/tester/.cargo/registry
WORKDIR /src/proto

FROM base AS bus
USER root
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends dbus gnome-keyring libsecret-tools \
 && rm -rf /var/lib/apt/lists/*
USER tester

FROM base AS nobus
USER root
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends libsecret-tools \
 && rm -rf /var/lib/apt/lists/*
USER tester

FROM base AS bare
