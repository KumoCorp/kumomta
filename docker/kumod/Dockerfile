# Using <https://github.com/LukeMathWalker/cargo-chef> to help cache
# the rust dependencies and make rebuilds go faster
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
RUN apt update
# Needed for building rocksdb
RUN apt install -y libclang-dev
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
RUN cargo build --release -p kumod && cargo build --release -p traffic-gen

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
WORKDIR /app
COPY --from=builder /app/target/release/kumod .
COPY --from=builder /app/target/release/traffic-gen .
COPY docker/kumod/docker-runner.sh .
EXPOSE 25/tcp
EXPOSE 587/tcp
EXPOSE 465/tcp
EXPOSE 2525/tcp
EXPOSE 2026/tcp
CMD ["/app/docker-runner.sh"]

