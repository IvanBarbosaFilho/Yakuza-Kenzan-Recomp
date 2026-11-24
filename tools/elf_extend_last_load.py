#!/usr/bin/env python3
"""Grow the PT_LOAD that ends the file by <bytes> of zeros (filesz and memsz), for code caves.
usage: elf_extend_last_load.py <in.elf> <out.elf> <bytes hex>"""
import struct, sys
src, dst, add = sys.argv[1], sys.argv[2], int(sys.argv[3], 16)
e = bytearray(open(src, 'rb').read())
phoff = struct.unpack('>Q', e[0x20:0x28])[0]; phentsize, phnum = struct.unpack('>HH', e[0x36:0x3a])
for i in range(phnum):
    o = phoff + i * phentsize
    pt, pf, off, va, pa, fs, ms, al = struct.unpack('>IIQQQQQQ', e[o:o + 56])
    if pt == 1 and fs and off + fs == len(e):
        assert fs == ms, 'the last segment has a bss tail'
        e[o + 32:o + 48] = struct.pack('>QQ', fs + add, ms + add)
        e += bytes(add)
        open(dst, 'wb').write(e)
        print(f'segment 0x{va:x}: 0x{fs:x} -> 0x{fs + add:x} bytes (ends 0x{va + fs + add:x})')
        break
else:
    raise SystemExit('no PT_LOAD ends the file')
