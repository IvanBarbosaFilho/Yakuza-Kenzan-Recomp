#!/usr/bin/env python3
"""Post-pass over the lifted PPU C: write the stack pointer back to the context at every
plain return of a function that keeps r1 in a local.

ppu_nv_dirty's liveness treats r1 as preserved for native callers, so epilogues pop the
frame in the local only (`r1 = r1 + N; return;`). Some call sites still reload r1 from the
context after the call (callees with an indirect-jump exit, e.g. func_009E56D0); those then
picked up the callee's lowered r1 -- the caller ran 0x1A0 bytes down its own frame, reading
garbage (Ryu ga Gotoku Kenzan! func_0089D804's item count) until it crashed.
Writing ctx->gpr[1] = r1 at the return makes the context right for every caller.

Usage: fix_r1_return.py <dir of ppu_recomp_*.cpp>   (idempotent)"""
import re, sys, glob

FUNC = re.compile(r'^void func_[0-9A-F]+\(ppu_context\* ctx\) \{')
COND_RET = re.compile(r'^(\s*)if \((.*)\) return;\s*$')
total = 0
for path in sorted(glob.glob(sys.argv[1] + '/ppu_recomp_*.cpp')):
    lines = open(path).read().split('\n')
    has_r1 = False; changed = 0
    for i, l in enumerate(lines):
        if FUNC.match(l):
            has_r1 = False
            continue
        if not has_r1 and 'uint64_t r1 = ctx->gpr[1];' in l:
            has_r1 = True
        if not has_r1 or 'ctx->gpr[1] = r1; return;' in l:
            continue
        s = l.strip()
        if s == 'return;':
            ind = l[:len(l) - len(l.lstrip())]
            lines[i] = ind + 'ctx->gpr[1] = r1; return;'
            changed += 1
        else:
            m = COND_RET.match(l)
            if m and '(ctx)' not in m.group(2):
                lines[i] = f'{m.group(1)}if ({m.group(2)}) {{ ctx->gpr[1] = r1; return; }}'
                changed += 1
    if changed:
        open(path, 'w').write('\n'.join(lines))
        total += changed
print(f'[fix_r1_return] {total} return(s) now write r1 back')
