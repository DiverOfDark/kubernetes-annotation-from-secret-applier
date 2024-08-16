FROM rust:alpine
RUN apk add pkgconfig openssl-dev libc-dev

ADD Cargo.toml /app/
ADD Cargo.lock /app/
ADD src /app/src
RUN cd /app && cargo build --release
RUN ls -la /app/target/release/

FROM alpine
RUN apk update \
    && apk add openssl ca-certificates
COPY --from=0 /app/target/release/kubernetes-annotation-from-secret-applier /app/
RUN ls -la /app/
CMD ["/app/kubernetes-annotation-from-secret-applier"]