FROM --platform=$BUILDPLATFORM alpine:latest AS builder
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
      "linux/arm/v7") echo armv7-unknown-linux-gnueabihf.tar.gz > /target.txt ;; \
      "linux/amd64") echo x86_64-unknown-linux-gnu.tar.gz > /target.txt ;; \
      "linux/arm64") echo aarch64-unknown-linux-gnu.tar.gz > /target.txt ;; \
      *) echo "Do $TARGETPLATFORM"; exit 1 ;; \
    esac
RUN apk --no-cache add curl
RUN curl -fvsSL https://github.com/brandur/redis-cell/releases/download/v0.3.1/redis-cell-v0.3.1-$(cat /target.txt) | tar -xzvC /tmp

FROM redis
COPY --from=builder /tmp/libredis_cell* /usr/local/bin
CMD ["redis-server", "--loadmodule", "/usr/local/bin/libredis_cell.so"]
