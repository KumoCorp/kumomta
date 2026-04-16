FROM --platform=$BUILDPLATFORM rust:latest AS rust
ARG TARGETPLATFORM

WORKDIR /toml2jsonc
COPY crates/toml2jsonc .
RUN case "$TARGETPLATFORM" in \
      "linux/amd64") echo x86_64-unknown-linux-musl > /target.txt ;; \
      "linux/arm64"|"linux/arm64/v8") echo aarch64-unknown-linux-musl > /target.txt ;; \
      *) echo "Do $TARGETPLATFORM"; exit 1 ;; \
    esac
RUN rustup target add $(cat /target.txt)
RUN cargo build --target $(cat /target.txt)

FROM docker.io/squidfunk/mkdocs-material
RUN pip install mkdocs-macros-plugin mkdocs-include-markdown-plugin mkdocs-exclude mkdocs-git-revision-date-localized-plugin
RUN pip install --force-reinstall click==8.2.1 # https://github.com/squidfunk/mkdocs-material/issues/8478#issuecomment-3386761434
COPY --from=rust /toml2jsonc/target/*/debug/toml2jsonc /util/toml2jsonc

CMD ["build"]
