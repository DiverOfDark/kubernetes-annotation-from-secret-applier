FROM rust:alpine
RUN apk add pkgconfig openssl-dev libc-dev
# cargo-auditable embeds the resolved crate list in the binary (.dep-v0
# section, survives strip) so Trivy can report Rust CVEs from the image.
RUN cargo install cargo-auditable@0.7.7 --locked

ADD Cargo.toml /app/
ADD Cargo.lock /app/
ADD src /app/src
RUN cd /app && cargo auditable build --release
RUN ls -la /app/target/release/

FROM alpine
RUN apk update \
    && apk add openssl ca-certificates
COPY --from=0 /app/target/release/kubernetes-annotation-from-secret-applier /app/
RUN ls -la /app/
CMD ["/app/kubernetes-annotation-from-secret-applier"]