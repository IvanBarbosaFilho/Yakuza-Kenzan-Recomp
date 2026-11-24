#!/usr/bin/env python3
"""Apply RPCS3 patch.yml entries (be32 + calloc) to a PPU ELF, statically, for the recompiler.

usage: apply_rpcs3_patch.py <patch.yml> <PPU-hash> <in.elf> <out.elf> <caves.json> <cave_base hex> "<entry>"...

RPCS3 semantics (Utilities/bin_patch.cpp, patch_engine::apply):
  be32 <ea> <word>   write a big-endian word at <ea>.
  calloc <ea> <n>    a code cave of n instructions plus a return branch: NOP-filled, its last word
                     `b <ea+4>`, and <ea> becomes `b <cave>`. While the cave still has a NOP slot,
                     every following entry is written to the next slot (whatever its address).
  load <anchor>      the anchor's entries, in place.
RPCS3 allocates each cave in fresh memory; here they are laid out back to back from <cave_base>,
which must be inside a loaded, executable segment of <in.elf> whose bytes are all zero there
(the English patch's 64 KB RWX segment at 0xBF0000 has room after its own code).
caves.json lists every cave: {"start", "end", "entry", "ret"} -- what the function list needs."""
import json, struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpcs3_patch_list import load

NOP = 0x60000000

def flat(lst):
    for it in lst:
        if it[0] == 'load': yield from flat(it[1])
        else: yield it

def main():
    pyml, h, src, dst, cj, base = sys.argv[1:7]
    names = sys.argv[7:]
    entry = load(pyml)[h]
    elf = bytearray(open(src, 'rb').read())
    phoff = struct.unpack('>Q', elf[0x20:0x28])[0]
    phentsize, phnum = struct.unpack('>HH', elf[0x36:0x3a])
    segs = []
    for i in range(phnum):
        o = phoff + i * phentsize
        pt, pf, off, va, pa, fs, ms, al = struct.unpack('>IIQQQQQQ', elf[o:o + 56])
        if pt == 1 and fs: segs.append((va, va + fs, off, pf))
    def foff(ea, n=4):
        for va, end, off, pf in segs:
            if va <= ea and ea + n <= end: return off + ea - va
        raise ValueError(f'0x{ea:x} is not in a loaded segment')
    def rd(ea): return struct.unpack('>I', elf[foff(ea):foff(ea) + 4])[0]
    def wr(ea, w): o = foff(ea); elf[o:o + 4] = struct.pack('>I', w & 0xFFFFFFFF)
    def b(frm, to):
        d = to - frm
        assert -0x2000000 <= d < 0x2000000 and not d & 3, (hex(frm), hex(to))
        return 0x48000000 | (d & 0x03FFFFFC)
    nxt = int(base, 16)
    caves, nwords = [], 0
    for name in names:
        reloc = 0
        for it in flat(entry[name]['Patch']):
            typ, ea, val = it[0], int(it[1]), it[2]
            if reloc and rd(reloc) != NOP: reloc = 0          # the cave is full
            if typ == 'calloc':
                n = int(val)
                cave = nxt; nxt += (n + 1) * 4
                for k in range(n + 1):
                    assert rd(cave + 4 * k) == 0, f'cave space 0x{cave + 4 * k:x} is not free'
                    wr(cave + 4 * k, NOP)
                wr(cave + 4 * n, b(cave + 4 * n, (ea & ~3) + 4))
                wr(ea & ~3, b(ea & ~3, cave))
                caves.append({'start': cave, 'end': cave + 4 * (n + 1), 'entry': ea & ~3, 'ret': (ea & ~3) + 4,
                              'patch': name})
                reloc = cave
                continue
            if typ != 'be32': raise SystemExit(f'unsupported patch type {typ} in {name}')
            if isinstance(val, str): raise SystemExit(f'config value {val!r} in {name}: not supported')
            at = ea
            if reloc: at, reloc = reloc, reloc + 4
            wr(at, int(val)); nwords += 1
    open(dst, 'wb').write(elf)
    json.dump(caves, open(cj, 'w'), indent=1)
    print(f'{len(caves)} caves, 0x{int(base, 16):x}..0x{nxt:x}, {nwords} words written -> {dst}')

if __name__ == '__main__':
    main()
