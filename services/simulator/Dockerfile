# Build context: services/ (the Cargo workspace).
FROM rust:1-alpine AS build
# OpenSSL for the SpacetimeDB SDK's WebSocket TLS, linked statically.
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconf
ENV OPENSSL_STATIC=1
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --locked --bin asimov-simulator && cp target/release/asimov-simulator /usr/local/bin/

FROM alpine:3
COPY --from=build /usr/local/bin/asimov-simulator /usr/local/bin/
USER nobody
CMD ["asimov-simulator"]
