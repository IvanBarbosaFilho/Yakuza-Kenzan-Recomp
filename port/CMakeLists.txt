cmake_minimum_required(VERSION 3.20)
project(KenzanRecomp
    VERSION 0.1.0
    LANGUAGES C CXX
    DESCRIPTION "inFAMOUS 2 (NPUA80638) static recompilation, firmware LLE (ps3recomp)"
)

# ---------------------------------------------------------------------------
# Standards
# ---------------------------------------------------------------------------
set(CMAKE_C_STANDARD 17)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Prefer Ninja if available
if(NOT CMAKE_GENERATOR MATCHES "Ninja" AND NOT DEFINED ENV{CMAKE_GENERATOR})
    message(STATUS "Tip: use -G Ninja for faster builds")
endif()

# ---------------------------------------------------------------------------
# The two directories this project is built from
# ---------------------------------------------------------------------------
# PS3RECOMP_DIR is the toolkit checkout: include/, runtime/, libs/, tools/.
# RECOMP_DIR is where the lifter's output for THIS game lives. They are
# separate on purpose -- a port keeps its generated code in its own tree and
# points at whatever toolkit checkout it is building against:
#
#   cmake -B build -G Ninja \
#         -DPS3RECOMP_DIR=/path/to/ps3recomp \
#         -DRECOMP_DIR=/path/to/my_game/recompiled
#
if(NOT DEFINED PS3RECOMP_DIR)
    # Default: this project is still sitting inside ps3recomp/templates/project/
    get_filename_component(PS3RECOMP_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../" ABSOLUTE)
endif()
get_filename_component(PS3RECOMP_DIR "${PS3RECOMP_DIR}" ABSOLUTE)

# The engine exe (slim split, 2026-11-14): no lifted code is linked here. The lifted PPU/SPU
# code and the lifted firmware are the GAME MODULE (Infamous2Game.dll / libInfamous2Game.so),
# built on the player's PC by tools/build_module.py from their own disc, and bound at the start
# of main (ps3emu/ps3_module.h). The boot scaffold and the port's sources include ppu_recomp.h
# for ppu_context and the accessors: the engine's own (tools/ppu_engine_header.py), without a
# lifted prototype.
find_package(Python3 COMPONENTS Interpreter REQUIRED)
set(ENGINE_PPU_HEADER_DIR "${CMAKE_CURRENT_BINARY_DIR}/engine_include")
set(ENGINE_PPU_HEADER "${ENGINE_PPU_HEADER_DIR}/ppu_recomp.h")
add_custom_command(
    OUTPUT "${ENGINE_PPU_HEADER}"
    COMMAND "${Python3_EXECUTABLE}" -I "${PS3RECOMP_DIR}/tools/ppu_engine_header.py" "${ENGINE_PPU_HEADER}"
    DEPENDS "${PS3RECOMP_DIR}/tools/ppu_engine_header.py" "${PS3RECOMP_DIR}/tools/ppu_lifter.py"
            "${PS3RECOMP_DIR}/tools/ppu_fastpath.py"
    COMMENT "Writing the engine's ppu_recomp.h")
if(NOT EXISTS "${ENGINE_PPU_HEADER}")   # configure time too: the first build's scan needs it
    execute_process(COMMAND "${Python3_EXECUTABLE}" -I "${PS3RECOMP_DIR}/tools/ppu_engine_header.py" "${ENGINE_PPU_HEADER}")
endif()
message(STATUS "Engine header:   ${ENGINE_PPU_HEADER}")

# ---------------------------------------------------------------------------
# ps3recomp runtime library
# ---------------------------------------------------------------------------
# Built from source as a subdirectory rather than imported as a prebuilt
# archive. A static library carries none of its own link requirements, so an
# imported one arrives without SDL2, pthreads, the Metal frameworks or the
# Winsock/XInput imports -- the link then fails on every platform for a
# different reason. add_subdirectory hands all of that over with the target.
add_subdirectory("${PS3RECOMP_DIR}" "${CMAKE_BINARY_DIR}/ps3recomp_sdk"
                 EXCLUDE_FROM_ALL)

# ---------------------------------------------------------------------------
# The HLE NID table
# ---------------------------------------------------------------------------
# Maps each firmware import NID onto the handler in the runtime that answers
# it. A port may keep a hand-curated one beside its lift; otherwise generate
# the canonical every-module table, which is what the toolkit's own boot
# harness uses.
if(TRUE)
    set(HLE_NID_TABLE "${CMAKE_CURRENT_BINARY_DIR}/ppu_hle_nids.cpp")
    # The table is scanned from the library sources: a new export needs a
    # regenerated table, or its import falls through to the unresolved stub.
    file(GLOB HLE_LIB_SOURCES CONFIGURE_DEPENDS "${PS3RECOMP_DIR}/libs/*/*.c")
    add_custom_command(
        OUTPUT "${HLE_NID_TABLE}"
        COMMAND "${Python3_EXECUTABLE}"
                "${PS3RECOMP_DIR}/tools/gen_hle_nids.py"
                --out "${HLE_NID_TABLE}" --all
        DEPENDS "${PS3RECOMP_DIR}/tools/gen_hle_nids.py"
                "${PS3RECOMP_DIR}/tools/nid_database.py"
                ${HLE_LIB_SOURCES}
        COMMENT "Generating the HLE NID -> handler table")
    message(STATUS "HLE NID table:   generated (every module)")
endif()

# ---------------------------------------------------------------------------
# The game executable
# ---------------------------------------------------------------------------
# The PPU boot scaffold is compiled here rather than taken from the runtime
# library: every one of these files #includes the lifter's per-game
# ppu_recomp.h, so they can only be built against a particular lift.
set(PS3RECOMP_SCAFFOLD
    "${PS3RECOMP_DIR}/runtime/ppu/ppu_loader.cpp"
    "${PS3RECOMP_DIR}/runtime/ppu/ppu_hle.cpp"
    "${PS3RECOMP_DIR}/runtime/ppu/ppu_fs.cpp"
    "${PS3RECOMP_DIR}/runtime/ppu/ppu_sysprx.cpp"
)

# (The lifted SPU images, the lifted firmware SPU code, spu_dispatch_mt.c, the job and data
# module registrations and the lifted firmware modules -- the user's own, never redistributed --
# are the game module's: tools/build_module.py.)

add_executable(${PROJECT_NAME}
    main.cpp
    stubs.cpp
    kenzan_game.cpp
    ${PS3RECOMP_SCAFFOLD}
    ${HLE_NID_TABLE}
    "${ENGINE_PPU_HEADER}"
)
# Every engine TU reaches the game module's shared state through ps3_mv_* (the runtime library's
# own CMakeLists does the same for its TUs).
if(MSVC)
    target_compile_options(${PROJECT_NAME} PRIVATE "/FI${PS3RECOMP_DIR}/runtime/module/ps3_module_vars.h")
else()
    target_compile_options(${PROJECT_NAME} PRIVATE -include "${PS3RECOMP_DIR}/runtime/module/ps3_module_vars.h")
    # Linux: the game module (a .so) binds the engine functions it calls against the exe
    target_link_options(${PROJECT_NAME} PRIVATE -rdynamic)
    target_link_libraries(${PROJECT_NAME} PRIVATE ${CMAKE_DL_LIBS})
endif()

# app.rc carries the icon, the version block and the manifest, and needs a
# resource compiler. Listing it as a source on a host that has none is not a
# link error, it is a "no rule to make" for a file nothing can consume.
if(WIN32 AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/app.rc")
    target_sources(${PROJECT_NAME} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/app.rc")
endif()

# ---------------------------------------------------------------------------
# macOS: an app bundle, so the system treats the game as a game
# ---------------------------------------------------------------------------
# Game Mode (macOS 14+: CPU/GPU priority, half the Bluetooth latency for
# controllers and AirPods) turns on for a fullscreen, frontmost app that
# gamepolicyd labels a game, and it labels one from the bundle's Info.plist
# (LSSupportsGameMode, or LSApplicationCategoryType = games). A bare executable
# never qualifies, not even with an Info.plist embedded in __TEXT,__info_plist:
# measured on macOS 27, only the bundle was "Found game". So the build writes
#
#   build/Infamous2Recomp.app/Contents/MacOS/Infamous2Recomp
#   build/Infamous2Recomp -> Infamous2Recomp.app/Contents/MacOS/Infamous2Recomp
#
# The symlink keeps every script that runs build/Infamous2Recomp working, and
# a run through it still has the bundle's identity (also measured). It is
# started directly, never through `open`, so the environment, stdout/stderr
# and the parent process stay what run.sh and the launcher expect.
# macos_host.m: App Nap off and USER_INTERACTIVE QoS for the port's threads.
if(APPLE)
    enable_language(OBJC)
    target_sources(${PROJECT_NAME} PRIVATE macos_host.m)
    set_source_files_properties(macos_host.m PROPERTIES COMPILE_OPTIONS "-fobjc-arc")
    target_link_libraries(${PROJECT_NAME} PRIVATE "-framework Foundation")

    # The bundle icon (Finder, Game Mode's menu) is the Dock placeholder that
    # run.sh also hands the window; sips ships with macOS.
    set(GAME_ICNS "${CMAKE_CURRENT_BINARY_DIR}/Infamous2Recomp.icns")
    add_custom_command(OUTPUT "${GAME_ICNS}"
        COMMAND sips -s format icns "${CMAKE_CURRENT_SOURCE_DIR}/window_icon.png"
                --out "${GAME_ICNS}"
        DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/window_icon.png"
        COMMENT "Making the bundle icon")
    target_sources(${PROJECT_NAME} PRIVATE "${GAME_ICNS}")
    set_source_files_properties("${GAME_ICNS}" PROPERTIES MACOSX_PACKAGE_LOCATION Resources)

    set_target_properties(${PROJECT_NAME} PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/Info.plist.in"
        MACOSX_BUNDLE_GUI_IDENTIFIER "io.github.erikomis.infamous2recomp.game"
        MACOSX_BUNDLE_BUNDLE_NAME "Infamous2Recomp"
        MACOSX_BUNDLE_EXECUTABLE_NAME "${PROJECT_NAME}"
        MACOSX_BUNDLE_ICON_FILE "Infamous2Recomp.icns"
        MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
        MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}")
    # A build directory from before the bundle holds the old executable at
    # this path; the link replaces it.
    add_custom_command(TARGET ${PROJECT_NAME} POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E rm -f "${CMAKE_CURRENT_BINARY_DIR}/${PROJECT_NAME}"
        COMMAND ${CMAKE_COMMAND} -E create_symlink
                "${PROJECT_NAME}.app/Contents/MacOS/${PROJECT_NAME}"
                "${CMAKE_CURRENT_BINARY_DIR}/${PROJECT_NAME}"
        VERBATIM)
endif()

target_include_directories(${PROJECT_NAME} PRIVATE
    "${ENGINE_PPU_HEADER_DIR}"                 # the engine's ppu_recomp.h (no lifted prototypes)
    "${PS3RECOMP_DIR}/runtime/module"
    "${PS3RECOMP_DIR}/include"
    "${PS3RECOMP_DIR}/runtime/ppu"
    "${PS3RECOMP_DIR}/runtime/spu"
    "${PS3RECOMP_DIR}/runtime/memory"
    "${PS3RECOMP_DIR}/runtime/syscalls"
    "${PS3RECOMP_DIR}/runtime/prx"             # prx_loader.h, for lifted PRX modules
    "${PS3RECOMP_DIR}/runtime/platform"        # win32_compat.h and friends
    "${PS3RECOMP_DIR}/libs/video"
    "${CMAKE_CURRENT_SOURCE_DIR}"
)

target_link_libraries(${PROJECT_NAME} PRIVATE
    ps3recomp_runtime
)

# The runtime is C++, and on Apple also Objective-C. Letting CMake infer the
# link language from the generated C would leave the C++ standard library out.
set_target_properties(${PROJECT_NAME} PROPERTIES LINKER_LANGUAGE CXX)

# Windows system import libraries. dbghelp symbolizes crash-handler addresses
# against the PDB, user32 and gdi32 are what the backend's window and its GDI
# present are made of, and winmm is timeBeginPeriod. None of the four names
# anything a POSIX linker can find; the D3D12 backend's own imports come in
# through the #pragma comment(lib) in libs/video/rsx_d3d12_backend.c.
if(WIN32)
    target_link_libraries(${PROJECT_NAME} PRIVATE
        dbghelp
        user32
        gdi32
        winmm
    )
endif()

# ---------------------------------------------------------------------------
# Compiler-specific flags
# ---------------------------------------------------------------------------
if(CMAKE_CXX_COMPILER_ID MATCHES "Clang|GNU")
    target_compile_options(${PROJECT_NAME} PRIVATE
        -Wall -Wextra -Wno-unused-parameter
        -fno-strict-aliasing
        # The lift spells PPC arithmetic as signed C arithmetic, which must wrap;
        # without this, clang exploits overflow UB (e.g. folds "x*x < 0" to false).
        -fwrapv
    )
elseif(MSVC)
    target_compile_options(${PROJECT_NAME} PRIVATE
        /W3 /wd4100 /wd4244 /wd4267
        /Zc:__cplusplus         # report correct __cplusplus value
        # ppu_recomp.h defines PPU_FENCE, the lowering of the guest's
        # sync/lwsync/eieio/isync, and in C that is <stdatomic.h>. The lifter
        # writes C++ by default, but --single-file writes ppu_recomp.c, and a
        # port that took it would not compile without this. The runtime's own
        # build already carries the same flag for the same reason.
        /experimental:c11atomics
    )
    target_compile_definitions(${PROJECT_NAME} PRIVATE
        _CRT_SECURE_NO_WARNINGS
    )
endif()

# (The lifted translation units' own flags -- no warnings, /bigobj, no FP contraction,
# PORT_LIFT_OPT, PORT_SPU_ARCH -- are tools/build_module.py's now.)

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------
install(TARGETS ${PROJECT_NAME} RUNTIME DESTINATION bin BUNDLE DESTINATION .)

# The title's cheats (cheats.cpp beside this file: title knowledge, kept out of git; the
# in-game menu's Cheats page). Built only when the tree has it.
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/cheats.cpp")
    target_sources(${PROJECT_NAME} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/cheats.cpp")
endif()
