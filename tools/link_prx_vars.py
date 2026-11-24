#!/usr/bin/env python3
"""Link a PRX's imported VARIABLES to the executable's exported ones, statically, in the relocated
image (lift_prx.py's <stem>_image.bin at <base>), as lv2's loader does at load time.
usage: link_prx_vars.py <prx> <meta dir> <stem> <base hex> <executable ELF>
Kenzan's minigames import ~25 variables of iori_elf (the EBOOT: its managers, heaps) and 2 of its
sys_libc; lift_prx.py links only functions (through HLE stubs), so these stayed 0 and every
minigame started with NULL objects. Both sides have fixed addresses here (the EBOOT's own, the
module's lifted base), so the link is a set of plain stores.
Each imported variable has a reference list {u32 type, u32 addr, u32 addend}... ending at type 0;
type 1 = the 32-bit address (value + addend) at addr, 4 = its low half, 5 = high, 6 = high-adjusted
(RPCS3 ppu_patch_refs)."""
import struct, sys
def be32(b, o): return struct.unpack_from('>I', b, o)[0]
def elf_var_exports(path):
    f = open(path, 'rb').read()
    po = struct.unpack_from('>Q', f, 0x20)[0]; ent, num = struct.unpack_from('>HH', f, 0x36)
    phs = [struct.unpack_from('>IIQQQQQQ', f, po + i * ent) for i in range(num)]
    def rd(va, n=4):
        for p in phs:
            if p[0] == 1 and p[3] <= va < p[3] + p[5]: return f[p[2] + va - p[3]:p[2] + va - p[3] + n]
        raise ValueError(hex(va))
    r32 = lambda va: struct.unpack('>I', rd(va))[0]
    def cstr(va):
        s = b''
        while True:
            c = rd(va, 1)
            if c == b'\0': return s.decode()
            s += c; va += 1
    out = {}
    for p in phs:
        if p[0] != 0x60000002: continue
        ent_s, ent_e = struct.unpack_from('>II', f, p[2] + 16)
        va = ent_s
        while va < ent_e:
            h = rd(va, 0x1C); ln = h[0]
            nfunc, nvar = struct.unpack_from('>HH', h, 6)
            name, nids, addrs = struct.unpack_from('>III', h, 0x10)
            lib = cstr(name) if name else ''
            for i in range(nvar):
                out[(lib, r32(nids + 4 * (nfunc + i)))] = r32(addrs + 4 * (nfunc + i))
            for i in range(nfunc):
                out[('fn', lib, r32(nids + 4 * i))] = r32(addrs + 4 * i)
            va += ln
    return out
def main():
    prx, meta, stem, base, elf = sys.argv[1:6]
    base = int(base, 16)
    sys.path.insert(0, __import__('os').path.join(__import__('os').path.dirname(__import__('os').path.abspath(__file__)), '..', 'engine', 'tools'))
    import lift_prx
    p = lift_prx.Prx(prx)
    mi = p.modinfo_off
    imp_s, imp_e = struct.unpack_from('>II', p.data, mi + 0x2C)
    ip = f'{meta}/{stem}_image.bin'
    img = bytearray(open(ip, 'rb').read())
    rd = lambda va: be32(img, va - base)
    def wr32(va, v): struct.pack_into('>I', img, va - base, v & 0xFFFFFFFF)
    def wr16(va, v): struct.pack_into('>H', img, va - base, v & 0xFFFF)
    def cstr(va):
        o = va - base; return img[o:img.index(b'\0', o)].decode()
    ex = elf_var_exports(elf)
    o, linked, refs, missing = imp_s, 0, 0, []
    while o < imp_e:
        sz = img[o]
        attr, nfunc, nvar = struct.unpack_from('>HHH', img, o + 4)
        lib = cstr(be32(img, o + 0x10))
        fnids = be32(img, o + 0x14)
        vnids, vstubs = struct.unpack_from('>II', img, o + 0x1C)
        todo = []
        # attribute 0x2000: the function NID table goes on with one reference list per function
        # (data holding the function's address: vtables), patched with its OPD (RPCS3
        # ppu_load_imports: "0x2000 seems to be correct flag indicating the presence of additional info")
        if attr & 0x2000:
            for i in range(nfunc):
                ref = rd(fnids + 4 * (nfunc + i))
                if ref: todo.append((('fn', lib, rd(fnids + 4 * i)), ref))
        for i in range(nvar):
            todo.append(((lib, rd(vnids + 4 * i)), rd(vstubs + 4 * i)))
        for key, ref in todo:
            nid = key[-1]
            val = ex.get(key)
            if val is None: missing.append(f'{lib}:{nid:08X}'); continue
            while True:
                t, a, add = rd(ref), rd(ref + 4), rd(ref + 8)
                if t == 0: break
                v = (val + add) & 0xFFFFFFFF
                if t == 1: wr32(a, v)
                elif t == 4: wr16(a, v)
                elif t == 5: wr16(a, v >> 16)
                elif t == 6: wr16(a, (v + 0x8000) >> 16)
                else: raise SystemExit(f'{lib}:{nid:08X}: reference type {t} at 0x{a:08X}')
                refs += 1; ref += 12
            linked += 1
        o += sz if sz else 0x2C
    if missing: raise SystemExit(f'not exported by {elf}: {missing}')
    open(ip, 'wb').write(img)
    print(f'{linked} imported variables and function references linked to {elf.split("/")[-1]} ({refs} references)')
if __name__ == '__main__':
    main()
