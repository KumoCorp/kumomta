# Using <https://github.com/LukeMathWalker/cargo-chef> to help cache
# the rust dependencies and make rebuilds go faster.
# Note that this layer must broadly match the runtime layer;
# right now bookworm debian has matching libssl as ubuntu:latest.
# If that diverges then weird or broken linkage issues will ensue.
FROM lukemathwalker/cargo-chef:latest-rust-bookworm AS chef
RUN apt update
# Needed for building rocksdb, kafka
RUN apt install -y libclang-dev cmake
WORKDIR /app


FROM chef as planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef as builder
COPY --from=planner /app/recipe.json recipe.json
# Build the dependencies and cache them in a layer
RUN cargo chef cook --release --recipe-path recipe.json
# and now build our first-party stuff
COPY . .
RUN cargo build --release -p kumod && \
  cargo build --release -p traffic-gen && \
  cargo build --release -p proxy-server && \
  cargo build --release -p kcli

# Using Ubuntu rather than alpine because rocksdb, a C++ library,
# is problematic when built with musl.
# <https://github.com/rust-rocksdb/rust-rocksdb/issues/174>.
# It does compile, but it just SEGV'd on startup for me.
FROM ubuntu:latest as runtime
# Create a user and group to run as.
# Note that we don't use the docker USER here because we need to
# start as root and then drop to this user. That is handled by
# the docker-runner.sh script.
RUN groupadd --system --gid 1000 kumod && useradd --system --gid kumod --uid 1000 kumod
RUN apt update && apt install -y libsqlite3-dev
WORKDIR /opt/kumomta/sbin
COPY --from=builder /app/target/release/kcli .
COPY --from=builder /app/target/release/kumod .
COPY --from=builder /app/target/release/proxy-server .
COPY --from=builder /app/target/release/traffic-gen .
COPY docker/kumod/docker-runner.sh .
WORKDIR /opt/kumomta/bounce_classifier
COPY assets/bounce_classifier/*.toml .
EXPOSE 25/tcp
EXPOSE 587/tcp
EXPOSE 465/tcp
EXPOSE 2525/tcp
EXPOSE 2026/tcp
CMD ["/opt/kumomta/sbin/docker-runner.sh"]

