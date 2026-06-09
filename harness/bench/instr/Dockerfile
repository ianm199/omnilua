FROM rust:slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends valgrind make gcc \
    && rm -rf /var/lib/apt/lists/*
