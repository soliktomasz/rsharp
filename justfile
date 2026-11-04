# RSharp's commands. Builds land in target/, warnings are errors (.cargo/config.toml).

build:
    cargo build

run *args:
    cargo run --bin rsharp -- {{args}}

# Every test, under cargo-nextest (cargo install cargo-nextest --locked): every binary's tests at once, the debugging
# ones one at a time (.config/nextest.toml).
test filter="":
    cargo nextest run --workspace {{filter}}

release:
    cargo build --release

# The main window captured to OUT after it settles (macOS only): just screenshot out.png PATH --file F --caret L:C
screenshot out *args: build
    tools/screenshot.sh {{out}} {{args}}
