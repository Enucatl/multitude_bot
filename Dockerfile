FROM rust:1-bookworm

USER 10001
WORKDIR /usr/src/app

COPY Cargo.toml .

# copy entity and migration crates for SeaORM
COPY entity entity
COPY migration migration

# cargo needs a lib.rs or main.rs file to compile dependencies
RUN mkdir src\
    && echo "//dummy file" > src/lib.rs\
    && cargo build\
    && rm src/lib.rs

# now copy and build the actual application
COPY src src
RUN cargo build

CMD ["./target/debug/multitude_bot"]
