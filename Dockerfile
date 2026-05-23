# The binary is cross-compiled in CI before this Dockerfile runs.
# This stage just packages it into a minimal image.

FROM scratch
COPY target/aarch64-unknown-linux-musl/release/flight-game-server /flight-game-server
# Run the binary when the container starts
CMD ["/flight-game-server"]