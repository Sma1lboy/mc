# mc-server (axum + better-auth + sqlx) production image. The server is
# stateless — point DATABASE_URL at Postgres (a Railway Postgres plugin or
# Supabase) and set AUTH_SECRET; PORT is injected by the platform.
#
# Multi-stage: build the release binary, then run on a slim base with CA certs
# (needed for outbound HTTPS to the DB + Mojang/Modrinth, all via rustls).
FROM rust:1-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release -p mc-server

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/mc-server /usr/local/bin/mc-server
# Bind all interfaces so the platform router can reach the container.
ENV HOST=0.0.0.0
EXPOSE 8787
CMD ["mc-server"]
