#!/usr/bin/env python3
"""Make the shader seed a release ships (engine: PS3RECOMP_VK_SHADER_SEED, rsx_vulkan_backend.c
vk_sc_seed_import / vk_pipe_async.inc vk_pw_load).

    make_shader_seed.py <shader cache dir> <out dir> <seed id>

From a cache that has played the game (~/.cache/ps3recomp/shader_cache/<title>): its
pipelines.rcp, and the .spv of every shader those recipes name that the cache holds. Shaders no
recipe names (older translator versions, abandoned runs) are left out. Writes <out>/seed.lst
(first line the id, then one file name per line), the .spv files and pipelines.rcp."""
import os, shutil, struct, sys

def main():
    src, out, sid = sys.argv[1:4]
    rcp = open(os.path.join(src, 'pipelines.rcp'), 'rb').read()
    if rcp[:8] != b'PS3RVKRC' or len(rcp) < 48:
        sys.exit('not a recipe file')
    ver, rsz, ssz, n = struct.unpack_from('<IIII', rcp, 8)
    if 48 + n * rsz != len(rcp):
        sys.exit(f'recipe file size mismatch: {n} x {rsz} + 48 != {len(rcp)}')
    keys = set()
    for i in range(n):
        va, vb, fa, fb = struct.unpack_from('<QQQQ', rcp, 48 + i * rsz)
        keys.add(f'{va:016x}{vb:016x}.spv')
        keys.add(f'{fa:016x}{fb:016x}.spv')
    have = sorted(k for k in keys if os.path.isfile(os.path.join(src, k)))
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    for k in have:
        shutil.copy2(os.path.join(src, k), os.path.join(out, k))
    shutil.copy2(os.path.join(src, 'pipelines.rcp'), os.path.join(out, 'pipelines.rcp'))
    with open(os.path.join(out, 'seed.lst'), 'w', newline='\n') as f:
        f.write(sid + '\n')
        for k in have:
            f.write(k + '\n')
    size = sum(os.path.getsize(os.path.join(out, k)) for k in have)
    print(f'{n} recipes, {len(keys)} shaders named, {len(have)} in the cache ({size // 1024} KB) -> {out}')

main()
