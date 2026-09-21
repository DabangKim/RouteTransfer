#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ -x "$PWD/.toolchains/cargo/bin/cargo" ]; then
  export CARGO_HOME="$PWD/.toolchains/cargo"
  export RUSTUP_HOME="$PWD/.toolchains/rustup"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
exec npm run tauri -- dev
