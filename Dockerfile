FROM rust:1.56.1 as builder
WORKDIR /usr/src/tcp_proxy
COPY . .
RUN cargo install --path .

FROM debian:buster-slim
COPY --from=builder /usr/local/cargo/bin/tcp_proxy /usr/local/bin/tcp_proxy
CMD ["tcp_proxy"]
