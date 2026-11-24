#!/usr/bin/env bash
# Opens the Kenzan launcher (settings + Play). Builds it the first time.
D=$(cd "$(dirname "$0")" && pwd)
B=$D/launcher/target/release/kenzan-launcher
if [ ! -x "$B" ]; then nix-shell "$D/launcher/shell.nix" --run "cd '$D/launcher' && cargo build --release" || exit 1; fi
exec nix-shell "$D/launcher/shell.nix" --run "KZ_ROOT='$D' '$B'"
