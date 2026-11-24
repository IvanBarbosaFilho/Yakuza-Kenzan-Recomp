/* Ryu ga Gotoku Kenzan! (BLJM-60064) on the LLE engine: the port's hooks and diagnostics. */
#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "ps3emu/ps3types.h"
#include "ps3emu/error_codes.h"
#include "sys_ppu_thread.h"
extern "C" uint8_t* vm_base;

#ifndef _WIN32
#include <pthread.h>
#include <unistd.h>
#endif

/* The game's own PRX modules load through the engine's PRX loader: pxd_shader_ps3.ppu.sprx is
 * lifted into lle/pxd_shader (tools/lift_firmware_module.py) and registered by file name, like the
 * firmware. Nothing to load by hand. */
extern "C" void ps3_load_prx_modules(void)
{
}

/* ---------------------------------------------------------------------------
 * KZ_DUMP_AT=<s>: every <s> seconds, print each guest thread's PC, LR, SP and
 * a guest call chain walked through the PPC64 stack back chain ([sp] = caller's
 * sp, [caller sp + 16] = saved LR). Linux stand-in for boot_main's Windows-only
 * hang watchdog: answers "where is the main thread stuck" without ptrace.
 * Values come from the thread's ppu_context, so a thread inside lifted code
 * shows where it last synced (calls, syscalls) -- close enough to name the loop.
 * -------------------------------------------------------------------------*/
extern "C" __attribute__((weak)) ppu_context* volatile g_ppu_main_ctx;   /* absent in the LLE engine: main is a regular guest thread */
extern "C" uint8_t* vm_base;
static uint64_t vm_read64(uint32_t ea)
{
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) v = (v << 8) | vm_base[(size_t)ea + i];
    return v;
}

/* The title's IO base: its GCM context pointer is at 0x104150F0 (EBOOT data); context->begin is
 * IO offset 0x1000. */
static uint32_t kz_iob(void)
{
    const uint32_t gctx = (uint32_t)(vm_read64(0x104150F0u) >> 32);
    return gctx ? ((uint32_t)(vm_read64(gctx) >> 32) & ~0xFFFFFu) : 0x18E00000u;
}

static void kz_dump_ctx(const char* name, int tid, const ppu_context* c, uint32_t prof_pc)
{
    uint32_t sp = (uint32_t)c->gpr[1];
    fprintf(stderr, "[kzdump] tid=%-2d %-22s cia=%08X lr=%08X sp=%08X prof=%08X r3=%08X r4=%08X\n",
            tid, name, (uint32_t)c->cia, (uint32_t)c->lr, sp, prof_pc,
            (uint32_t)c->gpr[3], (uint32_t)c->gpr[4]);
    fprintf(stderr, "[kzdump]        chain:");
    for (int d = 0; d < 16 && sp && sp < 0xF0000000u; d++) {
        uint32_t back = (uint32_t)vm_read64(sp);
        if (!back || back <= sp) break;
        uint32_t lr = (uint32_t)vm_read64(back + 16);
        fprintf(stderr, " %08X", lr);
        sp = back;
    }
    fprintf(stderr, "\n");
}

#ifndef _WIN32
static void* kz_dump_thread(void* arg)
{
    unsigned every = (unsigned)(uintptr_t)arg;
    for (int round = 1;; round++) {
        if (every) sleep(every);   /* 0: one round now (the KZ_BT_TRIGGER file) */
        fprintf(stderr, "[kzdump] ---- %u s ----\n", every * round);
        {
            /* GCM: control put/get (0x20002000 in this runtime), the default context
             * (ctx_out 0x104150F0: begin/end/current), labels 0x40/0x41, and the
             * frame fence func_005C6E08 polls: label ptr at TOC-0x3DC8, expected at
             * [[TOC-0x3E6C]] (TOC 0xBE0328). */
            auto r32 = [](uint32_t ea) { return (uint32_t)(vm_read64(ea) >> 32); };
            /* layout-free: labels/control from the engine's inject base, the IO base from the
             * title's own GCM context (its pointer lives at 0x104150F0 in the EBOOT's data). */
            const uint32_t inj = ppu_hle_inject_base;
            const uint32_t gctx = r32(0x104150F0u);
            uint32_t lab = r32(0xBE0328u - 0x3DC8u), exp_p = r32(0xBE0328u - 0x3E6Cu);
            fprintf(stderr, "[kzdump] gcm put=%08X get=%08X ctx@0F800000 cb=%08X begin=%08X end=%08X cur=%08X "
                    "label40=%08X label41=%08X fence@%08X=%08X expect@%08X=%08X\n",
                    r32((inj + 0x2000u)), r32((inj + 0x2004u)), r32(gctx + 0xCu), r32(gctx), r32(gctx + 4u), r32(gctx + 8u),
                    r32((inj + 0x400u)), r32((inj + 0x410u)), lab, lab ? r32(lab) : 0,
                    exp_p, exp_p ? r32(exp_p) : 0);
            /* The words at get (IO offset 0 = EA 0x18E00000 for this title's ioAddr). */
            uint32_t get = r32((inj + 0x2004u));
            if (get < 0x600000u) {
                fprintf(stderr, "[kzdump] fifo@%05X:", get);
                for (int w = -4; w < 20; w++) fprintf(stderr, " %08X", r32(kz_iob() + get + w * 4));
                fprintf(stderr, "\n");
            }
            /* The CALL target at io 0x1000 (0x00F00002 -> io 0xF00000 = EA 0x19D00000). */
            fprintf(stderr, "[kzdump] io1000: %08X  call@19D00000:", r32((kz_iob() + 0x1000u)));
            for (int w = 0; w < 16; w++) fprintf(stderr, " %08X", r32(0x19D00000u + w * 4));
            fprintf(stderr, "\n");
            /* io 0..0x1000 (EA 0x18E00000): the target of the CALL 0 at io 0x1064. */
            uint32_t nz = 0, first_nz = 0xFFFFFFFFu;
            for (uint32_t o = 0; o < 0x1000u; o += 4)
                if (r32(kz_iob() + o)) { nz++; if (first_nz == 0xFFFFFFFFu) first_nz = o; }
            fprintf(stderr, "[kzdump] io0..1000: %u nonzero words, first at %X; io1060:", nz, first_nz);
            for (int w = 0; w < 8; w++) fprintf(stderr, " %08X", r32((kz_iob() + 0x1058u) + w * 4));
            fprintf(stderr, "\n");
        }
        if (getenv("KZ_JC_DUMP")) {   /* the job chain the main thread runs before its EventFlagWait */
            fprintf(stderr, "[kzjc] cmds@18C86C90:");
            for (int w = 0; w < 16; w++) fprintf(stderr, " %016llX", (unsigned long long)vm_read64(0x18C86C90u + w * 8));
            fprintf(stderr, "\n[kzjc] guard@18C86980:");
            for (int w = 0; w < 8; w++) fprintf(stderr, " %08X", (uint32_t)(vm_read64(0x18C86980u + w * 4) >> 32));
            fprintf(stderr, "\n[kzjc] jc@18C86B80:");
            for (int w = 0; w < 16; w++) fprintf(stderr, " %08X", (uint32_t)(vm_read64(0x18C86B80u + w * 4) >> 32));
            fprintf(stderr, "\n");
        }
        if (round == 2 && getenv("KZ_MEMSCAN")) {
            /* Every place in IO-mapped memory that would release semaphore 3 (label base + 0x30):
             * NV406E OFFSET 0x30 then RELEASE, or NV4097 back-end SET_SEMAPHORE_OFFSET 0x30. */
            int hits = 0;
            for (uint32_t ea = kz_iob(); ea < (kz_iob() + 0x1200000u) && hits < 40; ea += 4) {
                const uint32_t w0 = (uint32_t)(vm_read64(ea) >> 32), w1 = (uint32_t)(vm_read64(ea + 4) >> 32);
                if (((w0 & 0xFFFF) == 0x0064 || (w0 & 0x1FFFF) == 0x1D6C) && (w0 >> 18) == 1 && w1 == 0x30) {
                    fprintf(stderr, "[kzscan] 0x%08X (io %06X):", ea, ea - kz_iob());
                    for (int k = 0; k < 6; k++) fprintf(stderr, " %08X", (uint32_t)(vm_read64(ea + k * 4) >> 32));
                    fprintf(stderr, "\n"); hits++;
                }
            }
            fprintf(stderr, "[kzscan] %d hit(s)\n", hits);
        }
        if (round == 1 && getenv("KZ_RING_DUMP")) {
            fprintf(stderr, "[kzring]");
            for (uint32_t o = 0x1000; o < 0x11D0; o += 4)
                fprintf(stderr, "%s%04X:%08X", (o & 0x1F) ? " " : "\n[kzring] ", o,
                        (uint32_t)(vm_read64(kz_iob() + o) >> 32));
            fprintf(stderr, "\n");
        }
        if (&g_ppu_main_ctx && g_ppu_main_ctx) {
            kz_dump_ctx("(main)", 0, g_ppu_main_ctx, 0);
            /* func_005C6E08's wait: while (*(*r30) + 2 <= r30[1]) usleep(30). */
            uint32_t r30 = (uint32_t)g_ppu_main_ctx->gpr[30];
            auto rd = [](uint32_t ea) { return (uint32_t)(vm_read64(ea) >> 32); };
            uint32_t pc = r30 ? rd(r30) : 0;
            fprintf(stderr, "[kzdump] wait r30=%08X counter@%08X=%08X target=%08X r29=%08X r31=%08X\n",
                    r30, pc, pc ? rd(pc) : 0, r30 ? rd(r30 + 4) : 0,
                    (uint32_t)g_ppu_main_ctx->gpr[29], (uint32_t)g_ppu_main_ctx->gpr[31]);
        }
        for (int i = 0; i < PPU_THREAD_MAX; i++) {
            ppu_thread_info* t = &g_ppu_threads[i];
            if (t->state != 1 && t->state != 2) continue;
            kz_dump_ctx(t->name, i + 1, &t->ctx, t->prof_pc);
        }
        fflush(stderr);
        if (!every) break;
    }
    return nullptr;
}

/* KZ_FLIPMARK=1: once a second, "[kzflips] <s> s flips=<n>" -- the guest flip count the
 * PAD_CLOCK=flips schedule of PAD_SCRIPT runs on, lined up with the [fs] opens around it. */
extern "C" unsigned cellGcm_flip_request_count(void);
static void* kz_flipmark_thread(void*)
{
    for (unsigned s = 1;; s++) {
        sleep(1);
        fprintf(stderr, "[kzflips] %u s flips=%u\n", s, cellGcm_flip_request_count());
        /* the main thread's stack pointer: it ended up 256 KB down a callback stack in the
         * 0xD0 region (crash at the first cutscene); say when it leaves the main stack */
        if (&g_ppu_main_ctx && g_ppu_main_ctx) {
            static uint32_t last_hi = 0;
            const uint32_t r1 = (uint32_t)g_ppu_main_ctx->gpr[1];
            if ((r1 >> 24) != last_hi || s % 30 == 0)
                fprintf(stderr, "[kzr1] %u s main r1=%016llX cia=%08X lr=%08X\n", s,
                        (unsigned long long)g_ppu_main_ctx->gpr[1], (uint32_t)g_ppu_main_ctx->cia, (uint32_t)g_ppu_main_ctx->lr);
            last_hi = r1 >> 24;
        }
    }
    return nullptr;
}
__attribute__((constructor)) static void kz_flipmark_start(void)
{
    if (!getenv("KZ_FLIPMARK")) return;
    pthread_t th;
    if (pthread_create(&th, nullptr, kz_flipmark_thread, nullptr) == 0) pthread_detach(th);
}

__attribute__((constructor)) static void kz_dump_start(void)
{
    const char* e = getenv("KZ_DUMP_AT");
    unsigned s = e ? (unsigned)atoi(e) : 0;
    if (!s) return;
    pthread_t th;
    if (pthread_create(&th, nullptr, kz_dump_thread, (void*)(uintptr_t)s) == 0) pthread_detach(th);
}
#endif

/* ---------------------------------------------------------------------------
 * KZ_BT_AT=<s>: once, <s> seconds in, every host thread of the process prints
 * its native backtrace (signal SIGRTMIN+5 -> backtrace()). Lifted functions are
 * named func_<guest address>, so the native stack IS the guest call chain,
 * innermost first. Addresses are printed relative to the executable's load base;
 * resolve them with tools/kz_symbolize.sh. No ptrace needed.
 * -------------------------------------------------------------------------*/
#ifndef _WIN32
#include <execinfo.h>
#include <signal.h>
#include <dirent.h>
#include <sys/uio.h>
#include <link.h>
#include <dlfcn.h>
#include <sys/syscall.h>

static uintptr_t s_kz_exe_base;
static int kz_phdr_cb(struct dl_phdr_info* info, size_t, void*)
{
    if (!info->dlpi_name || !info->dlpi_name[0]) { s_kz_exe_base = info->dlpi_addr; return 1; }
    return 0;
}

static void kz_bt_handler(int)
{
    void* fr[48];
    int n = backtrace(fr, 48);
    char line[1024];
    int len = snprintf(line, sizeof line, "[kzbt] tid=%ld comm=", (long)syscall(SYS_gettid));
    char comm[32] = {0};
    pthread_getname_np(pthread_self(), comm, sizeof comm);
    len += snprintf(line + len, sizeof line - len, "%s:", comm);
    /* Each frame as <file tag><offset>: "e" = the engine exe, "m" = the game module (.so), so
     * tools/kz_symbolize.py resolves it against the right binary. */
    for (int i = 1; i < n && len < (int)sizeof line - 32; i++) {
        Dl_info di;
        if (dladdr(fr[i], &di) && di.dli_fbase) {
            const char* nm = di.dli_fname ? strrchr(di.dli_fname, '/') : nullptr;
            const unsigned long off = (unsigned long)((uintptr_t)fr[i] - (uintptr_t)di.dli_fbase);
            if (nm && strstr(nm, "Infamous2Game")) len += snprintf(line + len, sizeof line - len, " m%lx", off);
            else if (nm && strstr(nm, ".so"))       len += snprintf(line + len, sizeof line - len, " %s+%lx%s%s", nm + 1, off,
                                                                    di.dli_sname ? ":" : "", di.dli_sname ? di.dli_sname : "");
            else                                    len += snprintf(line + len, sizeof line - len, " e%lx", off);
        } else {
            len += snprintf(line + len, sizeof line - len, " ?%lx", (unsigned long)(uintptr_t)fr[i]);
        }
    }
    line[len++] = '\n';
    if (write(2, line, len) < 0) {}
}

/* KZ_BT_EVERY=<s>: repeat the dump every <s> seconds after the first (a session the user drives:
 * whatever froze, the last round before they closed the window shows where). */
static void* kz_bt_thread(void* arg)
{
    sleep((unsigned)(uintptr_t)arg);
    dl_iterate_phdr(kz_phdr_cb, nullptr);
    struct sigaction sa; memset(&sa, 0, sizeof sa);
    sa.sa_handler = kz_bt_handler;
    sigaction(SIGRTMIN + 5, &sa, nullptr);
    const char* ev = getenv("KZ_BT_EVERY");
    const unsigned every = ev ? (unsigned)atoi(ev) : 0;
    for (unsigned round = 1;; round++) {
        fprintf(stderr, "[kzbt-round] %u\n", round);
        pid_t pid = getpid(), self = (pid_t)syscall(SYS_gettid);
        DIR* d = opendir("/proc/self/task");
        if (!d) return nullptr;
        while (struct dirent* e = readdir(d)) {
            pid_t t = (pid_t)atoi(e->d_name);
            if (t <= 0 || t == self) continue;
            syscall(SYS_tgkill, pid, t, SIGRTMIN + 5);
            usleep(20000);
        }
        closedir(d);
        if (!every) break;
        sleep(every);
    }
    return nullptr;
}

/* KZ_BT_TRIGGER=<file>: the handler is armed at startup, and every time <file> appears
 * (touch it while the game is frozen) it is removed and every thread prints its stack. */
static void* kz_dump_thread(void* arg);
static void kz_bt_all_threads(void)
{
    fprintf(stderr, "[kzbt-round] trigger\n");
    pid_t pid = getpid(), self = (pid_t)syscall(SYS_gettid);
    DIR* d = opendir("/proc/self/task");
    if (!d) return;
    while (struct dirent* e = readdir(d)) {
        pid_t t = (pid_t)atoi(e->d_name);
        if (t <= 0 || t == self) continue;
        syscall(SYS_tgkill, pid, t, SIGRTMIN + 5);
        usleep(20000);
    }
    closedir(d);
}
static void* kz_bt_trigger_thread(void* arg)
{
    const char* path = (const char*)arg;
    pthread_setname_np(pthread_self(), "kz bt trigger");
    for (;;) {
        sleep(1);
        if (access(path, F_OK) == 0) { unlink(path); kz_bt_all_threads(); sleep(1); kz_dump_thread(nullptr); }
    }
    return nullptr;
}

__attribute__((constructor)) static void kz_bt_start(void)
{
    if (const char* tp = getenv("KZ_BT_TRIGGER")) {
        if (*tp) {
            dl_iterate_phdr(kz_phdr_cb, nullptr);
            struct sigaction sa; memset(&sa, 0, sizeof sa);
            sa.sa_handler = kz_bt_handler;
            sigaction(SIGRTMIN + 5, &sa, nullptr);
            unlink(tp);
            pthread_t th;
            if (pthread_create(&th, nullptr, kz_bt_trigger_thread, (void*)tp) == 0) pthread_detach(th);
        }
    }
    const char* e = getenv("KZ_BT_AT");
    unsigned s = e ? (unsigned)atoi(e) : 0;
    if (!s) return;
    pthread_t th;
    if (pthread_create(&th, nullptr, kz_bt_thread, (void*)(uintptr_t)s) == 0) pthread_detach(th);
}
#endif

/* ---------------------------------------------------------------------------
 * KZ_WATCH_EA=<hex guest EA>: a hardware write watchpoint (perf breakpoint, 8 bytes) on
 * that guest address, inherited by every thread the main thread creates afterwards. Each
 * write is reported (thread, value, host backtrace with lifted function names) -- writes
 * by the main thread only when the value looks like the (sin, cos) float pair that
 * clobbered func_0089D804's count slot at 0x0FEFF4D8. Armed from ppu_run (engine).
 * -------------------------------------------------------------------------*/
#ifndef _WIN32
#include <errno.h>
#include <linux/perf_event.h>
#include <linux/hw_breakpoint.h>
static uint64_t s_kzw_host; static uint32_t s_kzw_ea; static pid_t s_kzw_main;
static volatile int s_kzw_n;
static void kz_watch_trap(int, siginfo_t* si, void*)
{
    if (!s_kzw_host) return;
    const uint8_t* p = (const uint8_t*)(uintptr_t)s_kzw_host;
    const uint32_t hi = ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
    const uint32_t lo = ((uint32_t)p[4] << 24) | ((uint32_t)p[5] << 16) | ((uint32_t)p[6] << 8) | p[7];
    const pid_t tid = (pid_t)syscall(SYS_gettid);
    /* only the clobber's shape: (small float, ~1.0f) -- the (sin, cos) of a small angle */
    if ((lo & 0xFF000000u) != 0x3F000000u || hi < 0x30000000u || hi >= 0x3C000000u) return;
    void* fr[40]; int n = backtrace(fr, 40);
    /* one report per distinct (thread kind, 3 innermost frames) */
    static uintptr_t seen[256][4]; static int nseen;
    uintptr_t key[4] = { (uintptr_t)(tid == s_kzw_main), n > 1 ? (uintptr_t)fr[1] : 0, n > 2 ? (uintptr_t)fr[2] : 0, n > 3 ? (uintptr_t)fr[3] : 0 };
    for (int i = 0; i < nseen; i++) if (!memcmp(seen[i], key, sizeof key)) return;
    if (nseen >= 256) return;
    memcpy(seen[nseen++], key, sizeof key);
    char line[1400]; int len = snprintf(line, sizeof line, "[kzbt] tid=%ld comm=kzwatch-%s-%08X-%08X:",
                                        (long)tid, tid == s_kzw_main ? "main" : "other", hi, lo);
    for (int i = 1; i < n && len < (int)sizeof line - 40; i++) {
        Dl_info di;
        if (dladdr(fr[i], &di) && di.dli_fbase) {
            const char* nm = di.dli_fname ? strrchr(di.dli_fname, '/') : nullptr;
            const unsigned long off = (unsigned long)((uintptr_t)fr[i] - (uintptr_t)di.dli_fbase);
            if (nm && strstr(nm, "Infamous2Game")) len += snprintf(line + len, sizeof line - len, " m%lx", off);
            else if (nm && strstr(nm, ".so"))       len += snprintf(line + len, sizeof line - len, " %s+%lx", nm + 1, off);
            else                                    len += snprintf(line + len, sizeof line - len, " e%lx", off);
        }
    }
    line[len++] = '\n';
    if (write(2, line, len) < 0) {}
    (void)si;
}
/* KZ_WATCHR_TRIGGER=<file>: while the game runs, write "<hex guest EA> [seconds]" (or "pad": the
 * game's copy of port 0's digital buttons, **(u32*)0x00BDB7D8 + 0x198) to the file and every thread
 * gets a hardware read/write breakpoint (perf, 2 bytes) on it for that long (default 8 s). Each
 * distinct host call chain that touches it is reported once as a [kzbt] line (tools/kz_symbolize.py
 * names the lifted functions), with the value it saw. */
static uint64_t s_kzr_host; static volatile int s_kzr_on; static uint32_t s_kzr_mask; static unsigned long s_kzr_hits; static int s_kzr_exec;
extern "C" uint32_t ppu_vm_size;                            /* guest memory size (0 = 4 GB) */
extern "C" __thread void* g_active_ctx;                       /* the running guest thread's ppu_context */
extern "C" void ppu_dump_guest_stack(void*, const char*);    /* its guest call chain (ppu_loader.cpp) */
extern "C" void kz_coh_stats(void);                          /* spu_coherency.c: reserved-page ages */
extern "C" void kz_store_stats(double secs);                 /* spu_coherency.c: careful stores per page */
extern "C" unsigned kz_coh_expire(int age_s);                /* spu_coherency.c: stale reservation marks cleared */
static void kz_watchr_trap(int, siginfo_t*, void*)
{
    if (!s_kzr_on) return;
    __atomic_add_fetch(&s_kzr_hits, 1, __ATOMIC_RELAXED);
    if (s_kzr_exec) {   /* an execute breakpoint on lifted code: the guest call chain, first 6 hits */
        if (s_kzr_hits <= 6) {
            char l[96]; const int n = snprintf(l, sizeof l, "[kzexec] hit %lu tid=%ld\n", s_kzr_hits, (long)syscall(SYS_gettid));
            if (write(2, l, n) < 0) {}
            if (g_active_ctx) ppu_dump_guest_stack(g_active_ctx, "kzexec");
        }
        return;
    }
    /* read the watched word through the kernel: a user load here would hit the breakpoint again
     * (SIGTRAP is blocked in the handler, so it re-fires on return: an endless trap loop) */
    uint8_t p[4] = {0};
    { struct iovec lo = { p, 4 }, re = { (void*)(uintptr_t)s_kzr_host, 4 };
      if (process_vm_readv(getpid(), &lo, 1, &re, 1, 0) != 4) return; }
    const unsigned v = ((unsigned)p[0] << 24) | ((unsigned)p[1] << 16) | ((unsigned)p[2] << 8) | p[3];
    if (s_kzr_mask && !(v & s_kzr_mask)) return;   /* only while the watched bits are set */
    void* fr[40]; int n = backtrace(fr, 40);
    static uintptr_t seen[512][4]; static int nseen;
    uintptr_t key[4] = { n > 1 ? (uintptr_t)fr[1] : 0, n > 2 ? (uintptr_t)fr[2] : 0, n > 3 ? (uintptr_t)fr[3] : 0, n > 4 ? (uintptr_t)fr[4] : 0 };
    for (int i = 0; i < nseen; i++) if (!memcmp(seen[i], key, sizeof key)) return;
    if (nseen >= 512) return;
    memcpy(seen[nseen++], key, sizeof key);
    char line[1400]; int len = snprintf(line, sizeof line, "[kzbt] tid=%ld comm=kzwatchr-%08X:", (long)syscall(SYS_gettid), v);
    for (int i = 1; i < n && len < (int)sizeof line - 40; i++) {
        Dl_info di;
        if (dladdr(fr[i], &di) && di.dli_fbase) {
            const char* nm = di.dli_fname ? strrchr(di.dli_fname, '/') : nullptr;
            const unsigned long off = (unsigned long)((uintptr_t)fr[i] - (uintptr_t)di.dli_fbase);
            if (nm && strstr(nm, "Infamous2Game")) len += snprintf(line + len, sizeof line - len, " m%lx", off);
            else if (nm && strstr(nm, ".so"))       len += snprintf(line + len, sizeof line - len, " %s+%lx", nm + 1, off);
            else                                    len += snprintf(line + len, sizeof line - len, " e%lx", off);
        }
    }
    line[len++] = '\n';
    if (write(2, line, len) < 0) {}
    if (g_active_ctx) ppu_dump_guest_stack(g_active_ctx, "kzwatchr");
}
/* Function coverage for an instrumented lift (PPU_MM_SNAP -> KZ_COV_HOOK): every lifted function
 * registers its counter on its first call. "cov start" zeroes them, "cov dump <file>" writes
 * "<count> <name>" for every function that ran since -- two windows (idle / a button held) diffed
 * name the code a button reaches. */
static struct { const char* name; unsigned int* n; } s_kzcov[200000];
static volatile int s_kzcov_n;
static int s_kzcov_lk;
extern "C" __attribute__((visibility("default"))) void kz_cov_reg(const char* name, unsigned int* n)
{
    while (__atomic_exchange_n(&s_kzcov_lk, 1, __ATOMIC_ACQUIRE)) {}
    if (s_kzcov_n < 200000) { s_kzcov[s_kzcov_n].name = name; s_kzcov[s_kzcov_n].n = n; s_kzcov_n++; }
    __atomic_store_n(&s_kzcov_lk, 0, __ATOMIC_RELEASE);
}
static void kz_cov_cmd(char* arg)
{
    if (!strncmp(arg, "start", 5)) {
        for (int i = 0; i < s_kzcov_n; i++) *s_kzcov[i].n = 0;
        fprintf(stderr, "[kzcov] start (%d functions known)\n", s_kzcov_n);
    } else if (!strncmp(arg, "dump ", 5)) {
        char* fn = arg + 5; fn[strcspn(fn, "\r\n ")] = 0;
        FILE* f = fopen(fn, "w"); int k = 0;
        for (int i = 0; f && i < s_kzcov_n; i++) if (*s_kzcov[i].n) { fprintf(f, "%u %s\n", *s_kzcov[i].n, s_kzcov[i].name); k++; }
        if (f) fclose(f);
        fprintf(stderr, "[kzcov] dump %s: %d functions ran\n", fn, k);
    }
}
static void* kz_watchr_thread(void* arg)
{
    const char* tf = (const char*)arg;
    struct sigaction sa; memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = kz_watchr_trap; sa.sa_flags = SA_SIGINFO | SA_RESTART;
    static int expire = -1;
    if (expire < 0) { const char* e = getenv("KZ_COH_EXPIRE"); expire = !(e && *e == '0'); }
    unsigned tick = 0, total = 0;
    for (;;) {
        usleep(300000);
        if (expire && (++tick % 7u) == 0) {   /* ~2 s: stale reservation marks back to the fast path */
            const unsigned n = kz_coh_expire(3);
            total += n;
            if (n && total <= 2000) fprintf(stderr, "[kzcoh] expired %u stale reserved pages (%u so far)\n", n, total);
        }
        FILE* f = fopen(tf, "r"); if (!f) continue;
        char buf[512] = {0}; if (!fgets(buf, sizeof buf, f)) buf[0] = 0; fclose(f); remove(tf);
        /* "exec <hex offset in the game module> [seconds]": a hardware execute breakpoint there */
        s_kzr_exec = 0;
        if (!strncmp(buf, "exec ", 5)) {
            char* e3; const unsigned long off = strtoul(buf + 5, &e3, 16);
            int secs = (int)strtol(e3, nullptr, 10); if (secs <= 0) secs = 10;
            uint64_t base = 0; const char* mp0 = getenv("PS3_GAME_MODULE"); char mpr[PATH_MAX]; const char* mp = mp0 && realpath(mp0, mpr) ? mpr : mp0;   /* maps has the resolved path */
            FILE* mf = fopen("/proc/self/maps", "r"); char ml[600];
            while (mf && fgets(ml, sizeof ml, mf)) {
                unsigned long lo, fo; char path[400] = {0};
                if (sscanf(ml, "%lx-%*lx %*s %lx %*s %*s %399s", &lo, &fo, path) == 3 && fo == 0 && mp && !strcmp(path, mp)) { base = lo; break; }
            }
            if (mf) fclose(mf);
            if (!base) { fprintf(stderr, "[kzexec] module base not found\n"); continue; }
            s_kzr_exec = 1; s_kzr_hits = 0; s_kzr_mask = 0;
            sigaction(SIGTRAP, &sa, nullptr);
            static long xf[512]; int nx = 0;
            DIR* d2 = opendir("/proc/self/task"); struct dirent* de2;
            while (d2 && (de2 = readdir(d2)) && nx < 512) {
                const long tid = strtol(de2->d_name, nullptr, 10); if (tid <= 0) continue;
                struct perf_event_attr a; memset(&a, 0, sizeof a);
                a.type = PERF_TYPE_BREAKPOINT; a.size = sizeof a;
                a.bp_type = HW_BREAKPOINT_X; a.bp_addr = base + off; a.bp_len = sizeof(long);
                a.sample_period = 1; a.exclude_kernel = 1; a.exclude_hv = 1; a.sigtrap = 1; a.remove_on_exec = 1;
                const long fd = syscall(SYS_perf_event_open, &a, (pid_t)tid, -1, -1, 0);
                if (fd >= 0) xf[nx++] = fd;
            }
            if (d2) closedir(d2);
            s_kzr_on = 1;
            fprintf(stderr, "[kzexec] module+%lx (host %p): %d threads armed for %d s\n", off, (void*)(uintptr_t)(base + off), nx, secs);
            sleep((unsigned)secs);
            s_kzr_on = 0;
            for (int i = 0; i < nx; i++) close((int)xf[i]);
            fprintf(stderr, "[kzexec] off: %lu hits\n", s_kzr_hits); s_kzr_hits = 0; s_kzr_exec = 0;
            continue;
        }
        uint32_t ea; char* ep = buf;
        if (!strncmp(buf, "cov ", 4)) { kz_cov_cmd(buf + 4); continue; }
        if (!strncmp(buf, "cohstat", 7)) { kz_coh_stats(); continue; }
        if (!strncmp(buf, "storestat", 9)) {   /* zero, wait N s (default 3), report */
            int secs = atoi(buf + 9); if (secs <= 0) secs = 3;
            kz_store_stats(1.0); sleep((unsigned)secs); kz_store_stats((double)secs); continue; }
        /* "find <hex value> [<hex value>...]": every aligned guest word in the heaps (0x10000000..0x10800000,
         * 0x30000000..0x50000000) equal to one of them -- objects by their vtable pointer */
        if (!strncmp(buf, "find ", 5)) {
            uint32_t want[8]; int nw = 0; char* q = buf + 5;
            while (nw < 8) { char* e2; const unsigned long v = strtoul(q, &e2, 16); if (e2 == q) break; want[nw++] = (uint32_t)v; q = e2; }
            static const uint32_t rg[2][2] = { { 0x00010000u, 0x0FF00000u }, { 0x0FF00000u, 0xD0000000u } };   /* the whole user space */
            int hits = 0;
            for (int r = 0; r < 2; r++)
                for (uint32_t a = rg[r][0]; a < rg[r][1] && hits < 2000; a += 4) {
                    if ((a & 0xFFFFFu) == 0 && ppu_vm_size && a >= ppu_vm_size) break;
                    const uint8_t* m = vm_base + a;
                    const uint32_t w = ((uint32_t)m[0] << 24) | ((uint32_t)m[1] << 16) | ((uint32_t)m[2] << 8) | m[3];
                    for (int i = 0; i < nw; i++) if (w == want[i]) { fprintf(stderr, "[kzfind] %08X at %08X\n", w, a); hits++; }
                }
            fprintf(stderr, "[kzfind] done, %d hits\n", hits);
            continue;
        }
        /* "peek <spec> [seconds] [words]": no breakpoint -- sample the words every 2 ms and log each change */
        const int peek = !strncmp(buf, "peek ", 5);
        if (peek) { memmove(buf, buf + 5, strlen(buf + 5) + 1); }
        if (!strncmp(buf, "pad", 3)) {
            /* TOC slot -> the variable -> the pad block (func_0058157C: r31 = [r2-0x4B50], base = [r31]) */
            const uint8_t* q = vm_base + 0x00BDB7D8u;
            const uint32_t var = ((uint32_t)q[0] << 24) | ((uint32_t)q[1] << 16) | ((uint32_t)q[2] << 8) | q[3];
            q = vm_base + var;
            ea = (((uint32_t)q[0] << 24) | ((uint32_t)q[1] << 16) | ((uint32_t)q[2] << 8) | q[3]); ep = buf + 3;
            if (*ep == '+') ea += (uint32_t)strtoul(ep + 1, &ep, 16); else ea += 0x198u;   /* "pad+<hex offset>" */
        } else ea = (uint32_t)strtoul(buf, &ep, 16);
        int secs = (int)strtol(ep, &ep, 10); if (secs <= 0) secs = 8;
        if (peek) {
            int nw = (int)strtol(ep, nullptr, 10); if (nw <= 0 || nw > 16) nw = 4;
            uint32_t last[16]; memset(last, 0xFF, sizeof last);
            struct timespec t0, t; clock_gettime(CLOCK_MONOTONIC, &t0);
            fprintf(stderr, "[kzpeek] guest %08X, %d words, %d s\n", ea, nw, secs);
            for (;;) {
                clock_gettime(CLOCK_MONOTONIC, &t);
                const double el = (double)(t.tv_sec - t0.tv_sec) + (double)(t.tv_nsec - t0.tv_nsec) * 1e-9;
                if (el > secs) break;
                int ch = 0; uint32_t now[16];
                for (int i = 0; i < nw; i++) { const uint8_t* q = vm_base + ea + 4u * i;
                    now[i] = ((uint32_t)q[0] << 24) | ((uint32_t)q[1] << 16) | ((uint32_t)q[2] << 8) | q[3]; if (now[i] != last[i]) ch = 1; }
                if (ch) { char l[400]; int n = snprintf(l, sizeof l, "[kzpeek] t=%.3f", el);
                    for (int i = 0; i < nw; i++) n += snprintf(l + n, sizeof l - n, " %08X", now[i]);
                    fprintf(stderr, "%s\n", l); memcpy(last, now, sizeof now); }
                usleep(2000);
            }
            fprintf(stderr, "[kzpeek] off\n");
            continue;
        }
        s_kzr_mask = (uint32_t)strtoul(ep, nullptr, 16);   /* "pad+4 20 8000": report only with those bits set */
        s_kzr_host = (uint64_t)(uintptr_t)(vm_base + ea);
        sigaction(SIGTRAP, &sa, nullptr);
        static long fds[512]; int nf = 0;
        DIR* d = opendir("/proc/self/task");
        struct dirent* de;
        while (d && (de = readdir(d)) && nf < 512) {
            const long tid = strtol(de->d_name, nullptr, 10); if (tid <= 0) continue;
            struct perf_event_attr a; memset(&a, 0, sizeof a);
            a.type = PERF_TYPE_BREAKPOINT; a.size = sizeof a;
            a.bp_type = HW_BREAKPOINT_RW; a.bp_addr = s_kzr_host & ~3ull; a.bp_len = HW_BREAKPOINT_LEN_4;
            a.sample_period = 1; a.exclude_kernel = 1; a.exclude_hv = 1; a.sigtrap = 1; a.remove_on_exec = 1;
            const long fd = syscall(SYS_perf_event_open, &a, (pid_t)tid, -1, -1, 0);
            if (fd >= 0) fds[nf++] = fd;
        }
        if (d) closedir(d);
        s_kzr_on = 1;
        fprintf(stderr, "[kzwatchr] guest %08X (host %p): %d threads armed for %d s, mask %08X\n", ea, (void*)(uintptr_t)s_kzr_host, nf, secs, s_kzr_mask);
        sleep((unsigned)secs);
        s_kzr_on = 0;
        for (int i = 0; i < nf; i++) close((int)fds[i]);
        fprintf(stderr, "[kzwatchr] off: %lu traps\n", s_kzr_hits); s_kzr_hits = 0;
    }
    return nullptr;
}
extern "C" void kz_watch_arm_main(void)
{
    { const char* rt = getenv("KZ_WATCHR_TRIGGER");
      if (rt && *rt) { pthread_t th; if (pthread_create(&th, nullptr, kz_watchr_thread, (void*)rt) == 0) pthread_detach(th); } }
    const char* e = getenv("KZ_WATCH_EA");
    if (!e || !*e) return;
    s_kzw_ea = (uint32_t)strtoul(e, nullptr, 16);
    s_kzw_host = (uint64_t)(uintptr_t)(vm_base + s_kzw_ea);
    s_kzw_main = (pid_t)syscall(SYS_gettid);
    struct sigaction sa; memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = kz_watch_trap; sa.sa_flags = SA_SIGINFO | SA_RESTART;
    sigaction(SIGTRAP, &sa, nullptr);
    struct perf_event_attr a; memset(&a, 0, sizeof a);
    a.type = PERF_TYPE_BREAKPOINT; a.size = sizeof a;
    a.bp_type = HW_BREAKPOINT_W; a.bp_addr = s_kzw_host; a.bp_len = HW_BREAKPOINT_LEN_8;
    a.sample_period = 1; a.exclude_kernel = 1; a.exclude_hv = 1;
    a.inherit = 1; a.inherit_thread = 1; a.sigtrap = 1; a.remove_on_exec = 1;
    long fd = syscall(SYS_perf_event_open, &a, 0, -1, -1, 0);
    fprintf(stderr, "[kzwatch] armed on guest %08X (host %p): fd=%ld%s%s\n", s_kzw_ea, (void*)(uintptr_t)s_kzw_host, fd,
            fd < 0 ? " errno=" : "", fd < 0 ? strerror(errno) : "");
}
#endif
