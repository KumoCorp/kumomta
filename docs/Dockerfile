FROM rust:latest AS rust

COPY Cargo.toml .
COPY crates /crates
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --target x86_64-unknown-linux-musl -p toml2jsonc

FROM docker.io/squidfunk/mkdocs-material
RUN pip install mkdocs-macros-plugin mkdocs-include-markdown-plugin mkdocs-exclude mkdocs-git-revision-date-localized-plugin
COPY --from=rust /target/x86_64-unknown-linux-musl/debug/toml2jsonc /util/toml2jsonc

CMD ["build"]
