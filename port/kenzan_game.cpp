/* Ryu ga Gotoku Kenzan! (BLJM-60064): the title hooks the runner (main.cpp, from the inFAMOUS 2
 * port base) calls. Kenzan needs none of inFAMOUS 2's GPU passes, clock log or community patches,
 * so they are empty here. */
#include <cstdint>
extern "C" void infamous2_gpu_light_install(void) {}
extern "C" void infamous2_gpu_ssao_install(void) {}
extern "C" void infamous2_edge_watch_install(void) {}
extern "C" void infamous2_gclock_frame(void) {}
extern "C" void infamous2_apply_patches(const char*) {}
extern "C" void infamous2_apply_aspect(const char*) {}
