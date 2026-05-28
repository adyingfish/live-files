FROM rust:1-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release --bin live-files-server

FROM debian:bookworm-slim
COPY --from=build /app/target/release/live-files-server /usr/local/bin/
EXPOSE 8080
ENTRYPOINT ["live-files-server"]
