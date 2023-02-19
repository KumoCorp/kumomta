FROM alpine:3.16
WORKDIR /app
COPY target/x86_64-unknown-linux-musl/release/kumod .
COPY ci/docker-runner.sh .
EXPOSE 25
EXPOSE 587
EXPOSE 465
EXPOSE 2525
EXPOSE 2026
CMD ["/app/docker-runner.sh"]

