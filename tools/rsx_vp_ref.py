#!/usr/bin/env python3
"""Reference interpreter for RSX (NV40) vertex programs, from the ucode -- not from our HLSL.

usage: rsx_vp_ref.py <capture dir> <seq> [--vert N] [--trace] [--mode rsx|ours]

Reads <dir>/<seq>_{ucode,consts,verts}.bin and _meta.txt (engine KZ_VPCAP_*), runs the program
for each captured vertex and prints o[0] (clip position) and the outputs. Two semantic modes:
  rsx   the hardware rules as RPCS3's VertexProgramDecompiler has them (the oracle);
  ours  the rules engine/libs/video/rsx_vp_decompiler.c had before PS3RECOMP_VP_CC_RPCS3
        (CC written only with both update bits, never by the scalar unit; a branch tests
        the first swizzled CC component; ARL floors).
--trace prints every instruction's writes; --diff runs both modes and stops at the first
instruction whose register file differs.

Field layout (same as rsx_vp_decompiler.c's header comment):
  D0: cc swizzle w/z/y/x [2:9], cond [10:12], cond_test [13], update0 [14], dst_tmp [15:20],
      src abs [21:23], addr_reg_sel [24], cond_reg_sel [25], saturate [26], update1 [29],
      vec_result [30]; addr_swz [0:1]
  D1: src0h [0:7], input_src [8:11], const_src [12:21], vec_op [22:26], sca_op [27:31]
  D2: src2h [0:5], src1 [6:22], src0l [23:31]
  D3: end [0], index_const [1], dst [2:6], sca_dst_tmp [7:12], vec mask w/z/y/x [13:16],
      sca mask w/z/y/x [17:20], src2l [21:31]
"""
import math, struct, sys

def f32(x):
    """round to single precision (the RSX VP computes in fp32)"""
    try:
        return struct.unpack('<f', struct.pack('<f', x))[0]
    except OverflowError:
        return math.copysign(math.inf, x)

def vmul(a, b):   # NV rule: 0 * anything = 0
    return 0.0 if a == 0.0 or b == 0.0 else f32(a * b)

def load(d, seq):
    uc = open(f'{d}/{seq}_ucode.bin', 'rb').read()
    cb = open(f'{d}/{seq}_consts.bin', 'rb').read()
    vb = open(f'{d}/{seq}_verts.bin', 'rb').read()
    meta = {}
    for line in open(f'{d}/{seq}_meta.txt'):
        k, *v = line.split(); meta[k] = v
    words = struct.unpack('<%dI' % (len(uc) // 4), uc)
    instrs = [words[i:i + 4] for i in range(0, len(words), 4)]
    consts = [list(struct.unpack('<4f', cb[i:i + 16])) for i in range(0, len(cb), 16)]
    stride = int(meta['stride'][0]); nv = int(meta['n_verts'][0])
    slots = [int(x) for x in meta['slot_by_attr']]
    verts = []
    for n in range(nv):
        v = []
        for a in range(16):
            s = slots[a]
            if s < 0: v.append([0.0, 0.0, 0.0, 1.0]); continue
            o = n * stride + s * 16
            v.append(list(struct.unpack('<4f', vb[o:o + 16])))
        verts.append(v)
    return instrs, consts, verts, meta

SW = 'xyzw'

class VP:
    def __init__(self, instrs, consts, start, mode, bits=0):
        self.I, self.C, self.start, self.mode, self.bits = instrs, consts, start, mode, bits

    def src(self, s, absf, st, inp, csrc, idx_const, asel, aswz):
        t = s & 3
        if t == 2:   base = st['v'][inp]
        elif t == 3:
            ci = csrc
            if idx_const: ci = (csrc + st['a'][asel][aswz]) & 511
            base = self.C[ci] if ci < len(self.C) else [0.0] * 4
        else:        base = st['r'][(s >> 2) & 63] if ((s >> 2) & 63) < 32 else [0.0] * 4
        sw = [(s >> 14) & 3, (s >> 12) & 3, (s >> 10) & 3, (s >> 8) & 3]
        val = [base[c] for c in sw]
        if absf: val = [abs(x) for x in val]
        if (s >> 16) & 1: val = [-x for x in val]
        return val

    def ccpass(self, st, d0, comp):
        cond = (d0 >> 10) & 7
        if cond == 7: return True
        if cond == 0: return False
        sel = (d0 >> 25) & 1
        sw = [(d0 >> 8) & 3, (d0 >> 6) & 3, (d0 >> 4) & 3, (d0 >> 2) & 3]
        x = st['cc'][sel][sw[comp]]
        return {1: x < 0, 2: x == 0, 3: x <= 0, 4: x > 0, 5: x != 0, 6: x >= 0}[cond]

    def write(self, st, reg, idx, mask, val, d0, test):
        tgt = st[reg][idx]
        for c in range(4):
            if mask[c] and (not test or self.ccpass(st, d0, c)):
                tgt[c] = val[c]

    def run(self, v, trace=False, hook=None):
        st = {'v': v, 'r': [[0.0] * 4 for _ in range(32)], 'o': [[0.0, 0.0, 0.0, 1.0] for _ in range(16)],
              'a': [[0, 0, 0, 0], [0, 0, 0, 0]], 'cc': [[0.0] * 4, [0.0] * 4]}
        pc, calls, steps = 0, [], 0
        while 0 <= pc < len(self.I) and steps < 20000:
            steps += 1
            d0, d1, d2, d3 = self.I[pc]
            nxt = pc + 1
            vop, sop = (d1 >> 22) & 31, (d1 >> 27) & 31
            inp, csrc, idxc = (d1 >> 8) & 15, (d1 >> 12) & 0x3FF, (d3 >> 1) & 1
            s0 = ((d2 >> 23) & 0x1FF) | ((d1 & 0xFF) << 9)
            s1 = (d2 >> 6) & 0x1FFFF
            s2 = ((d3 >> 21) & 0x7FF) | ((d2 & 0x3F) << 11)
            asel, aswz = (d0 >> 24) & 1, d0 & 3
            A = self.src(s0, (d0 >> 21) & 1, st, inp, csrc, idxc, asel, aswz)
            B = self.src(s1, (d0 >> 22) & 1, st, inp, csrc, idxc, asel, aswz)
            Cs = self.src(s2, (d0 >> 23) & 1, st, inp, csrc, idxc, asel, aswz)
            sat = (d0 >> 26) & 1
            test = ((d0 >> 13) & 1) and ((d0 >> 10) & 7) != 7
            dst_tmp, dst, sdst = (d0 >> 15) & 63, (d3 >> 2) & 31, (d3 >> 7) & 63
            vres = (d0 >> 30) & 1
            vm = [(d3 >> 16) & 1, (d3 >> 15) & 1, (d3 >> 14) & 1, (d3 >> 13) & 1]
            sm = [(d3 >> 20) & 1, (d3 >> 19) & 1, (d3 >> 18) & 1, (d3 >> 17) & 1]
            upd0, upd1 = (d0 >> 14) & 1, (d0 >> 29) & 1
            ccsel = (d0 >> 25) & 1
            before = repr(st) if hook else None
            # ---- vector unit
            if any(vm) and vop:
                r = None
                if vop == 1: r = A
                elif vop == 2: r = [vmul(A[i], B[i]) for i in range(4)]
                elif vop == 3: r = [f32(A[i] + Cs[i]) for i in range(4)]
                elif vop == 4: r = [f32(vmul(A[i], B[i]) + Cs[i]) for i in range(4)]
                elif vop == 5: r = [f32(sum(vmul(A[i], B[i]) for i in range(3)))] * 4
                elif vop == 6: r = [f32(sum(vmul(A[i], B[i]) for i in range(3)) + B[3])] * 4
                elif vop == 7: r = [f32(sum(vmul(A[i], B[i]) for i in range(4)))] * 4
                elif vop == 8: r = [1.0, vmul(A[1], B[1]), A[2], B[3]]
                elif vop == 9: r = [min(A[i], B[i]) for i in range(4)]
                elif vop == 10: r = [max(A[i], B[i]) for i in range(4)]
                elif vop == 11: r = [1.0 if A[i] < B[i] else 0.0 for i in range(4)]
                elif vop == 12: r = [1.0 if A[i] >= B[i] else 0.0 for i in range(4)]
                elif vop == 14: r = [A[i] - math.floor(A[i]) for i in range(4)]
                elif vop == 15: r = [float(math.floor(A[i])) for i in range(4)]
                elif vop == 16: r = [1.0 if A[i] == B[i] else 0.0 for i in range(4)]
                elif vop == 17: r = [0.0] * 4
                elif vop == 18: r = [1.0 if A[i] > B[i] else 0.0 for i in range(4)]
                elif vop == 19: r = [1.0 if A[i] <= B[i] else 0.0 for i in range(4)]
                elif vop == 20: r = [1.0 if A[i] != B[i] else 0.0 for i in range(4)]
                elif vop == 21: r = [1.0] * 4
                elif vop == 22: r = [float((x > 0) - (x < 0)) for x in A]
                elif vop == 13:   # ARL
                    conv = (lambda x: int(math.floor(x))) if self.mode == 'ours' else (lambda x: int(x))
                    for c in range(4):
                        if vm[c] and (not test or self.ccpass(st, d0, c)):
                            st['a'][asel][c] = conv(A[c]) if math.isfinite(A[c]) else 0
                else:
                    if trace: print(f'  @{pc} unhandled vec op {vop}')
                if r is not None:
                    if sat: r = [min(max(x, 0.0), 1.0) for x in r]
                    own = (vres and dst != 0x1F) or dst_tmp != 0x3F
                    if vres and dst != 0x1F: self.write(st, 'o', dst & 15, vm, r, d0, test)
                    if dst_tmp != 0x3F: self.write(st, 'r', dst_tmp & 31, vm, r, d0, test)
                    if self.mode == 'ours':
                        if upd0 and upd1: self.write(st, 'cc', ccsel, vm, r, d0, test)
                    elif not own and (dst != 0x1F or upd0 or upd1):
                        self.write(st, 'cc', ccsel, vm, r, d0, test)
                    if trace: print(f'  @{pc} vec{vop} -> {[round(x, 4) for x in r]} o{dst if vres else "-"} r{dst_tmp}')
            # ---- flow
            if sop in (8, 9, 10, 11, 12, 17, 18):
                tgt = ((((d0 >> 23) & 1) << 9) | ((d2 & 0x3F) << 3) | ((d3 >> 29) & 7))
                tgt = tgt - self.start if tgt >= self.start else tgt
                cond = (d0 >> 10) & 7
                if sop in (17, 18):   # BRB / CLB: the transform branch bits
                    take = (((self.bits >> ((d3 >> 23) & 31)) & 1) != 0) == bool((d3 >> 28) & 1)
                    if self.mode == 'ours': take = False   # the old decompiler ran straight through
                elif cond == 7: take = True
                elif cond == 0: take = False
                elif self.mode == 'ours': take = self.ccpass(st, d0, 0)
                else: take = any(self.ccpass(st, d0, c) for c in range(4))
                if trace: print(f'  @{pc} flow{sop} -> {tgt} take={take}')
                if sop in (8, 9, 17):
                    if take: nxt = tgt
                elif sop in (10, 11, 18):
                    if take: calls.append(nxt); nxt = tgt
                elif sop == 12:
                    if take: nxt = calls.pop() if calls else -1
            # ---- scalar unit (reads src2.x)
            elif any(sm) and sop:
                x = Cs[0]; r = None
                if sop == 1: r = Cs
                elif sop == 2: r = [f32(1.0 / x) if x else math.copysign(math.inf, x)] * 4
                elif sop == 3: r = [min(max(f32(1.0 / x) if x else 1.884467e19, 5.42101e-20), 1.884467e19)] * 4
                elif sop == 4: r = [f32(1.0 / math.sqrt(max(abs(x), 1e-10)))] * 4
                elif sop in (5, 14): r = [f32(2.0 ** x) if x < 128 else math.inf] * 4
                elif sop in (6, 13): r = [f32(math.log2(max(abs(x), 1e-10)))] * 4
                elif sop == 15: r = [f32(math.sin(x))] * 4
                elif sop == 16: r = [f32(math.cos(x))] * 4
                elif sop == 7:
                    sx, sy, sw = Cs[0], Cs[1], Cs[3]
                    r = [1.0, max(sx, 0.0), (2.0 ** (min(max(sw, -127.996), 127.996) * math.log2(max(sy, 1e-10)))) if sx > 0 else 0.0, 1.0]
                else:
                    if trace: print(f'  @{pc} unhandled sca op {sop}')
                if r is not None:
                    if sat: r = [min(max(v_, 0.0), 1.0) for v_ in r]
                    own = (not vres and dst != 0x1F) or sdst != 0x3F
                    if not vres and dst != 0x1F: self.write(st, 'o', dst & 15, sm, r, d0, test)
                    if sdst != 0x3F: self.write(st, 'r', sdst & 31, sm, r, d0, test)
                    if self.mode != 'ours' and not own and (dst != 0x1F or upd0 or upd1):
                        self.write(st, 'cc', ccsel, sm, r, d0, test)
                    if trace: print(f'  @{pc} sca{sop} -> {[round(v_, 4) for v_ in r]} o{dst if not vres else "-"} r{sdst}')
            if hook: hook(pc, st)
            if d3 & 1 and nxt == pc + 1: break
            pc = nxt
        return st

def main():
    a = sys.argv[1:]
    d, seq = a[0], a[1]
    trace = '--trace' in a
    only = int(a[a.index('--vert') + 1]) if '--vert' in a else None
    modes = ['rsx', 'ours'] if '--mode' not in a else [a[a.index('--mode') + 1]]
    instrs, consts, verts, meta = load(d, seq)
    start = int(meta['vp_start'][0])
    bits = int(meta.get('branch_bits', ['0'])[0])
    print(f'{len(instrs)} instructions, start slot {start}, {len(verts)} vertices')
    for n, v in enumerate(verts):
        if only is not None and n != only: continue
        res = {}
        for m in modes:
            st = VP(instrs, consts, start, m, bits).run([list(x) for x in v], trace=trace and len(modes) == 1)
            res[m] = st
            p = st['o'][0]
            ndc = [p[i] / p[3] if p[3] else float('nan') for i in range(3)]
            print(f'v{n} [{m:4}] pos {[round(x, 3) for x in p]}  ndc {[round(x, 3) for x in ndc]}  col0 {[round(x, 3) for x in st["o"][1]]}  tc0 {[round(x, 3) for x in st["o"][7]]}')
        if '--diff' in a and len(modes) == 2:
            # step both and report the first instruction where the register files part
            log = {}
            for m in modes:
                seqs = []
                VP(instrs, consts, start, m, bits).run([list(x) for x in v], hook=lambda pc, st, s=seqs: s.append((pc, repr(st['r']), repr(st['cc']), repr(st['a']))))
                log[m] = seqs
            for i, (x, y) in enumerate(zip(log['rsx'], log['ours'])):
                if x != y:
                    print(f'  first divergence at step {i}: rsx @{x[0]} vs ours @{y[0]}')
                    break

if __name__ == '__main__':
    main()
