FROM rust:1-alpine AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin asimov-simulator

FROM alpine:3
COPY --from=build /app/target/release/asimov-simulator /usr/local/bin/
USER nobody
CMD ["asimov-simulator"]
