FROM redis
ADD https://github.com/brandur/redis-cell/releases/download/v0.3.0/redis-cell-v0.3.0-x86_64-unknown-linux-gnu.tar.gz /tmp
RUN ["tar", "-xf", "/tmp/redis-cell-v0.3.0-x86_64-unknown-linux-gnu.tar.gz", "-vC", "/usr/local/bin"]
CMD ["redis-server", "--loadmodule", "/usr/local/bin/libredis_cell.so"]
