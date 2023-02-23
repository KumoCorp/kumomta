FROM alpine:3.16
WORKDIR /app
COPY docker-runner.sh .
COPY kumod .
EXPOSE 25/tcp
EXPOSE 587/tcp
EXPOSE 465/tcp
EXPOSE 2525/tcp
EXPOSE 2026/tcp
CMD ["/app/docker-runner.sh"]

