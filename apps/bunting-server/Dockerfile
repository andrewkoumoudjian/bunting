# Bunting venue server container image (ADR 0044).
#
# Build from the repository root:
#   docker build -f apps/bunting-server/Dockerfile -t bunting .
# Run with a configuration mounted at /etc/bunting/server.json and durable
# state under /var/lib/bunting. Configurations bind FIX and admin listeners
# to loopback unless a mutual-TLS terminator is configured, so run the image
# in a pod with its TLS terminator, or with host networking:
#   docker run --network host -v "$PWD/etc:/etc/bunting:ro" \
#     -v bunting-state:/var/lib/bunting bunting
# The image contains the same `bunting` executable as the release archives;
# `bunting server` runs the venue in-process.

FROM rust:1.88.0-bookworm AS build
# Use the image's toolchain as is; rust-toolchain.toml would otherwise add
# clippy, rustfmt and the wasm32 target, which a release build does not use.
ENV RUSTUP_TOOLCHAIN=1.88.0
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p bunting-cli \
    && install -D -m 0755 target/release/bunting /out/usr/local/bin/bunting \
    && install -D -m 0644 -t /out/usr/share/bunting/config apps/bunting-server/config/*.json \
    && install -d -m 0755 /out/etc/bunting \
    && install -d -m 0700 -o 65532 -g 65532 /out/var/lib/bunting

# glibc runtime: the static musl build measured about 2.7x slower order
# acknowledgements (implementation log, slice 27).
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /out/ /
USER 65532:65532
VOLUME ["/var/lib/bunting"]
WORKDIR /var/lib/bunting
ENTRYPOINT ["/usr/local/bin/bunting"]
CMD ["server", "/etc/bunting/server.json"]
