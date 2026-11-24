#!/usr/bin/env python3
"""Apply an RPCS3 patch.yml PRX section ("PRX-<hash>-0") to a relocated PRX image (lift_prx.py's
<stem>_image.bin, loaded at <base>), statically, for the recompiler.
usage: apply_prx_patch.py <patch.yml> <section> <meta dir> <stem> <base hex> [name prefix]
RPCS3 semantics (as tools/apply_rpcs3_patch.py): a PRX section's addresses are offsets from its
LOAD segment's start (segment 0 = the image start here: lift_prx places it at base + p_vaddr 0).
calloc caves, which RPCS3 allocates in fresh memory, are appended after the image (16-byte
aligned); the image grows, the caves join <stem>_functions.json, and the cave range is printed
on the last line as "CAVES lo-hi" (lift_firmware_module.py passes it on as PPU_DETOUR_CAVES and
the new code end). Every entry whose name starts with the prefix (default "Framerate -") is applied."""
import json, os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpcs3_patch_list import load
NOP = 0x60000000
def flat(lst):
    for it in lst:
        if it[0] == 'load': yield from flat(it[1])
        else: yield it
def main():
    pyml, sec, meta, stem, base = sys.argv[1:6]
    prefix = sys.argv[6] if len(sys.argv) > 6 else 'Framerate -'
    base = int(base, 16)
    entry = load(pyml)[sec]
    names = [n for n in entry if n.startswith(prefix)]
    if not names: raise SystemExit(f'no "{prefix}*" entry in {sec}')
    ip = os.path.join(meta, stem + '_image.bin')
    img = bytearray(open(ip, 'rb').read())
    orig_len = len(img)
    def rd(ea): return struct.unpack_from('>I', img, ea - base)[0]
    def wr(ea, w):
        o = ea - base
        if o + 4 > len(img): img.extend(b'\0' * (o + 4 - len(img)))
        struct.pack_into('>I', img, o, w & 0xFFFFFFFF)
    def b(frm, to):
        d = to - frm
        assert -0x2000000 <= d < 0x2000000 and not d & 3, (hex(frm), hex(to))
        return 0x48000000 | (d & 0x03FFFFFC)
    lo = nxt = base + ((orig_len + 15) & ~15)
    caves, nwords = [], 0
    for name in names:
        reloc = 0
        for it in flat(entry[name]['Patch']):
            typ, off, val = it[0], int(it[1]), it[2]
            if reloc and rd(reloc) != NOP: reloc = 0
            if typ == 'calloc':
                n = int(val); ea = base + (off & ~3)
                assert ea - base < orig_len, f'calloc at 0x{off:x} outside the image'
                cave = nxt; nxt += (n + 1) * 4
                for k in range(n): wr(cave + 4 * k, NOP)
                wr(cave + 4 * n, b(cave + 4 * n, ea + 4))
                wr(ea, b(ea, cave))
                caves.append({'start': cave, 'end': cave + 4 * (n + 1), 'entry': ea, 'patch': name})
                reloc = cave
                continue
            if typ != 'be32': raise SystemExit(f'unsupported patch type {typ} in {name}')
            if isinstance(val, str): raise SystemExit(f'config value {val!r} in {name}: not supported')
            if reloc: at, reloc = reloc, reloc + 4
            else:
                at = base + off
                assert off + 4 <= orig_len, f'be32 at 0x{off:x} outside the image'
            wr(at, int(val)); nwords += 1
    open(ip, 'wb').write(img)
    fp = os.path.join(meta, stem + '_functions.json')
    fs = json.load(open(fp))
    fs += [{'start': f"0x{c['start']:08X}", 'end': f"0x{c['end']:08X}"} for c in caves]
    json.dump(fs, open(fp, 'w'), indent=1)
    json.dump(caves, open(os.path.join(meta, stem + '_caves.json'), 'w'), indent=1)
    print(f'{sec}: {", ".join(names)}: {len(caves)} caves, {nwords} words')
    print(f'CAVES 0x{lo:08X}-0x{nxt:08X}')
if __name__ == '__main__':
    main()
