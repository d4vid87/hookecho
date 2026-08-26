# HookEcho as a service: `--serve` in a container, radar snapshots included.
#
# The snapshot endpoint renders through wgpu with no display attached, so the runtime image
# carries Mesa's lavapipe software Vulkan driver. That is what makes this image ~400 MB rather
# than ~100 MB; the JSON endpoints alone would not need it, but a radar viewer that can't produce
# a radar image is a strange thing to ship.

FROM rust:bookworm AS build

# Same list CI installs — eframe/rodio want ALSA and the windowing headers even for a build that
# never opens a window.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev libxcb1-dev libgtk-3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .
RUN cargo build --release -p hookecho --bin hookecho

FROM debian:bookworm-slim

# Runtime halves of the build dependencies, plus lavapipe (mesa-vulkan-drivers) for the renderer.
# No ca-certificates: the app uses rustls with webpki-roots compiled in.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2 libxkbcommon0 libwayland-client0 libxcb1 libgtk-3-0 \
        mesa-vulkan-drivers libvulkan1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/hookecho /usr/local/bin/hookecho

# Settings (your saved locations are what this reports on) and the tile/volume cache.
VOLUME ["/root/.config/hookecho", "/root/.cache/hookecho"]
EXPOSE 8080

# 0.0.0.0 inside the container is the whole point of the container; publish the port deliberately
# (`-p 127.0.0.1:8080:8080` to keep it on the host's loopback). Publishing it anywhere else wants
# a token: set `serve_token` in the mounted settings.json, or add `--serve-token <secret>` here.
ENTRYPOINT ["hookecho", "--serve", "8080", "--bind", "0.0.0.0"]
