#!/usr/bin/env bash
# Builds the Windows release folder from Linux (cross-compiled, nothing runs on Windows here):
#   Kenzan Launcher.exe               the launcher (Rust, x86_64-pc-windows-msvc)
#   engine/KenzanRecomp.exe, SDL2.dll the engine (clang-cl + lld-link, static C runtime)
#   engine/KenzanGame{,_60,_120}.dll  the game modules (tools/build_module.py --host windows)
#   engine/EBOOT_{60,120}.elf         the frame-rate patched executables
#   engine/7z.exe, 7z.dll             7-Zip, for the launcher's install
# The toolchain lives in $WIN (default work/win): xwin's MSVC CRT + Windows SDK
# (xwin/splat), toolchain-clangcl.cmake, shell.nix, SDL2 (sdl2/, sdl2-inc/), glslang built /MT
# (glslang-install/), rustup with the msvc target (rustup/, cargo/), 7zip/.
#   bash packaging/windows/build_windows.sh [engine] [launcher] [modules] [package]   (default: all)
set -euo pipefail
D=$(cd "$(dirname "$0")/../.." && pwd)
WIN=${WIN:-$D/work/win}
X=$WIN/xwin/splat
OUT=${OUT:-$WIN/Kenzan-Windows}
steps=${*:-engine launcher modules package}
has() { [[ " $steps " == *" $1 "* ]]; }

if has engine; then
  echo "== engine"
  VH=$(nix-build '<nixpkgs>' -A vulkan-headers --no-out-link)
  nix-shell "$WIN/shell.nix" --run "cmake -S '$D/port' -B '$WIN/build-engine' -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE='$WIN/toolchain-clangcl.cmake' -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded -DPS3RECOMP_DIR='$D/engine' -DPS3RECOMP_RSX_VULKAN=ON \
    -DPS3RECOMP_BUILD_TESTS=OFF -DPS3RECOMP_RL_ACHIEVEMENTS=OFF -DPS3RECOMP_VULKAN_INCLUDE_DIR='$VH/include' \
    -DPS3RECOMP_SDL2_INCLUDE_PARENT='$WIN/sdl2-inc' -DSDL2_DIR='$WIN/sdl2/SDL2-2.32.10/cmake' \
    -Dglslang_DIR='$WIN/glslang-install/lib/cmake/glslang' -DPS3RECOMP_REQUIRE_GUEST_PROGRAMS=ON \
    '-DCMAKE_C_FLAGS_RELEASE=/O2 /Ob2 /DNDEBUG /Z7' '-DCMAKE_CXX_FLAGS_RELEASE=/O2 /Ob2 /DNDEBUG' \
    '-DCMAKE_EXE_LINKER_FLAGS_RELEASE=/INCREMENTAL:NO /DEBUG /OPT:REF /OPT:ICF' > '$WIN/engine-cfg.log' 2>&1 \
    && nice -n 19 ninja -C '$WIN/build-engine' -j '${JOBS:-10}' KenzanRecomp > '$WIN/engine-build.log' 2>&1" \
    || { grep -E 'error|FAILED' "$WIN/engine-build.log" | head -20; exit 1; }
fi

if has launcher; then
  echo "== launcher"
  export RUSTUP_HOME=$WIN/rustup CARGO_HOME=$WIN/cargo
  nix-shell -p rustup llvmPackages.lld llvmPackages.clang-unwrapped llvmPackages.llvm --run "
    export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=lld-link
    export RUSTFLAGS='-Lnative=$X/crt/lib/x86_64 -Lnative=$X/sdk/lib/um/x86_64 -Lnative=$X/sdk/lib/ucrt/x86_64 -Ctarget-feature=+crt-static'
    export CC_x86_64_pc_windows_msvc=clang-cl AR_x86_64_pc_windows_msvc=llvm-lib
    export CFLAGS_x86_64_pc_windows_msvc='--target=x86_64-pc-windows-msvc /imsvc$X/crt/include /imsvc$X/sdk/include/ucrt /imsvc$X/sdk/include/um /imsvc$X/sdk/include/shared'
    cd '$D/launcher' && nice -n 19 rustup run stable cargo build --release --target x86_64-pc-windows-msvc \
      --target-dir '$WIN/launcher-target' 2>&1 | grep -E '^error|Finished'"
fi

if has modules; then
  echo "== game modules (30, 60, 120)"
  mkdir -p "$WIN/llvm/bin"
  nix-shell "$WIN/shell.nix" --run "ln -sfn \$(command -v clang-cl) '$WIN/llvm/bin/clang-cl'; ln -sfn \$(command -v lld-link) '$WIN/llvm/bin/lld-link'
    cd '$D'
    for f in 30 60 120; do
      p=work/port\$f; [ \$f = 30 ] && p=port
      nice -n 19 python3 -I tools/build_module.py --port \$p --engine engine --out '$WIN/module'\$f --dll-dir '$WIN/m'\$f \
        --llvm '$WIN/llvm' --host windows --jobs '${JOBS:-12}' > '$WIN/mod'\$f.log 2>&1 || { echo \"module \$f FAILED\"; tail -5 '$WIN/mod'\$f.log; exit 1; }
      tail -1 '$WIN/mod'\$f.log
    done"
fi

if has package; then
  echo "== package -> $OUT"
  rm -rf "$OUT"; mkdir -p "$OUT/engine" "$OUT/licenses"
  cp "$WIN/launcher-target/x86_64-pc-windows-msvc/release/kenzan-launcher.exe" "$OUT/Kenzan Launcher.exe"
  cp "$WIN/build-engine/KenzanRecomp.exe" "$WIN/sdl2/SDL2-2.32.10/lib/x64/SDL2.dll" "$OUT/engine/"
  cp "$WIN/m30/Infamous2Game.dll" "$OUT/engine/KenzanGame.dll"
  cp "$WIN/m60/Infamous2Game.dll" "$OUT/engine/KenzanGame_60.dll"
  cp "$WIN/m120/Infamous2Game.dll" "$OUT/engine/KenzanGame_120.dll"
  cp "$D/work/EBOOT_60.elf" "$D/work/EBOOT_120.elf" "$OUT/engine/"
  cp "$WIN/7zip/7z.exe" "$WIN/7zip/7z.dll" "$OUT/engine/"
  cp "$WIN/7zip/License.txt" "$OUT/licenses/7-Zip-License.txt"
  cp "$WIN/sdl2/SDL2-2.32.10/LICENSE.txt" "$OUT/licenses/SDL2-License.txt"
  cp "$D/launcher/assets/OFL-ShipporiMincho.txt" "$OUT/licenses/"
  du -sh "$OUT"
fi
