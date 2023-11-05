FROM rust:1-bookworm

USER 10001
WORKDIR /usr/src/app

COPY Cargo.toml .

# cargo needs a lib.rs or main.rs file to compile dependencies
# remove lines 1-3 and last two lines
# these contain dependenceis on my crates and I don't want to donwload/recompile everything
# every time I change those
RUN mkdir src\
    && echo "//dummy file" > src/lib.rs\
    && sed -i '1,3d' Cargo.toml\
    && sed -i -e :a -e '$d;N;2,3ba' -e 'P;D' Cargo.toml\
    && cargo build\
    && rm src/lib.rs\
    && rm Cargo.toml

# copy the full Cargo.toml as well as entity and migration crates for SeaORM
COPY Cargo.toml .
COPY entity entity
COPY migration migration

# now copy and build the actual application
COPY src src
RUN cargo build

CMD ["./target/debug/multitude_bot"]
