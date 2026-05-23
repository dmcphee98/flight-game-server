# Multi-stage build:
# Stage 1 (builder): A large image with the full Rust toolchain, Zig, and the app source code.
#                    Used by GitHub Actions to compile the binary, then is discarded.
# Stage 2 (scratch): An empty base image receives just the compiled binary.
#                    Has no OS, keeping the image small and eliminating OS-level security vulnerabilities.
#                    Only this image is pushed to ECR.

# --- Builder ---
FROM rust:1 AS builder
WORKDIR /app

# GitHub Actions runners are x86. Our EC2 instance is ARM64 (t4g.micro).
# To cross-compile, we install the aarch64-unknown-linux-musl target.
RUN rustup target add aarch64-unknown-linux-musl
# cargo-zigbuild simplifies cross-compilation and improves build performance.
RUN cargo install cargo-zigbuild
# Copy the entire project into the container.
COPY . .
# Compile for ARM64
RUN cargo zigbuild --release --target aarch64-unknown-linux-musl

# --- Scratch ---
FROM scratch
COPY --from=builder /app/target/aarch64-unknown-linux-musl/release/flight-game-server /flight-game-server
# Run the binary when the container starts
CMD ["/flight-game-server"]