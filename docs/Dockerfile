FROM rust:latest AS rust

WORKDIR /toml2jsonc
COPY crates/toml2jsonc .
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --target x86_64-unknown-linux-musl

FROM docker.io/squidfunk/mkdocs-material
RUN pip install mkdocs-macros-plugin mkdocs-include-markdown-plugin mkdocs-exclude mkdocs-git-revision-date-localized-plugin
COPY --from=rust /toml2jsonc/target/x86_64-unknown-linux-musl/debug/toml2jsonc /util/toml2jsonc

CMD ["build"]
