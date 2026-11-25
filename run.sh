#!/usr/bin/env bash
# Runs Ryu ga Gotoku Kenzan! (BLJM-60064) on the LLE engine (Linux/Vulkan).
#   ./run.sh                 play          KZ_TIMEOUT=40 ./run.sh   debug run, killed after 40 s
# Logs: work/logs/last_run_{out,err}.txt. Extra env vars pass through.
set -u
D=$(cd "$(dirname "$0")" && pwd)
W=$D/work
BIN=${KZ_EXE:-$W/build/KenzanRecomp}
# KZ_ASAN=1: the AddressSanitizer engine (work/build-asan), to catch the boot-time heap corruption
# ("malloc(): unaligned tcache chunk"). SDL3 preloaded: sdl2-compat cannot dlopen it under ASan.
if [ -n "${KZ_ASAN:-}" ]; then BIN=$W/build-asan/KenzanRecomp; export LD_PRELOAD=/nix/store/9jr57qbn2jjcsryx36dga15ny3x6nc2a-sdl3-3.4.10-lib/lib/libSDL3.so.0
  export LD_LIBRARY_PATH=$(cat "$W/asan_ldpath.txt")${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
  export ASAN_OPTIONS=detect_leaks=0:halt_on_error=1:detect_odr_violation=0:verify_asan_link_order=0; fi
EBOOT=$D/game/PS3_GAME/USRDIR/EBOOT.elf
# kenzan.ini (the launcher's settings; the in-game menu's Frame rate writes fps= there): its fps
# is the default when KZ_FPS is not given
export KZ_INI=$D/kenzan.ini
if [ -z "${KZ_FPS:-}" ] && [ -f "$KZ_INI" ]; then KZ_FPS=$(sed -n 's/^fps=\([0-9]*\).*/\1/p' "$KZ_INI" | tail -1); fi
export KZ_FPS=${KZ_FPS:-60}
# the controls (input.toml: the launcher's Controls tab and the in-game F2 > Controls both edit it)
export PS3_INPUT_MAP=${PS3_INPUT_MAP-$D/input.toml}
# mods: one folder per mod in mods/, its files read in place of the game's (engine ppu_fs_loose_mods.h)
export PS3_LOOSE_MODS_DIR=${PS3_LOOSE_MODS_DIR-$D/mods}; mkdir -p "$PS3_LOOSE_MODS_DIR"
# the in-game menu: About / Update / Display (engine vk_overlay_menu.inc); no aspect patch in this build
export PS3_PORT_NAME=${PS3_PORT_NAME-"Ryu ga Gotoku Kenzan! PC port"} PS3_PORT_BY=${PS3_PORT_BY-"by IvanBarbosaFilho"}
export PS3_PORT_URL=${PS3_PORT_URL-https://github.com/IvanBarbosaFilho/Yakuza-Kenzan-Recomp} PS3_UPDATE_BY=${PS3_UPDATE_BY-git}
export PS3_ASPECT_UNSUPPORTED=${PS3_ASPECT_UNSUPPORTED-1}
# KZ_FPS=60 (default): the community 60 FPS patch (RPCS3 patch.yml "Framerate - Core: 60 FPS", Pakundo)
# applied to the EBOOT (tools/apply_rpcs3_patch.py -> work/EBOOT_60.elf) and lifted into its own module.
# KZ_FPS=30: the game as shipped.
# KZ_FPS=120: the patch's "Framerate - Core: 120 FPS" (EBOOT_120.elf, its own module), the vblank at 120 Hz.
#   120 everywhere outside the festival; the festival drops to ~40 (main thread: PPU<->SPU store coherence).
if [ "${KZ_FPS:-60}" = 120 ] && [ -f "$W/EBOOT_120.elf" ] && [ -f "$W/build/libInfamous2Game_120.so" ]; then
  EBOOT=$W/EBOOT_120.elf; export PS3_VFS_ROOT=${PS3_VFS_ROOT:-$D/game} KZ_VBLANK_HZ=${KZ_VBLANK_HZ:-120}
  export PS3_GAME_MODULE=${PS3_GAME_MODULE:-$W/build/libInfamous2Game_120.so}
elif [ "${KZ_FPS:-60}" = 60 ] && [ -f "$W/EBOOT_60.elf" ] && [ -f "$W/build/libInfamous2Game_60.so" ]; then
  EBOOT=$W/EBOOT_60.elf; export PS3_VFS_ROOT=${PS3_VFS_ROOT:-$D/game}
  export PS3_GAME_MODULE=${PS3_GAME_MODULE:-$W/build/libInfamous2Game_60.so}
fi
mkdir -p "$W/hdd0" "$W/hdd1" "$W/no_flash" "$W/logs"
rm -rf "$W/hdd1/cache"
export PS3_TITLE="Ryu ga Gotoku Kenzan! (ps3recomp LLE)"
export PS3_HDD0_ROOT=$W/hdd0 PS3_HDD1_ROOT=$W/hdd1 PS3_DEV_FLASH=$W/no_flash PS3_NP_STRICT_OFFLINE=1
export PS3_GAME_MODULE=${PS3_GAME_MODULE:-$W/build/libInfamous2Game.so}
# Kenzan GPU sync (engine/libs/video/cellGcmSys.c "(Kenzan)"): the flip-sequence semaphore 3 nobody releases.
export GCM_SEMA_ACQUIRE=${GCM_SEMA_ACQUIRE:-1} GCM_SEMA_ACQUIRE_MS=${GCM_SEMA_ACQUIRE_MS:-50}
# Rumble off: with it on, 2 of 2 boots died in malloc ("unaligned tcache chunk", heap corrupted before the
# first present; 2026-11-22), none with it off. PS3_RUMBLE=1 turns it on (cellPad.c rumble path to check).
export PS3_RUMBLE=${PS3_RUMBLE:-0}
export GCM_NO_TITLE_RECYCLE=${GCM_NO_TITLE_RECYCLE:-1}
# Flip semaphore (label 0x30) freed and label 1 cleared at the vblank after the present (cellGcmSys.c
# KZ_FLIPSEMA / KZ_VSYNC), as the display does: the title paces itself, no 50 ms give-up per frame.
# (The menu blinking with them was the engine presenting "buffer 7": rsx_draw_engine.c sink_flip.)
export KZ_FLIPSEMA=${KZ_FLIPSEMA:-1} KZ_VSYNC=${KZ_VSYNC:-1}
# inFAMOUS 2 enhancement defaults that are not Kenzan's: shadow maps at the PS3 size (2x = the engine default).
export PS3RECOMP_SHADOW_RES=${PS3RECOMP_SHADOW_RES:-1}
# Black spots on characters: the shadow-mask pass (fp fafb66cb1802b231, 780 draws) re-draws the scene with
# a depth-equal test and another vertex program; FMA contraction made its depth differ by ULPs, the test
# failed in random pixels and the mask read "shadowed" there. NoContraction in every vertex program fixes it.
export PS3RECOMP_VP_PRECISE=${PS3RECOMP_VP_PRECISE:-1}
# Internal resolution 1080p (1.5x of the PS3's 720p), the user's monitor; 720p/1440p/4k also take.
export PS3RECOMP_RENDER_SCALE=${PS3RECOMP_RENDER_SCALE:-1080p}
# Live knobs: edit work/toggles.txt while the game runs (NAME=value per line, re-read every 250 ms).
export PS3_TOGGLE_FILE=${PS3_TOGGLE_FILE:-$W/toggles.txt}
# The inFAMOUS 2 native job rewrites, by job number: not Kenzan's jobs.
export PS3_NATIVE_JOBS=${PS3_NATIVE_JOBS:-0} PS3_NATIVE_CULL=${PS3_NATIVE_CULL:-0} PS3_NATIVE_ANIM=${PS3_NATIVE_ANIM:-0}
# touch work/bt_now while the game is frozen: every thread prints its stack to the log
# (diag, off) KZ_WATCH_EA=<hex EA>: hardware write watch on a main-stack slot (stubs.cpp); traps on every write
# (diag, off) SPU_PUT_RANGE=<lo>,<hi>: log SPU DMA writes into that range
export KZ_FLIPMARK=${KZ_FLIPMARK:-1}
export KZ_BT_TRIGGER=${KZ_BT_TRIGGER:-$W/bt_now}
# touch work/fplog_now: the next frame's fragment programs are listed in the log ([kzfp])
export KZ_FPLOG_TRIGGER=${KZ_FPLOG_TRIGGER:-$W/fplog_now}
# touch work/shdump_now: the next frame's shadow map and shadow mask are saved in work/shdump
export KZ_SHDUMP_TRIGGER=${KZ_SHDUMP_TRIGGER:-$W/shdump_now} KZ_SHDUMP_DIR=${KZ_SHDUMP_DIR:-$W/shdump}
# echo <fp hex> > work/vpcap_now (or KZ_VPCAP_FP=<fp>): the next draw's vertex program inputs, for tools/rsx_vp_ref.py
export KZ_VPCAP_TRIGGER=${KZ_VPCAP_TRIGGER:-$W/vpcap_now} KZ_VPCAP_DIR=${KZ_VPCAP_DIR:-$W/vpcap}
# touch work/step_now: the next frame stepped draw by draw (work/step/NN_DDDD_target.ppm + NN_index.txt)
# echo "pad" > work/kzwatchr_now: who reads the pad buttons (8 s of [kzbt] lines; tools/kz_symbolize.py)
export KZ_WATCHR_TRIGGER=${KZ_WATCHR_TRIGGER:-$W/kzwatchr_now}
export KZ_STEP_TRIGGER=${KZ_STEP_TRIGGER:-$W/step_now} KZ_STEP_DIR=${KZ_STEP_DIR:-$W/step}
# (diag) [kzshmap]: blits, depth targets and draws touching the shadow maps' memory
# export KZ_SHMAP_LOG=1
export PS3RECOMP_VK_WINDOW=1 PS3RECOMP_VK_GUEST_PROGRAMS=1
export PS3RECOMP_SPIRV_CACHE=${PS3RECOMP_SPIRV_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/kenzan-lle/spirv}
mkdir -p "$PS3RECOMP_SPIRV_CACHE"
[ -n "${KZ_TIMEOUT:-}" ] && export PS3_STDIO_UNBUFFERED=${PS3_STDIO_UNBUFFERED:-1}
run() { if [ -n "${KZ_TIMEOUT:-}" ]; then timeout -s KILL "$KZ_TIMEOUT" "$BIN" "$EBOOT"; else "$BIN" "$EBOOT"; fi; }
cd "$W"
# Test runs (KZ_TIMEOUT) log to last_test_*: a player session may be open and writing last_run_*.
LOG=last_run; [ -n "${KZ_TIMEOUT:-}" ] && LOG=last_test; export LOG
if [ -n "${IN_NIX_SHELL:-}" ]; then run >"$W/logs/${LOG}_out.txt" 2>"$W/logs/${LOG}_err.txt"
else export -f run; export BIN EBOOT KZ_TIMEOUT W
  nix-shell "$D/../shell.nix" --run 'run >"$W/logs/${LOG}_out.txt" 2>"$W/logs/${LOG}_err.txt"' 2>/dev/null; fi
echo "exit=$?  logs: $W/logs/${LOG}_{out,err}.txt"
# A session without KZ_TIMEOUT (a player's run) keeps its own copy: later test runs overwrite last_run_*.
if [ -z "${KZ_TIMEOUT:-}" ]; then
  ts=$(date +%Y%m%d-%H%M%S); mkdir -p "$W/logs/player"
  cp "$W/logs/last_run_err.txt" "$W/logs/player/$ts-err.txt"; cp "$W/logs/last_run_out.txt" "$W/logs/player/$ts-out.txt"
  echo "player logs: $W/logs/player/$ts-{out,err}.txt"
fi
