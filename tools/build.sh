#!/usr/bin/env bash
# Build Kenzan on the LLE engine (engine/), the way the inFAMOUS 2 port's Linux build does:
#   1. the engine exe (CMake + Ninja: runtime, HLE libraries, Vulkan, the runner in port/) -> work/build/KenzanRecomp
#   2. the game module (tools/build_module.py: lifted EBOOT/SPU/jobs + lifted firmware) -> work/build/libInfamous2Game.so
# (The module file name is the engine's fixed PS3_MODULE_FILE; it holds Kenzan's code.)
# Inside nix-shell ../shell.nix, or it re-enters it.  Usage: tools/build.sh [engine|module]  (default both)
set -eo pipefail
D=$(cd "$(dirname "$0")/.." && pwd)
if [ -z "${IN_NIX_SHELL:-}" ]; then exec nix-shell "$D/../shell.nix" --run "bash '$0' $*" 2>/dev/null; fi
B=$D/work/build
what=${1:-both}
if [ "$what" != module ]; then
  flags=; grep -qw avx2 /proc/cpuinfo && flags="-march=x86-64-v3 -ffp-contract=off"
  cmake -S "$D/port" -B "$B" -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
    -DCMAKE_C_FLAGS="$flags" -DCMAKE_CXX_FLAGS="$flags" -DCMAKE_EXE_LINKER_FLAGS=-fuse-ld=lld \
    -DPS3RECOMP_DIR="$D/engine" -DPS3RECOMP_RSX_VULKAN=ON -DPS3RECOMP_BUILD_TESTS=OFF \
    -DPS3RECOMP_RL_ACHIEVEMENTS=OFF -DPS3RECOMP_FFMPEG_RUNTIME=ON > "$D/work/logs/configure.log" 2>&1 \
    || { tail -30 "$D/work/logs/configure.log"; exit 1; }
  ninja -C "$B" -j "${JOBS:-10}" KenzanRecomp
fi
if [ "$what" != engine ]; then
  # build_module.py wants an LLVM folder: bin/clang + bin/ld.lld (the nix-shell's)
  L=$D/work/llvm; mkdir -p "$L/bin"
  ln -sfn "$(command -v clang)" "$L/bin/clang"; ln -sfn "$(command -v clang++)" "$L/bin/clang++"
  ln -sfn "$(command -v ld.lld)" "$L/bin/ld.lld"
  python3 -I "$D/tools/build_module.py" --port "$D/port" --engine "$D/engine" --out "$B/module" \
    --dll-dir "$B" --llvm "$L" --host linux --jobs "${JOBS:-10}"
fi
