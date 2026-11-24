/*
 * ps3recomp game project -- entry point template.
 *
 * This is the runner. It boots a lifted title through the toolkit's PPU
 * scaffold: allocate the flat guest VM, load the PPU ELF named on the command
 * line, register the lifted function table and the HLE NID handlers, start a
 * frame clock, and dispatch the entry OPD.
 *
 * The scaffold does the work; nothing below reimplements it. ppu_load_elf,
 * ppu_recomp_register, ppu_hle_init, ppu_sysprx_register, ppu_fs_register,
 * lv2_init_syscalls and ppu_run all come from the toolkit, compiled into this
 * project from PS3RECOMP_DIR (see CMakeLists.txt). What is left here is the
 * part a port owns: which backend to present through, how the frame clock is
 * paced, and whatever diagnostics the title turns out to need.
 *
 * runtime/ppu/tests/boot_main.cpp in the toolkit is the same boot in its
 * fully-instrumented form -- crash filter, hang watchdog, guest-PC sampling
 * profiler. Read it when a boot goes wrong; copy from it what the title
 * needs. It is deliberately not what a fresh project starts with.
 *
 * The one part that IS duplicated from it is the frame clock below, because a
 * port is expected to change its pacing and a shared one would be the wrong
 * shape for that. It was copied faithfully, comments and all. If a pacing bug
 * is fixed in one of the two, look at the other.
 *
 * This builds on Windows, macOS and Linux. CI builds and runs it against the
 * boot smoke title on the last two, and compile-checks it against clang-cl on
 * the first (tools/check_ppu_scaffold.py).
 */

/* The lifter's generated header, which declares ppu_context and the function
 * table. It comes first: everything below is written against that struct. */
#include "ppu_recomp.h"

/* The Win32 names -- Sleep, GetTickCount64, CreateThread, InterlockedIncrement,
 * VirtualAlloc, the scalar typedefs -- from one place. On Windows this is a
 * passthrough to <windows.h>; off Windows the toolkit's shim supplies the same
 * names over pthreads and mmap, so the call sites below are written once. */
#include "win32_compat.h"
#include "ps3emu/ps3_ftrace.h"   /* PS3_FTRACE: present spans */
#include "ps3emu/ps3_toggles.h"  /* PS3_TOGGLE_FILE: knobs switched while the game runs */
#if defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#include <pthread.h>
/* macos_host.m: App Nap off for the run; USER_INTERACTIVE QoS with PS3_QOS=1. */
extern "C" void infamous2_host_begin_activity(void);
extern "C" int  infamous2_host_qos_enabled(void);
extern "C" void infamous2_host_qos_self(void);
#else
static inline void infamous2_host_begin_activity(void) {}
static inline int  infamous2_host_qos_enabled(void) { return 0; }
static inline void infamous2_host_qos_self(void) {}
#endif

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifndef _WIN32
#include <sys/mman.h>   /* the guest window (guest_vm_map_flat) */
#endif

#ifdef _WIN32
/* timeBeginPeriod: <windows.h> arrives with WIN32_LEAN_AND_MEAN set, which
 * excludes the multimedia timer API, so it is asked for by name -- and after
 * windows.h, since timeapi.h uses UINT. No shim exists off Windows because
 * there is nothing to shim: the 15.6 ms default timer granularity this works
 * around is a Windows problem. */
#include <timeapi.h>
#endif

/* ---------------------------------------------------------------------------
 * What the scaffold gives this file
 * -----------------------------------------------------------------------*/
extern "C" {
uint32_t ppu_load_elf(const char* path);      /* ELF -> guest RAM, returns entry OPD */
void infamous2_apply_patches(const char* elf_path);   /* game_patches.cpp: PS3_PATCHES */
void infamous2_apply_aspect(const char* elf_path);    /* game_patches.cpp: PS3_ASPECT (hor+) */
void     ppu_recomp_register(void);           /* generated: lifted table -> address map */
void     ppu_hle_init(void);                  /* firmware import NID -> HLE handler */
void     ppu_sysprx_register(void);           /* boot-critical CRT (sys_initialize_tls, ...) */
void     ppu_fs_register(void);               /* cellFs over the real game directory */
void     lv2_init_syscalls(void);             /* the lv2 syscall table */
void     cellGame_init_from_paramsfo(const char* sfo_path);  /* libs/system/cellGame.c */
int      ppu_run(uint32_t entry_opd, uint32_t stack_top);
/* toolkit ppu_loader.cpp (ppu-mem): the guest window's reservation, the hot ranges in large pages
 * with PS3_LARGE_PAGES; a toolkit without it links ppu_vm_reserve_plain below (the old one). */
uint8_t* ppu_vm_reserve(uint64_t size);

/* This project's own: load whatever real system PRX modules the title needs
 * into guest RAM and register their exports. stubs.cpp has the empty version
 * a game with no lifted PRX wants. */
void     ps3_load_prx_modules(void);

extern const char* ppu_vfs_root;              /* host dir the PS3 mount points map into */

/* The status the guest handed to sys_process_exit. A title that exits that way
 * never returns through ppu_run, so this is only read on the path where the
 * entry function unwound instead. */
extern int     g_sys_process_exit_called;
extern int32_t g_sys_process_exit_code;

/* Host-provided symbols the runtime and the HLE libraries link against. */
uint8_t* vm_base = nullptr;
extern uint32_t ppu_vm_size;                  /* ppu_loader.cpp: the OOB guard */

/* Guest-callback dispatch. g_ps3_guest_caller is the hook the HLE runtime
 * calls back into recompiled code through -- cellSysutil events and the GCM
 * vblank/flip handlers. ppu_guest_call does the OPD -> dispatch. */
typedef void (*ps3_guest_caller_fn)(uint32_t, uint64_t, uint64_t, uint64_t, uint64_t,
                                    uint64_t, uint64_t, uint64_t, uint64_t);
extern ps3_guest_caller_fn g_ps3_guest_caller;
uint64_t ppu_guest_call(uint32_t, uint64_t, uint64_t, uint64_t, uint64_t,
                        uint64_t, uint64_t, uint64_t, uint64_t);

/* The GCM half of the frame clock. */
void     cellGcmTickVBlank(void);
void     cellGcmTickFlip(void);
int      cellGcm_take_flip_pending(void);
void     cellGcm_rsx_process_fifo(void);      /* drain get -> put */
unsigned cellGcm_flip_request_count(void);
int      cellGcm_drain_sema_blocked(void);
}

/* ---------------------------------------------------------------------------
 * RSX present backend
 * -----------------------------------------------------------------------*/
/* D3D12 on Windows, Metal on Apple, and the null backend's headless software
 * path anywhere else -- the same selection runtime/host/host_posix.c and the
 * toolkit's boot harness make. All three expose the same three entry points,
 * so the frame clock below is backend-agnostic. */
#if defined(PS3RECOMP_RSX_VULKAN) && !defined(__APPLE__)
/* Vulkan when the toolkit is built with it (-DPS3RECOMP_RSX_VULKAN=ON), ahead
 * of D3D12: it is the backend that runs the shared draw engine, where the
 * inFAMOUS 2 render fixes live (the D3D12 backend drives the older live_draw
 * path and shows this title black). */
extern "C" int  rsx_vulkan_backend_init(uint32_t w, uint32_t h, const char* title);
extern "C" void rsx_vulkan_backend_present(void);
extern "C" int  rsx_vulkan_backend_pump_messages(void);
#  define rsx_backend_init    rsx_vulkan_backend_init
#  define rsx_backend_present rsx_vulkan_backend_present
#  define rsx_backend_pump    rsx_vulkan_backend_pump_messages
#  define RSX_BACKEND_NAME    "Vulkan"
#elif defined(_WIN32)
extern "C" int  rsx_d3d12_backend_init(uint32_t w, uint32_t h, const char* title);
extern "C" void rsx_d3d12_backend_present(void);
extern "C" int  rsx_d3d12_backend_pump_messages(void);
#  define rsx_backend_init    rsx_d3d12_backend_init
#  define rsx_backend_present rsx_d3d12_backend_present
#  define rsx_backend_pump    rsx_d3d12_backend_pump_messages
#  define RSX_BACKEND_NAME    "D3D12"
#elif defined(__APPLE__)
extern "C" int  rsx_metal_backend_init(uint32_t w, uint32_t h, const char* title);
extern "C" void rsx_metal_backend_present(void);
extern "C" int  rsx_metal_backend_pump_messages(void);
#  define rsx_backend_init    rsx_metal_backend_init
#  define rsx_backend_present rsx_metal_backend_present
#  define rsx_backend_pump    rsx_metal_backend_pump_messages
#  define RSX_BACKEND_NAME    "Metal"
#else
extern "C" int  rsx_null_backend_init(uint32_t w, uint32_t h, const char* title);
extern "C" void rsx_null_backend_present(void);
extern "C" int  rsx_null_backend_pump_messages(void);
#  define rsx_backend_init    rsx_null_backend_init
#  define rsx_backend_present rsx_null_backend_present
#  define rsx_backend_pump    rsx_null_backend_pump_messages
#  define RSX_BACKEND_NAME    "null (headless software)"
#endif

/* ---------------------------------------------------------------------------
 * Configuration
 * -----------------------------------------------------------------------*/
/* The flat VM must span every region the PS3 memory map uses, not just the
 * game image: sys_ppu_thread_create puts thread stacks at 0xD0000000, so a
 * smaller arena makes every spawned thread's stack access out of bounds. */
#define VM_SIZE       0x100010000ull   /* the full 32-bit guest space + a 64K guard */
#define STACK_TOP     0x0FF00000u      /* main-thread stack, below the 0x10000000 segment */
#define WINDOW_WIDTH  1280
#define WINDOW_HEIGHT 720

/* ---------------------------------------------------------------------------
 * Frame clock
 * -----------------------------------------------------------------------*/
/* On real hardware the RSX raises a vblank interrupt about 60 times a second
 * and that is what drives the game's frame loop. With no RSX it is synthesized
 * here: a host thread calls cellGcmTickVBlank/TickFlip, which invoke the
 * handlers the guest registered, and drains the GCM FIFO. Without this the
 * title initialises, registers its handlers and then waits forever.
 *
 * The ticks are driven off real elapsed time rather than off how long present()
 * took. A hidden or occluded window makes present block hard, and pacing the
 * ticks behind it paces the whole game behind it. */
static volatile LONG g_frames_presented = 0;

/* Frames handed to the backend at a guest FLIP boundary -- one per frame the
 * guest actually finished. The presents made before the guest's first flip, so
 * a fresh window is not left blank through a long boot, carry no guest frame
 * and are deliberately not counted. */
extern "C" unsigned ppu_boot_frames_presented(void)
{
    return (unsigned)g_frames_presented;
}

/* GCM_FLIP_IMMEDIATE=1: complete the flip when the frame is presented instead
 * of on the next 16 ms vblank tick. The title waits on the flip before it
 * starts the next frame, and on a frame that takes ~70 ms the tick costs it
 * ~8 ms on average for nothing. */
static int g_flip_immediate = 0;

#if defined(__APPLE__)
/* PS3RECOMP_METAL_FPS=1: the window title shows display and guest FPS. */
extern "C" void rsx_metal_backend_note_guest_frame(void);
#endif

/* game_clock.c: PS3_GCLOCK_LOG (the game's own frame clock, per frame). */
extern "C" void infamous2_gclock_frame(void);
/* Toolkit sys_timer.c: PS3_TB_SCALE, the guest timebase's rate in percent.
 * Weak: a toolkit without the knob links, and the toggle does nothing. */
#if defined(_MSC_VER)
extern "C" volatile int ps3_tb_scale_pct;
#else
extern "C" __attribute__((weak)) volatile int ps3_tb_scale_pct;
#endif

static void present_guest_frame(void)
{
    infamous2_gclock_frame();
    const uint64_t ft_t0 = PS3_FT_ON() ? ps3_ft_now() : 0;
    rsx_backend_present();
    if (ft_t0) ps3_ft_span(PS3_FT_PRESENT, 0, 0, 0, ft_t0, ps3_ft_now());
#if defined(__APPLE__)
    rsx_metal_backend_note_guest_frame();
#endif
    if (g_flip_immediate) cellGcmTickFlip();
    /* This thread increments and guest threads read, so interlocked rather
     * than a volatile ++, which on arm64 is neither atomic nor a fence. */
    InterlockedIncrement(&g_frames_presented);
}

/* cellSysutil.c: the in-game menu's Quit game (REQUEST_EXITGAME, then exit 0). */
extern "C" void ps3_host_quit(const char* why);
/* cellGcmSys.c: the frame clock's idle wait, woken early by a FIFO kick. */
extern "C" void cellGcm_fifo_kick_wait(unsigned ms);
#ifdef PORT_PGO_GEN
extern "C" int __llvm_profile_write_file(void);   /* compiler-rt profile runtime */
#endif
/* The game module (slim split, 2026-11-14): the lifted code is a DLL / .so the engine binds at the
 * start of main (ps3emu/ps3_module.h); PGO training instruments the module, not the engine. */
#include "ps3emu/ps3_module.h"

static DWORD WINAPI frame_clock(LPVOID)
{
    /* PS3_QOS=1: this thread walks the FIFO, encodes the draws and presents --
     * half of every frame. It starts at QOS_CLASS_DEFAULT like any pthread. */
    infamous2_host_qos_self();
    const char* title = getenv("PS3_TITLE");
    if (!title || !*title) title = "ps3recomp";

    int rsx_ok = (rsx_backend_init(WINDOW_WIDTH, WINDOW_HEIGHT, title) == 0);
    fprintf(stderr, "[rsx] %s backend init %s\n", RSX_BACKEND_NAME,
            rsx_ok ? "OK -- window open" : "FAILED");

    /* Pacing knobs (see tools/profile_run.sh for the A/B that sets them). */
    { const char* e = getenv("GCM_FLIP_IMMEDIATE"); g_flip_immediate = e ? atoi(e) : 0; }
    int sema_fast_poll = 0;
    { const char* e = getenv("GCM_SEMA_FASTPOLL"); sema_fast_poll = e ? atoi(e) : 0; }
    /* PS3_FRAMECLOCK_US=<us>: the sleep between two passes of this loop.
     * The FIFO has no doorbell: a put the guest writes, a flip it asks for,
     * waits here until the next pass, so this is also the latency of every
     * one of those. It was a fixed 4 ms; in the New Marais walk the RSX
     * thread sat idle ~4 ms after each flip request; 1 ms gave +2..+5% fps
     * (live A/B 2026-10-22, each stretch against its neighbours; 35.9 vs
     * 33.8 fps over the whole window, mean frame 27.8 vs 29.6 ms).
     * Default 1000; 4000 is the old pacing. Read live (PS3_TOGGLE_FILE). */
    int clock_us_env = 1000;
    { const char* e = getenv("PS3_FRAMECLOCK_US"); if (e && atoi(e) > 0) clock_us_env = atoi(e); }
    /* PS3_HOT_QOS=1: this thread walks the FIFO and encodes every draw; keep
     * it on a performance core alongside the SPU threads (lv2_register.c). */
    { const char* e = getenv("PS3_HOT_QOS");
      if (e && atoi(e)) SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL); }

    /* PS3_TB_SCALE=<percent>: the guest timebase runs at that rate (toolkit
     * sys_timer.c). inFAMOUS 2 takes its frame delta from mftb, so 33 makes
     * it step its simulation as if it ran at 3x the frame rate (slowed down
     * in wall time): a frame-rate-dependence check that does not need a fast
     * machine. Default 100. Read live here (PS3_TOGGLE_FILE). */
    int tb_scale_env = 100;
    { const char* e = getenv("PS3_TB_SCALE"); if (e && atoi(e) > 0) tb_scale_env = atoi(e); }

    ULONGLONG next_tick = GetTickCount64();
    /* (Kenzan) KZ_VBLANK_HZ=<hz>: the vblank tick from a microsecond clock at that rate (the 120 FPS
     * patch wants 120 Hz; 8 ms steps would be 125 Hz and the game 4% fast). Unset: the 16 ms tick. */
    long long vb_period_us = 0, vb_next_us = 0;
    { const char* e = getenv("KZ_VBLANK_HZ"); const int hz = e ? atoi(e) : 0;
      if (hz >= 30 && hz <= 480) { vb_period_us = (1000000LL + hz / 2) / hz;
          fprintf(stderr, "[frameclock] vblank at %d Hz (%lld us)\n", hz, vb_period_us); } }
    struct timespec vb_ts;

    for (;;) {
        /* GCM_SEMA_FASTPOLL=1: while the drain is parked on a semaphore acquire
         * (the SPU lighting label), re-check every 1 ms instead of every 4. */
        {   int us = ps3_toggle_get("PS3_FRAMECLOCK_US", clock_us_env);
            if (us <= 0) us = clock_us_env;
            if (sema_fast_poll && cellGcm_drain_sema_blocked() && us > 1000) us = 1000;
#ifdef _WIN32
            /* The idle wait wakes early on a kick: a guest thread polling the
             * ref register found no fence queued (cellGcm_ref_on_poll), so the
             * guest waits on this thread -- walk the FIFO now instead of after
             * the rest of a ~1-2 ms Sleep. inFAMOUS 2's main thread waits here
             * for its frame-N-1 fence every frame. GCM_FIFO_KICK=0 = the old
             * plain Sleep. */
            cellGcm_fifo_kick_wait((unsigned)((us + 999) / 1000));
#else
            usleep((useconds_t)us);
#endif
        }
        ULONGLONG now = GetTickCount64();
        /* PS3_TOGGLE_FILE=<path>: re-read every 250 ms when it changed, so an
         * A/B flips a knob inside one run (tools/ab_live.sh). Without the
         * variable this is one predictable branch. */
        ps3_toggles_poll();

        {   const int tb = ps3_toggle_get("PS3_TB_SCALE", tb_scale_env);
            if (&ps3_tb_scale_pct && tb > 0 && tb != ps3_tb_scale_pct) ps3_tb_scale_pct = tb; }

        long long vb_now_us = 0;
        if (vb_period_us) { clock_gettime(CLOCK_MONOTONIC, &vb_ts); vb_now_us = (long long)vb_ts.tv_sec * 1000000LL + vb_ts.tv_nsec / 1000;
                            if (!vb_next_us) vb_next_us = vb_now_us; }
        int fired = 0;
        while ((vb_period_us ? (vb_now_us - vb_next_us >= 0) : ((long long)(now - next_tick) >= 0)) && fired < 240) {
            cellGcmTickVBlank();
            cellGcmTickFlip();
            /* Present a pending flip BEFORE draining any further. The flip
             * fires at a get==put frame boundary on the guest thread, so the
             * batch held right now is exactly the completed frame; presenting
             * after the drain races the guest's next-frame writes and shows a
             * mixed one. */
            if (rsx_ok && cellGcm_take_flip_pending()) {
                present_guest_frame();
            }
            /* Drain the FIFO every tick. This is what writes the RSX sync-fence
             * labels the game's per-frame logic blocks on, so it has to keep
             * advancing at 60 Hz even while present() throttles. */
            if (rsx_ok) cellGcm_rsx_process_fifo();
            if (vb_period_us) vb_next_us += vb_period_us; else
            next_tick += 16;             /* ~60 Hz */
            fired++;
        }
        if (fired >= 240) { next_tick = now; if (vb_period_us) vb_next_us = vb_now_us; }   /* fell too far behind -- resync */

        /* Drain at the outer cadence too, not only on the 16 ms tick: titles
         * fence every render pass on a label the drain writes, and one wave of
         * those at 16 ms apiece paces the guest into single-figure frame rates.
         * The real RSX writes them in microseconds. */
        if (rsx_ok) {
            if (cellGcm_take_flip_pending()) {
                present_guest_frame();
            }
            cellGcm_rsx_process_fifo();

            /* Window events at most every 4 ms, as before PS3_FRAMECLOCK_US:
             * a windowed pump is a dispatch_sync to the main thread, and this
             * loop now runs every 1 ms. */
            static ULONGLONG last_pump = 0;
            if (now - last_pump >= 4 || now < last_pump) {
                last_pump = now;
                if (rsx_backend_pump() != 0) {
                    rsx_ok = 0;              /* window closed */
                    /* The red button quits like F1 > Quit game / Cmd+Q, instead
                     * of leaving the process running with no window. */
                    ps3_host_quit("window closed");
                    continue;
                }
            }
            /* Present a flip the drain just reached, now: the drain stops at
             * the flip, so the batch is exactly the finished frame, and the
             * guest is waiting on this flip before it starts the next one.
             * Consuming the pending flag here matters -- this used to present
             * on the flip count changing and leave the flag set, so the check
             * above re-showed the same frame on the next pass with no draws:
             * every frame was presented twice. Before the first flip present
             * freely, so the window is not blank during boot. */
            if (cellGcm_take_flip_pending())
                present_guest_frame();
            else if (cellGcm_flip_request_count() == 0)
                rsx_backend_present();
        }
    }
    return 0;
}

/* ---------------------------------------------------------------------------
 * VFS root
 * -----------------------------------------------------------------------*/
/* Derive the directory holding PS3_GAME from the EBOOT path
 * <root>/PS3_GAME/USRDIR/EBOOT.elf -> <root>. $PS3_VFS_ROOT overrides. */
static char s_vfs_root[1024];

static void derive_vfs_root(const char* eboot)
{
    const char* env = getenv("PS3_VFS_ROOT");
    if (env && *env) { ppu_vfs_root = env; return; }

    strncpy(s_vfs_root, eboot, sizeof s_vfs_root - 1);
    for (char* p = s_vfs_root; *p; p++) if (*p == '\\') *p = '/';
    /* strip EBOOT.elf, USRDIR and PS3_GAME */
    for (int i = 0; i < 3; i++) { char* s = strrchr(s_vfs_root, '/'); if (s) *s = 0; }
    if (!s_vfs_root[0]) strcpy(s_vfs_root, ".");
    ppu_vfs_root = s_vfs_root;
}

/* ---------------------------------------------------------------------------
 * Guest VM
 * -----------------------------------------------------------------------*/
#ifdef _WIN32
/* Demand-paging for the flat VM: reserve the whole 4 GB guest space, which
 * costs no commit, and commit each 64 KB page on first access. Every 32-bit
 * guest offset is then valid, so a garbage guest pointer reads zero instead of
 * killing the process. Faults outside the arena fall through untouched. */
static LONG WINAPI vm_commit_veh(EXCEPTION_POINTERS* ep)
{
    if (ep->ExceptionRecord->ExceptionCode == EXCEPTION_ACCESS_VIOLATION) {
        ULONG_PTR fault = ep->ExceptionRecord->ExceptionInformation[1];
        uintptr_t base  = (uintptr_t)vm_base;
        if (vm_base && fault >= base && fault < base + VM_SIZE) {
            void* page = (void*)(fault & ~(uintptr_t)0xFFFF);
            if (VirtualAlloc(page, 0x10000, MEM_COMMIT, PAGE_READWRITE))
                return EXCEPTION_CONTINUE_EXECUTION;
        }
    }
    return EXCEPTION_CONTINUE_SEARCH;
}
#endif

#ifdef _WIN32
extern "C" uint8_t* ppu_vm_reserve_plain(uint64_t size)
{
    return (uint8_t*)VirtualAlloc(NULL, (SIZE_T)size, MEM_RESERVE, PAGE_READWRITE);
}
#pragma comment(linker, "/alternatename:ppu_vm_reserve=ppu_vm_reserve_plain")
#endif

#ifndef _WIN32
/* The guest window off Windows, 4 GB aligned. The HLE turns a host pointer
 * back into a guest address by truncating it to 32 bits (GUEST_EA; toolkit
 * runtime/memory/vm.h), which holds only while the low 32 bits of vm_base are
 * zero. The old calloc'd arena sat on no such boundary, so every truncated
 * pointer named another guest address, a different one each run: silent
 * corruption, not a crash. Over-map by 4 GB and trim the head and tail (mmap
 * can hand back part of a mapping: no retry, no race). MAP_NORESERVE and lazy
 * backing: only the pages the title touches cost memory, like the Windows
 * reservation's commit on first touch.
 * guest_vm_map_flat is from ThiagoFraga761/infamous2-pc (runtime/memory/
 * guest_vm.h in its work/patches/ps3recomp.patch), MIT License, Copyright (c)
 * 2026 the infamous2-recomp contributors. */
static uint8_t* guest_vm_map_flat(size_t size)
{
    const size_t align = (size_t)0x100000000ull;
    size = (size + 0xFFFFu) & ~(size_t)0xFFFFu;   /* whole pages on any host (<= 64 KB) */
    uint8_t* raw = (uint8_t*)mmap(NULL, size + align, PROT_READ | PROT_WRITE,
                                  MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (raw == MAP_FAILED)
        return NULL;
    uint8_t* base = (uint8_t*)(((uintptr_t)raw + align - 1u) & ~(uintptr_t)(align - 1u));
    uint8_t* end = raw + size + align;
    if (base > raw)
        munmap(raw, (size_t)(base - raw));
    if (end > base + size)
        munmap(base + size, (size_t)(end - (base + size)));
    return base;
}
#endif

static bool alloc_guest_vm(void)
{
#ifdef _WIN32
    AddVectoredExceptionHandler(1, vm_commit_veh);
    vm_base = ppu_vm_reserve(VM_SIZE);   /* = VirtualAlloc(NULL, VM_SIZE, MEM_RESERVE, PAGE_READWRITE) unless PS3_LARGE_PAGES */
    ppu_vm_size = 0;              /* the whole space is backed; no OOB guard needed */
#else
    /* The whole 32-bit guest space + the 64K guard, as on Windows: 4 GB aligned
     * and backed lazily by the OS (guest_vm_map_flat above), so every guest
     * offset is valid and no OOB guard is needed. */
    vm_base = guest_vm_map_flat((size_t)VM_SIZE);
    ppu_vm_size = 0;
#endif
    ps3_module_sync();   /* the game module's copy of vm_base */
    return vm_base != nullptr;
}

/* ---------------------------------------------------------------------------
 * Main
 * -----------------------------------------------------------------------*/
static void harness_guest_caller(uint32_t opd, uint64_t a0, uint64_t a1,
                                 uint64_t a2, uint64_t a3, uint64_t a4,
                                 uint64_t a5, uint64_t a6, uint64_t a7)
{
    ppu_guest_call(opd, a0, a1, a2, a3, a4, a5, a6, a7);
}

#ifdef _WIN32
/* ---------------------------------------------------------------------------
 * Started with no arguments (GitHub #29): Steam's "Add a Non-Steam Game",
 * Playnite, or a shortcut pointed at Infamous2Recomp.exe itself. tools\play.ps1
 * -Attach <this pid> sets the session up exactly as "Play inFAMOUS 2.cmd" does
 * (the game-folder link and file checks, the save backup, the settings, the
 * session folder play\<date-time>) and writes the game's variables to a file.
 * This process takes them, writes its own stdout.log / stderr.log there and
 * runs as that session's game, so its window stays the process the launcher
 * started: Steam Input and the Steam overlay find the game. play.ps1 stays
 * beside it as it does for a game it starts itself (the thread record, the
 * update check, summary.txt when the game ends).
 * Returns 1 with argc/argv set to the ELF, 0 when there is no tools\play.ps1
 * above the exe (the usage line as before), -1 when it failed (already shown).
 * -----------------------------------------------------------------------*/
#include <string>

static std::wstring bare_tail(const wchar_t* path, size_t max_chars)
{
    std::wstring out;
    FILE* f = _wfopen(path, L"rb");
    if (!f) return out;
    std::string s;
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) s.append(buf, n);
    fclose(f);
    if (s.size() > max_chars) s = s.substr(s.size() - max_chars);
    int w = MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), NULL, 0);
    if (w > 0) {
        out.resize((size_t)w);
        MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), &out[0], w);
    }
    return out;
}

static int bare_fail(const wchar_t* what, const wchar_t* log)
{
    std::wstring msg = what;
    std::wstring tail = log ? bare_tail(log, 1500) : std::wstring();
    if (!tail.empty()) msg += L"\n\nLast lines of the launcher's output:\n\n" + tail;
    MessageBoxW(NULL, msg.c_str(), L"inFAMOUS 2", MB_OK | MB_ICONWARNING);
    return -1;
}

static int bare_start(int* argc, char*** argv)
{
    static wchar_t exe[2048], root[2048], ps1[2100];
    DWORD n = GetModuleFileNameW(NULL, exe, 2048);
    if (!n || n >= 2048) return 0;
    wcscpy(root, exe);
    int found = 0;
    for (int up = 0; up < 6 && !found; up++) {          /* <port>\work\port\build\Infamous2Recomp.exe */
        wchar_t* s = wcsrchr(root, L'\\');
        if (!s) break;
        *s = 0;
        _snwprintf(ps1, 2099, L"%ls\\tools\\play.ps1", root);
        ps1[2099] = 0;
        found = GetFileAttributesW(ps1) != INVALID_FILE_ATTRIBUTES;
    }
    if (!found) return 0;

    wchar_t tmp[MAX_PATH + 1], envf[MAX_PATH + 64], logf[MAX_PATH + 64];
    if (!GetTempPathW(MAX_PATH + 1, tmp)) return bare_fail(L"Windows has no temporary folder for the game.", NULL);
    const DWORD pid = GetCurrentProcessId();
    _snwprintf(envf, MAX_PATH + 63, L"%lsinf2-start-%lu.env", tmp, (unsigned long)pid);
    _snwprintf(logf, MAX_PATH + 63, L"%lsinf2-start-%lu.log", tmp, (unsigned long)pid);
    envf[MAX_PATH + 63] = logf[MAX_PATH + 63] = 0;
    DeleteFileW(envf);

    SECURITY_ATTRIBUTES sa = { sizeof sa, NULL, TRUE };
    HANDLE hlog = CreateFileW(logf, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, &sa,
                              CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    static wchar_t cmd[8192];
    _snwprintf(cmd, 8191, L"powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%ls\" -Attach %lu -EnvFile \"%ls\"",
               ps1, (unsigned long)pid, envf);
    cmd[8191] = 0;
    STARTUPINFOW si;
    ZeroMemory(&si, sizeof si);
    si.cb = sizeof si;
    si.dwFlags = STARTF_USESHOWWINDOW | (hlog != INVALID_HANDLE_VALUE ? STARTF_USESTDHANDLES : 0);
    si.wShowWindow = SW_HIDE;
    si.hStdInput = NULL;
    si.hStdOutput = si.hStdError = hlog;
    PROCESS_INFORMATION pi;
    if (!CreateProcessW(NULL, cmd, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, root, &si, &pi)) {
        if (hlog != INVALID_HANDLE_VALUE) CloseHandle(hlog);
        return bare_fail(L"The game could not start Windows PowerShell to set itself up.\n"
                         L"Start it with \"Play inFAMOUS 2.cmd\" in the port's folder instead.", NULL);
    }
    if (hlog != INVALID_HANDLE_VALUE) CloseHandle(hlog);

    /* play.ps1 writes <file>.part and renames it, so the file is whole once it exists. It may wait first:
     * the game lock (a test run), the save backup. Ten minutes, then give up. */
    const ULONGLONG t0 = GetTickCount64();
    int ok = 0;
    for (;;) {
        if (GetFileAttributesW(envf) != INVALID_FILE_ATTRIBUTES) { ok = 1; break; }
        if (WaitForSingleObject(pi.hProcess, 100) == WAIT_OBJECT_0) {
            ok = GetFileAttributesW(envf) != INVALID_FILE_ATTRIBUTES;
            break;
        }
        if (GetTickCount64() - t0 > 600000) break;
    }
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);   /* play.ps1 goes on beside the game */
    if (!ok)
        return bare_fail(L"The game could not set itself up (tools\\play.ps1 stopped).", logf);

    FILE* f = _wfopen(envf, L"rb");
    if (!f) return bare_fail(L"The game could not read its settings from the launcher.", logf);
    std::string all;
    {
        char buf[4096];
        size_t r;
        while ((r = fread(buf, 1, sizeof buf, f)) > 0) all.append(buf, r);
        fclose(f);
    }
    DeleteFileW(envf);
    if (all.size() >= 3 && (unsigned char)all[0] == 0xEF && (unsigned char)all[1] == 0xBB && (unsigned char)all[2] == 0xBF)
        all.erase(0, 3);

    std::wstring elf, dir, so, se;
    int ended = 0;
    size_t pos = 0;
    while (pos < all.size()) {
        size_t e = all.find('\n', pos);
        if (e == std::string::npos) e = all.size();
        std::string line = all.substr(pos, e - pos);
        pos = e + 1;
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (line == "END") { ended = 1; break; }
        size_t eq = line.find('=');
        if (eq == std::string::npos || eq == 0) continue;
        int wn = MultiByteToWideChar(CP_UTF8, 0, line.data(), (int)line.size(), NULL, 0);
        if (wn <= 0) continue;
        std::wstring w((size_t)wn, L'\0');
        MultiByteToWideChar(CP_UTF8, 0, line.data(), (int)line.size(), &w[0], wn);
        size_t weq = w.find(L'=');
        std::wstring k = w.substr(0, weq), v = w.substr(weq + 1);
        if (k == L"PS3_START_ELF") elf = v;
        else if (k == L"PS3_START_DIR") dir = v;
        else if (k == L"PS3_START_STDOUT") so = v;
        else if (k == L"PS3_START_STDERR") se = v;
        else {
            SetEnvironmentVariableW(k.c_str(), v.c_str());   /* the OS block (SDL reads it) ... */
            _wputenv_s(k.c_str(), v.c_str());                /* ... and the C runtime's (getenv) */
        }
    }
    if (!ended || elf.empty())
        return bare_fail(L"The game's settings from the launcher were incomplete.", logf);
    if (!so.empty()) _wfreopen(so.c_str(), L"w", stdout);
    if (!se.empty()) _wfreopen(se.c_str(), L"w", stderr);
    if (!dir.empty()) SetCurrentDirectoryW(dir.c_str());
    DeleteFileW(logf);

    /* argv[1] in the code page the C runtime gives every argument (what play.ps1's start passes) */
    static char elf_a[4096];
    if (!WideCharToMultiByte(CP_ACP, 0, elf.c_str(), -1, elf_a, sizeof elf_a, NULL, NULL))
        return bare_fail(L"The game's file path could not be read.", NULL);
    static char* nargv[3];
    nargv[0] = (*argv)[0];
    nargv[1] = elf_a;
    nargv[2] = NULL;
    *argc = 2;
    *argv = nargv;
    printf("[start] no arguments: set up by tools\\play.ps1 -Attach (session %ls)\n", dir.c_str());
    return 1;
}
#endif

extern "C" void infamous2_gpu_light_install(void);   /* gpu_light.c */
extern "C" void infamous2_gpu_ssao_install(void);    /* gpu_ssao.c */
extern "C" void infamous2_edge_watch_install(void);  /* edge_watch.c (PS3_EDGE_WATCH) */

int main(int argc, char** argv)
{
#ifdef _WIN32
    /* No ELF given: a launcher started the exe itself (bare_start above, GitHub #29). */
    if (argc < 2 && bare_start(&argc, &argv) < 0)
        return 2;
#endif
    if (argc < 2) {
        printf("usage: %s <PPU ELF>\n", argv[0]);
        return 2;
    }
    /* The game module first: the lifted code's own registrations (SPU images, firmware modules,
     * hooks) run here, and everything below finds them. A missing or mismatched module is a
     * message and an exit (PS3_GAME_MODULE=<path>: another module file). */
    if (ps3_module_attach() != 0)
        return 3;
    /* PS3RECOMP_ENG_GPU_LIGHT: the SPU light job's pixel work moves to the GPU. */
    infamous2_gpu_light_install();
    infamous2_gpu_ssao_install();
    infamous2_edge_watch_install();

#ifdef _WIN32
#pragma comment(lib, "winmm.lib")
    /* 1 ms timer resolution. The default granularity is about 15.6 ms, which
     * inflates every shorter wait the title makes and throttles the whole
     * thing. POSIX timers are already fine-grained. */
    timeBeginPeriod(1);
    setvbuf(stdout, NULL, _IONBF, 0);   /* unbuffered: do not lose prints on a kill */
    /* PS3_SAMPLE=<ms>: the runtime's sampling profiler (ppu_loader.cpp), which
     * only the toolkit's boot harness ever started. No-op when unset. */
    { extern void ps3_sampler_start(void); ps3_sampler_start(); }
#endif

    printf("=== ps3recomp game runner ===\n");

    /* Before any thread exists: keep macOS from App-Napping or timer-throttling
     * the run, and with PS3_QOS=1 put the main thread (the guest's own when
     * headless) at USER_INTERACTIVE; the PPU/SPU threads then do the same at
     * their entry. */
    infamous2_host_begin_activity();
    infamous2_host_qos_self();
#if defined(__APPLE__)
    printf("[host] App Nap off; thread QoS %s\n", infamous2_host_qos_enabled()
           ? "USER_INTERACTIVE for PPU/SPU/RSX threads (PS3_QOS=1)"
           : "left at the system default (PS3_QOS=1 raises it)");
#endif

    if (!alloc_guest_vm()) {
        fprintf(stderr, "ERROR: could not allocate the guest address space\n");
        return 1;
    }

    uint32_t entry = ppu_load_elf(argv[1]);
    if (!entry) {
        fprintf(stderr, "ERROR: could not load %s\n", argv[1]);
        return 1;
    }
    infamous2_apply_patches(argv[1]);   /* community patches from the player's RPCS3 patch.yml */
    infamous2_apply_aspect(argv[1]);    /* PS3_ASPECT: the cameras' aspect ratio, hor+ */

    derive_vfs_root(argv[1]);
    printf("[boot] VFS root: %s\n", ppu_vfs_root);

    /* Real title id, from the game's own PARAM.SFO. Without it cellGame keeps
     * its BLES00000 placeholder and builds every /dev_hdd0/game/<id> path from
     * that. Same lookup as runtime/ppu/tests/boot_main.cpp. */
    {
        static const char* const kSfoLayouts[] = {
            "%s/PS3_GAME/PARAM.SFO",   /* vfs root = disc root      */
            "%s/../PARAM.SFO",         /* vfs root = .../USRDIR     */
            "%s/PARAM.SFO",            /* vfs root = the game dir   */
        };
        char sfo[1100];
        int found = 0;
        for (size_t i = 0; i < sizeof kSfoLayouts / sizeof kSfoLayouts[0]; i++) {
            snprintf(sfo, sizeof sfo, kSfoLayouts[i], ppu_vfs_root);
            FILE* f = fopen(sfo, "rb");
            if (f) { fclose(f); found = 1; break; }
        }
        if (!found)
            snprintf(sfo, sizeof sfo, "%s/PS3_GAME/PARAM.SFO", ppu_vfs_root);
        cellGame_init_from_paramsfo(sfo);
    }

    ppu_recomp_register();   /* lifted function table -> address map */
    ps3_load_prx_modules();  /* this game's lifted system PRX, if it has any */
    ppu_hle_init();          /* firmware import NID -> HLE handlers */
    ppu_sysprx_register();   /* boot-critical CRT */
    ppu_fs_register();       /* cellFs over the game directory */
    lv2_init_syscalls();     /* the lv2 syscall table */

    /* Install the guest-callback hook, then start the frame clock. It no-ops
     * until the title registers its vblank and flip handlers during init. */
    g_ps3_guest_caller = harness_guest_caller;
    CreateThread(NULL, 4u * 1024 * 1024, frame_clock, NULL, 0, NULL);
    /* PGO training (an instrumented game module, or an instrumented single exe): the scripted runs
     * end in a kill, which skips the profile runtime's atexit writer. PS3_PGO_DUMP_AT=<s>[,<s>...]:
     * write the counters at those times (each write replaces the file). */
#ifndef PORT_PGO_GEN
    if (getenv("PS3_PGO_DUMP_AT") && ps3_module_current() && ps3_module_current()->profile_dump)
#endif
    { static auto pgo_dump = [](LPVOID) -> DWORD {
          const char* e = getenv("PS3_PGO_DUMP_AT");
          ULONGLONG t0 = GetTickCount64();
          for (const char* p = e; p && *p;) {
              char* q; double s = strtod(p, &q); if (q == p) break;
              ULONGLONG due = t0 + (ULONGLONG)(s * 1000);
              while (GetTickCount64() < due) Sleep(200);
#ifdef PORT_PGO_GEN
              int rc = __llvm_profile_write_file();
#else
              int rc = ps3_module_profile_dump();
#endif
              fprintf(stderr, "[pgo] profile written at %.0f s (rc=%d)\n", s, rc);
              p = *q == ',' ? q + 1 : q;
          }
          return 0; };
      if (getenv("PS3_PGO_DUMP_AT")) CreateThread(NULL, 0, pgo_dump, NULL, 0, NULL); }
    ps3_module_sync();   /* (again, before any lifted code runs) the module's copies of the engine's pointers */
    printf("\n[boot] dispatching entry OPD 0x%08X (stack top 0x%08X)\n\n",
           entry, STACK_TOP);

#if defined(__APPLE__)
    /* AppKit only runs on the process's main thread: the backend creates the
     * window and pumps its events with dispatch_sync onto the main queue. With
     * the guest on the main thread nothing ever serviced that queue, so the
     * frame clock blocked in its first event pump, the FIFO stopped draining
     * and the title spun for ever on a fence -- a grey window that never drew.
     * Windowed, the guest runs on its own thread (the 256 MB host stack every
     * guest thread gets) and the main thread turns the run loop, which drains
     * the main queue. Headless never touches AppKit and keeps the old layout. */
    int rc;
    if (!getenv("HEADLESS") && !getenv("PS3RECOMP_METAL_HEADLESS")) {
        struct boot_args { uint32_t entry; int rc; volatile int done; };
        static boot_args ba;
        ba.entry = entry; ba.rc = 0; ba.done = 0;
        pthread_attr_t attr;
        pthread_attr_init(&attr);
        pthread_attr_setstacksize(&attr, 256u * 1024u * 1024u);
        pthread_t th;
        auto body = [](void* a) -> void* {
            boot_args* b = (boot_args*)a;
            infamous2_host_qos_self();   /* the guest's main thread */
            b->rc = ppu_run(b->entry, STACK_TOP);
            __atomic_store_n(&b->done, 1, __ATOMIC_RELEASE);
            CFRunLoopStop(CFRunLoopGetMain());
            return nullptr;
        };
        if (pthread_create(&th, &attr, body, &ba) != 0) {
            fprintf(stderr, "[boot] could not start the guest thread\n");
            return 1;
        }
        pthread_attr_destroy(&attr);
        while (!__atomic_load_n(&ba.done, __ATOMIC_ACQUIRE))
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, false);
        pthread_join(th, nullptr);
        rc = ba.rc;
    } else {
        rc = ppu_run(entry, STACK_TOP);
    }
#else
    int rc = ppu_run(entry, STACK_TOP);
#endif
    printf("\n[boot] ppu_run returned %d (entry function unwound)\n", rc);

    /* A title that called sys_process_exit never reaches this line: that path
     * ends in the host exit(). Getting here means the entry function returned
     * instead, so hand back the status the guest published if it published
     * one. Returning a hardcoded 0 would report success for a run that never
     * got anywhere. */
    return g_sys_process_exit_called ? (int)g_sys_process_exit_code : 0;
}
