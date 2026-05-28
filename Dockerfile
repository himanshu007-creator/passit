FROM rust:alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
COPY src ./src
RUN cargo build --release

FROM alpine:3.20
RUN apk add --no-cache libgcc
COPY --from=build /app/target/release/passit /usr/local/bin/passit
ENTRYPOINT ["passit"]
