/*
 * ps3recomp - HLSL -> SPIR-V through glslang, for the Vulkan backend
 *
 * See rsx_shader_spirv.h for the binding contract. The front end is driven
 * exactly as rsx_shader_msl.cpp drives it (C++ API for setEntryPoint, Vulkan
 * rules, auto-mapped bindings and locations, then mapIO, legalized SPIR-V),
 * with three differences, all because the output goes to a Vulkan driver
 * rather than to spirv-cross:
 *
 *   - Vulkan 1.0 client, SPIR-V 1.0: the backend creates a 1.0 instance and
 *     promises to run on 1.0-level drivers. (The MSL path targets 1.3 because
 *     that is spirv-cross's floor, not a driver's.)
 *   - Texture and sampler registers are shifted apart (setShiftBinding),
 *     because Vulkan has one binding namespace per set.
 *   - The vertex stage flips Y (setInvertY): D3D clip space is +y up.
 */

#include <cstdlib>
#include "rsx_shader_spirv.h"

#include <stdio.h>
#include <string.h>

#if defined(PS3RECOMP_HAVE_SPIRV_TRANSLATION)

#include <glslang/Public/ShaderLang.h>
#include <glslang/Public/ResourceLimits.h>
#include <glslang/SPIRV/GlslangToSpv.h>

#include <vector>

namespace {

void put_log(char* log, u32 log_size, const char* what, const char* detail)
{
    if (!log || log_size == 0) return;
    snprintf(log, log_size, "%s%s%s", what ? what : "",
             (what && detail && *detail) ? ": " : "", detail ? detail : "");
}

bool glslang_ready()
{
    static bool inited = false;
    if (!inited) {
        if (!glslang::InitializeProcess()) return false;
        inited = true;
    }
    return true;
}

/* rsx_spirv_set_nan_preserve / rsx_spirv_set_denorm: see rsx_shader_spirv.h. */
int g_nan_preserve = 0;
int g_denorm = 0;      /* 0 none, 1 DenormPreserve, 2 DenormFlushToZero (fp32) */

/* Declare the fp32 float-controls execution modes asked for --
 * SignedZeroInfNanPreserve (g_nan_preserve) and DenormPreserve or
 * DenormFlushToZero (g_denorm) -- on every entry point (SPV_KHR_float_controls;
 * SPIR-V 1.0 needs the extension named). In the module's section order: the
 * capabilities after the last OpCapability, the extension after them
 * (OpExtension comes before OpExtInstImport), the execution modes after the
 * last OpEntryPoint (before any existing OpExecutionMode, which is also
 * allowed). A mode whose capability the module already has is left alone.
 * False on a module this does not understand (left as it was). */
bool add_float_controls(std::vector<unsigned int>& m)
{
    enum { OpExtension = 10, OpCapability = 17, OpEntryPoint = 15, OpExecutionMode = 16 };
    /* capability, execution mode */
    enum { CapSZINP = 4466, ModeSZINP = 4461, CapDenormPreserve = 4464, ModeDenormPreserve = 4459,
           CapDenormFTZ = 4465, ModeDenormFTZ = 4460 };
    unsigned want[2][2]; int nwant = 0;
    if (g_nan_preserve) { want[nwant][0] = CapSZINP; want[nwant][1] = ModeSZINP; nwant++; }
    if (g_denorm == 1)  { want[nwant][0] = CapDenormPreserve; want[nwant][1] = ModeDenormPreserve; nwant++; }
    if (g_denorm == 2)  { want[nwant][0] = CapDenormFTZ; want[nwant][1] = ModeDenormFTZ; nwant++; }
    if (!nwant) return true;
    if (m.size() < 5 || m[0] != 0x07230203u) return false;
    size_t cap_end = 0, ep_end = 0;
    std::vector<unsigned int> entries;
    for (size_t i = 5; i < m.size();) {
        const unsigned op = m[i] & 0xFFFFu, n = m[i] >> 16;
        if (n == 0 || i + n > m.size()) return false;
        if (op == OpCapability) {
            for (int k = 0; k < nwant; k++)
                if (n >= 2 && m[i + 1] == want[k][0]) want[k][0] = 0;   /* already there */
            cap_end = i + n;
        } else if (op == OpEntryPoint) {
            if (n < 3) return false;
            entries.push_back(m[i + 2]);
            ep_end = i + n;
        } else if (op != OpExtension && op != 11 /* OpExtInstImport */ && op != 14 /* OpMemoryModel */ &&
                   op != OpExecutionMode) {
            break;   /* past the module's head */
        }
        i += n;
    }
    int nadd = 0;
    for (int k = 0; k < nwant; k++) {
        if (!want[k][0]) continue;
        want[nadd][0] = want[k][0]; want[nadd][1] = want[k][1]; nadd++;
    }
    if (!nadd) return true;
    if (!cap_end || !ep_end || entries.empty() || cap_end > ep_end) return false;
    static const char ext[] = "SPV_KHR_float_controls";          /* 22 chars + NUL: 6 words */
    std::vector<unsigned int> ext_ins(1 + (sizeof ext + 3) / 4, 0u);
    ext_ins[0] = ((unsigned)ext_ins.size() << 16) | OpExtension;
    memcpy(&ext_ins[1], ext, sizeof ext);
    std::vector<unsigned int> out;
    out.reserve(m.size() + 2u * (unsigned)nadd + ext_ins.size() + 4u * (unsigned)nadd * entries.size());
    out.insert(out.end(), m.begin(), m.begin() + (long)cap_end);
    for (int k = 0; k < nadd; k++) { out.push_back((2u << 16) | OpCapability); out.push_back(want[k][0]); }
    out.insert(out.end(), ext_ins.begin(), ext_ins.end());
    out.insert(out.end(), m.begin() + (long)cap_end, m.begin() + (long)ep_end);
    for (unsigned id : entries)
        for (int k = 0; k < nadd; k++) {
            out.push_back((4u << 16) | OpExecutionMode); out.push_back(id);
            out.push_back(want[k][1]); out.push_back(32u);
        }
    out.insert(out.end(), m.begin() + (long)ep_end, m.end());
    m.swap(out);
    return true;
}

} // namespace

extern "C" void rsx_spirv_set_nan_preserve(int on) { g_nan_preserve = on ? 1 : 0; }
extern "C" void rsx_spirv_set_denorm(int mode) { g_denorm = (mode == 1 || mode == 2) ? mode : 0; }

extern "C" int rsx_hlsl_to_spirv_available(void) { return 1; }

extern "C" int rsx_hlsl_to_spirv(const char* hlsl, int stage,
                                 u32* words, u32 max_words, u32* out_words,
                                 char* log, u32 log_size)
{
    if (out_words) *out_words = 0;
    if (!hlsl || !words || !out_words || max_words == 0) {
        put_log(log, log_size, "bad arguments", nullptr);
        return -1;
    }
    if (stage != RSX_SHADER_STAGE_VERTEX && stage != RSX_SHADER_STAGE_FRAGMENT) {
        put_log(log, log_size, "unknown shader stage", nullptr);
        return -1;
    }
    if (!glslang_ready()) {
        put_log(log, log_size, "glslang::InitializeProcess failed", nullptr);
        return -1;
    }

    const EShLanguage lang = (stage == RSX_SHADER_STAGE_VERTEX) ? EShLangVertex
                                                                : EShLangFragment;
    const EShMessages msgs =
        (EShMessages)(EShMsgSpvRules | EShMsgVulkanRules | EShMsgReadHlsl);

    glslang::TShader shader(lang);
    const char* strings[1] = { hlsl };
    shader.setStrings(strings, 1);
    shader.setEnvInput(glslang::EShSourceHlsl, lang, glslang::EShClientVulkan, 100);
    shader.setEnvClient(glslang::EShClientVulkan, glslang::EShTargetVulkan_1_0);
    shader.setEnvTarget(glslang::EShTargetSpv, glslang::EShTargetSpv_1_0);
    /* SPIR-V FragCoord.w is 1/w (FXC's SV_POSITION.w is w): tells the FP
     * decompiler's _rsx_inv_w helper which one it gets. */
    /* PS3RECOMP_FRAGCOORD_W=w: treat the fragment position's w as the clip w
     * (the decompiler then emits 1/w) instead of its reciprocal -- a switch
     * to settle which one this stage's position carries, by looking. */
    { static int rcp = -1; if (rcp < 0) { const char* e = getenv("PS3RECOMP_FRAGCOORD_W"); rcp = (e && *e == 'w') ? 0 : 1; }
      if (rcp) shader.setPreamble("#define RSX_FRAGCOORD_W_IS_RCP 1\n"); }
    shader.setEntryPoint("main");
    shader.setAutoMapBindings(true);
    shader.setAutoMapLocations(true);
    shader.setShiftBinding(glslang::EResTexture, RSX_SPIRV_TEXTURE_BINDING);
    shader.setShiftBinding(glslang::EResSampler, RSX_SPIRV_SAMPLER_BINDING);
    if (lang == EShLangVertex) shader.setInvertY(true);

    if (!shader.parse(GetDefaultResources(), 100, false, msgs)) {
        put_log(log, log_size, "HLSL parse", shader.getInfoLog());
        return -1;
    }
    glslang::TProgram program;
    program.addShader(&shader);
    if (!program.link(msgs)) {
        put_log(log, log_size, "HLSL link", program.getInfoLog());
        return -1;
    }
    if (!program.mapIO()) {
        put_log(log, log_size, "HLSL io map", program.getInfoLog());
        return -1;
    }

    std::vector<unsigned int> spirv;
    spv::SpvBuildLogger logger;
    glslang::SpvOptions opts;
    opts.disableOptimizer = false;     /* run the HLSL legalization passes */
    glslang::GlslangToSpv(*program.getIntermediate(lang), spirv, &logger, &opts);
    if (spirv.empty()) {
        put_log(log, log_size, "SPIR-V generation", logger.getAllMessages().c_str());
        return -1;
    }
    if ((g_nan_preserve || g_denorm) && !add_float_controls(spirv)) {
        put_log(log, log_size, "SPIR-V float controls", "unexpected module head");
        return -1;
    }
    if (spirv.size() > max_words) {
        char n[64];
        snprintf(n, sizeof n, "%zu words, room for %u", spirv.size(), max_words);
        put_log(log, log_size, "SPIR-V output too large", n);
        return -1;
    }
    memcpy(words, spirv.data(), spirv.size() * sizeof(u32));
    *out_words = (u32)spirv.size();
    return 0;
}

/* The backend's own GLSL passes. Same front end and targets as above, without
 * the HLSL-only parts: no binding shifts or auto-mapping (the source is
 * explicit), no Y flip (the passes are full-screen triangles that only read
 * gl_FragCoord), no fragcoord preamble. The legalisation passes are an HLSL
 * matter, so the SPIR-V goes to the driver as glslang wrote it -- which also
 * keeps every NoContraction (`precise`) decoration exactly where the source
 * put it. */
extern "C" int rsx_glsl_to_spirv(const char* glsl, int stage,
                                 u32* words, u32 max_words, u32* out_words,
                                 char* log, u32 log_size)
{
    if (out_words) *out_words = 0;
    if (!glsl || !words || !out_words || max_words == 0) {
        put_log(log, log_size, "bad arguments", nullptr);
        return -1;
    }
    if (stage != RSX_SHADER_STAGE_VERTEX && stage != RSX_SHADER_STAGE_FRAGMENT) {
        put_log(log, log_size, "unknown shader stage", nullptr);
        return -1;
    }
    if (!glslang_ready()) {
        put_log(log, log_size, "glslang::InitializeProcess failed", nullptr);
        return -1;
    }

    const EShLanguage lang = (stage == RSX_SHADER_STAGE_VERTEX) ? EShLangVertex
                                                                : EShLangFragment;
    const EShMessages msgs = (EShMessages)(EShMsgSpvRules | EShMsgVulkanRules);

    glslang::TShader shader(lang);
    const char* strings[1] = { glsl };
    shader.setStrings(strings, 1);
    shader.setEnvInput(glslang::EShSourceGlsl, lang, glslang::EShClientVulkan, 100);
    shader.setEnvClient(glslang::EShClientVulkan, glslang::EShTargetVulkan_1_0);
    shader.setEnvTarget(glslang::EShTargetSpv, glslang::EShTargetSpv_1_0);
    shader.setEntryPoint("main");

    if (!shader.parse(GetDefaultResources(), 450, false, msgs)) {
        put_log(log, log_size, "GLSL parse", shader.getInfoLog());
        return -1;
    }
    glslang::TProgram program;
    program.addShader(&shader);
    if (!program.link(msgs)) {
        put_log(log, log_size, "GLSL link", program.getInfoLog());
        return -1;
    }

    std::vector<unsigned int> spirv;
    spv::SpvBuildLogger logger;
    glslang::SpvOptions opts;
    opts.disableOptimizer = true;
    glslang::GlslangToSpv(*program.getIntermediate(lang), spirv, &logger, &opts);
    if (spirv.empty()) {
        put_log(log, log_size, "SPIR-V generation", logger.getAllMessages().c_str());
        return -1;
    }
    if (spirv.size() > max_words) {
        char n[64];
        snprintf(n, sizeof n, "%zu words, room for %u", spirv.size(), max_words);
        put_log(log, log_size, "SPIR-V output too large", n);
        return -1;
    }
    memcpy(words, spirv.data(), spirv.size() * sizeof(u32));
    *out_words = (u32)spirv.size();
    return 0;
}

#else /* !PS3RECOMP_HAVE_SPIRV_TRANSLATION */

extern "C" void rsx_spirv_set_nan_preserve(int on) { (void)on; }
extern "C" void rsx_spirv_set_denorm(int mode) { (void)mode; }

extern "C" int rsx_hlsl_to_spirv_available(void) { return 0; }

extern "C" int rsx_hlsl_to_spirv(const char* hlsl, int stage,
                                 u32* words, u32 max_words, u32* out_words,
                                 char* log, u32 log_size)
{
    (void)hlsl; (void)stage; (void)words; (void)max_words;
    if (out_words) *out_words = 0;
    if (log && log_size)
        snprintf(log, log_size, "HLSL -> SPIR-V translation not built (glslang not found)");
    return -1;
}

extern "C" int rsx_glsl_to_spirv(const char* glsl, int stage,
                                 u32* words, u32 max_words, u32* out_words,
                                 char* log, u32 log_size)
{
    (void)glsl; (void)stage; (void)words; (void)max_words;
    if (out_words) *out_words = 0;
    if (log && log_size)
        snprintf(log, log_size, "GLSL -> SPIR-V compilation not built (glslang not found)");
    return -1;
}

#endif
