#!/usr/bin/env python3
"""Find switch jump tables whose targets the PPU lift cannot reach.

    check_jumptables.py <EBOOT.elf> <lifted dir (ppu_recomp_*.cpp)> <EBOOT.functions.json> [code_end hex]

A switch compiles to `... add rX,rX,rBase; mtctr rX; bctr` with the table of signed 32-bit offsets
(relative to the table) right after the bctr. ppu_lifter turns a table it recognises into
`switch ((uint32_t)ctr) { case <target>: goto loc_<target>; ... default: ppu_icall_inl }`; a
table it misses leaves the bctr to the indirect-call dispatch, which only knows registered
function entries -- a target that is neither a case nor an entry is an "unresolved indirect
call" at run time (the Kenzan chapter-1 crash: func_009E56D0, 29 cases). This lists every inline
table in the code and the targets nothing covers."""
import json, re, struct, sys, glob, os

def main():
    elf_path, rdir, fjson = sys.argv[1:4]
    code_end = int(sys.argv[4], 16) if len(sys.argv) > 4 else None
    d = open(elf_path, 'rb').read()
    if '@' in sys.argv[1] or elf_path.endswith('.bin'):
        # a PRX image from lift_firmware_module.py (meta/<stem>_image.bin): raw, loaded at RAW_BASE
        text = (0, int(os.environ['RAW_BASE'], 16), len(d))
    else:
        phoff = struct.unpack('>Q', d[0x20:0x28])[0]; n = struct.unpack('>H', d[0x38:0x3A])[0]
        text = None
        for i in range(n):
            o = phoff + i * 0x38
            t, fl, off, va, pa, fs, ms, al = struct.unpack('>IIQQQQQQ', d[o:o + 0x38])
            if t == 1 and (fl & 1) and text is None:
                text = (off, va, fs)
    off, va, fs = text
    end = va + fs if code_end is None else code_end
    def w(a): return struct.unpack_from('>I', d, off + a - va)[0]

    # what the lift covers: switch cases and registered entries
    cases = set()
    for f in glob.glob(os.path.join(rdir, '*recomp*.c*')):
        for m in re.finditer(r'case 0x([0-9A-Fa-f]{8})u: goto loc_', open(f, errors='replace').read()):
            cases.add(int(m.group(1), 16))
    j = json.load(open(fjson))
    entries = set()
    def walk(x):
        if isinstance(x, dict):
            for k in ('start', 'addr', 'entry', 'address'):
                if k in x and isinstance(x[k], (int, str)):
                    v = x[k]; entries.add(int(v, 16) if isinstance(v, str) else v)
            for v in x.values(): walk(v)
        elif isinstance(x, list):
            if len(x) >= 1 and all(isinstance(y, (int, str)) for y in x[:2]):
                try:
                    v = x[0]; entries.add(int(v, 16) if isinstance(v, str) else int(v))
                except (ValueError, TypeError):
                    pass
            for v in x: walk(v)
    walk(j)

    BCTR = 0x4E800420
    tables = bad_tables = 0
    uncovered_total = 0
    for a in range(va, end - 8, 4):
        if w(a) != BCTR:
            continue
        # mtctr within the 4 words before: a computed jump, not a plain return path
        if not any((w(a - 4 * k) & 0xFC1FFFFF) == 0x7C0903A6 for k in range(1, 5)):
            continue
        base = a + 4
        tg = []
        p = base
        while p < end:
            e = struct.unpack('>i', struct.pack('>I', w(p)))[0]
            t = base + e
            if e == 0 or e & 3 or abs(e) > 0x20000 or not (va <= t < end):
                break
            tg.append(t); p += 4
        if len(tg) < 2:
            continue
        tables += 1
        miss = sorted({t for t in tg if t not in cases and t not in entries})
        if miss:
            bad_tables += 1; uncovered_total += len(miss)
            print(f'bctr 0x{a:08X}: table of {len(tg)} at 0x{base:08X}, {len(miss)} target(s) uncovered: '
                  + ' '.join(f'0x{t:08X}' for t in miss[:8]) + (' ...' if len(miss) > 8 else ''))
    print(f'{tables} inline jump tables; {bad_tables} with uncovered targets ({uncovered_total} targets); '
          f'{len(cases)} switch cases and {len(entries)} entries in the lift')

main()
