# Purpose-built Lean sandbox image for the proof farm.
#
# Bakes a *pinned* Lean toolchain into the image so proofs can be checked fully
# offline (`--network=none`) with a version that matches `proof/lean/lean-toolchain`.
# There is no maintained official Lean 4 Docker image, so we install `elan` and
# the exact toolchain here, then warm it so the `lean`/`lake` shims resolve
# without any network access at run time.
#
# Build (from the repository root):
#   docker build -f lean-farm/lean-runner.Dockerfile \
#     --build-arg LEAN_TOOLCHAIN=leanprover/lean4:v4.22.0 \
#     -t spec-to-proof/lean-runner:v4.22.0 lean-farm
#
# The farm launches this image with a hardened `docker run` (read-only root,
# cap-drop ALL, no-new-privileges, non-root user 1000, network none, resource
# caps); nothing below assumes any additional privileges.

FROM debian:bookworm-slim

# Runtime libraries Lean links against, plus certs/curl for the one-time install.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        libgmp10 \
    && rm -rf /var/lib/apt/lists/*

# Non-root user (uid/gid 1000) matching the farm's `run_as_user`/`run_as_group`.
RUN useradd --create-home --uid 1000 --user-group lean

USER lean
ENV ELAN_HOME=/home/lean/.elan \
    PATH=/home/lean/.elan/bin:$PATH

# Install elan and the pinned toolchain, then warm it so `lean` runs offline.
ARG LEAN_TOOLCHAIN=leanprover/lean4:v4.22.0
RUN curl -sSfL https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh -o /tmp/elan-init.sh \
    && sh /tmp/elan-init.sh -y --default-toolchain "${LEAN_TOOLCHAIN}" \
    && rm /tmp/elan-init.sh \
    && lean --version \
    && lake --version

# The farm bind-mounts the proof source read-only at /work and runs
# `lean /work/Proof.lean`.
WORKDIR /work
