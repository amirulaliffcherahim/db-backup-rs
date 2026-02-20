# Build stage
FROM rust:latest as builder

WORKDIR /usr/src/db-backup-rs
COPY . .
RUN cargo install --path .

# Runtime stage
FROM debian:bookworm-slim

# Install database clients for dumping
RUN apt-get update && apt-get install -y \
    mariadb-client \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/db-backup-rs /usr/local/bin/db-backup-rs

# Create a non-root user (optional but recommended)
# RUN useradd -m appuser
# USER appuser

CMD ["db-backup-rs", "daemon"]
