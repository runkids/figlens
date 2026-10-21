FROM rust:1.97.1
RUN rustup component add rustfmt clippy
# `figlens` on PATH runs this checkout's source (see .devcontainer/bin/figlens).
ENV PATH="/work/.devcontainer/bin:${PATH}"
