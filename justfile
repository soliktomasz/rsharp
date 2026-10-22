# RSharp's commands. Builds land in target/, warnings are errors (.cargo/config.toml).

build:
    cargo build

run *args:
    cargo run --bin rsharp -- {{args}}

test filter="":
    cargo test --workspace {{filter}}

release:
    cargo build --release

# The main window captured to OUT after it settles (macOS only): just screenshot out.png PATH --file F --caret L:C
screenshot out *args: build
    tools/screenshot.sh {{out}} {{args}}
