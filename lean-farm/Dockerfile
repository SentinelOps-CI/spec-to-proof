# Multi-stage build for Lean Farm
# Stage 1: Build stage
FROM rust:1.75-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy Cargo files
COPY Cargo.toml Cargo.lock ./

# Create dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies
RUN cargo build --release --bin lean-farm

# Remove dummy main.rs and copy real source
RUN rm src/main.rs
COPY src ./src

# Build the application
RUN cargo build --release --bin lean-farm

# Stage 2: Runtime stage
FROM debian:bookworm-slim as runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    docker.io \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r leanfarm && useradd -r -g leanfarm leanfarm

# Create necessary directories
RUN mkdir -p /var/lean-farm/{artifacts,cache,code,tmp} \
    && chown -R leanfarm:leanfarm /var/lean-farm

# Copy binary from builder
COPY --from=builder /app/target/release/lean-farm /usr/local/bin/

# Copy configuration
COPY config /etc/lean-farm/

# Set ownership
RUN chown -R leanfarm:leanfarm /usr/local/bin/lean-farm \
    && chown -R leanfarm:leanfarm /etc/lean-farm

# Security hardening
RUN chmod 755 /usr/local/bin/lean-farm \
    && chmod 644 /etc/lean-farm/*

# Drop capabilities
RUN setcap -r /usr/local/bin/lean-farm

# Set read-only root filesystem
RUN mkdir -p /tmp /var/lean-farm/tmp \
    && chmod 1777 /tmp /var/lean-farm/tmp

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Expose ports
EXPOSE 8080 9090

# Switch to non-root user
USER leanfarm

# Set working directory
WORKDIR /var/lean-farm

# Set environment variables. The Lean toolchain itself lives in the separate
# sandbox runner image (see lean-runner.Dockerfile); this service image only
# needs a Docker client to launch those hardened sandboxes.
ENV LEAN_FARM_SECURE=true \
    LEAN_FARM_ISOLATED=true \
    RUST_LOG=info

# Entry point
ENTRYPOINT ["/usr/local/bin/lean-farm"]

# Default command
CMD ["--config=/etc/lean-farm/config.yaml"] 