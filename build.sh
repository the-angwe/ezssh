#!/usr/bin/env bash
# ezSsh build helper. Works in Git Bash on Windows, WSL, macOS, and Linux.
#
# Usage:
#   ./build.sh            # cargo run (debug), the real database
#   ./build.sh run        # same as above; extra arguments go to ezssh
#   ./build.sh build      # cargo build (debug)
#   ./build.sh release    # cargo build --profile dist → target/dist/ezssh
#   ./build.sh package    # installers for this OS via cargo-packager
#   ./build.sh test       # cargo test + clippy
#   ./build.sh clean      # remove target/
#   ./build.sh help
set -euo pipefail

cd "$(dirname "$0")"

color() { printf '\033[1;36m>>>\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m!!!\033[0m %s\n' "$*" >&2; exit 1; }

require() {
  command -v "$1" >/dev/null 2>&1 || err "$1 not found in PATH. Install it and retry."
}

cmd="${1:-run}"
[ $# -gt 0 ] && shift

case "$cmd" in
  run)
    require cargo
    color "cargo run -p ezssh-app"
    exec cargo run -p ezssh-app -- "$@"
    ;;

  build)
    require cargo
    color "cargo build (debug)"
    exec cargo build --workspace
    ;;

  release)
    require cargo
    color "cargo build --profile dist (fat LTO, a few minutes)"
    exec cargo build -p ezssh-app --profile dist --locked
    ;;

  package)
    require cargo
    command -v cargo-packager >/dev/null 2>&1 \
      || err "cargo-packager not found. Install it: cargo install cargo-packager --locked"
    color "cargo packager (see crates/ezssh-app/packaging/README.md)"
    exec cargo packager -p ezssh-app --release "$@"
    ;;

  test)
    require cargo
    color "cargo test"
    cargo test --workspace
    color "cargo clippy"
    cargo clippy --workspace --all-targets -- -D warnings
    color "All checks passed."
    ;;

  clean)
    color "Removing target/"
    rm -rf target
    color "Clean."
    ;;

  help|-h|--help)
    sed -n '3,12p' "$0"
    ;;

  *)
    err "Unknown command: $cmd. Run ./build.sh help."
    ;;
esac
