#!/usr/bin/env python3
"""Resolve [kzbt] backtraces (e<off> = engine exe, m<off> = game module) to function names.
usage: kz_symbolize.py [log]   (inside nix-shell: needs nm)"""
import bisect, re, subprocess, sys, os
D = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
log = sys.argv[1] if len(sys.argv) > 1 else os.path.join(D, 'work', 'logs', 'last_run_err.txt')
bins = {'e': os.path.join(D, 'work', 'build', 'KenzanRecomp'), 'm': os.path.join(D, 'work', 'build', os.environ.get('KZ_MODULE', 'libInfamous2Game_60.so'))}
tabs = {}
for k, b in bins.items():
    syms = []
    for l in subprocess.run(['nm', '-C', '--defined-only', b], capture_output=True, text=True).stdout.splitlines():
        p = l.split(' ', 2)
        if len(p) == 3 and p[1] in 'tTwW':
            syms.append((int(p[0], 16), p[2].split('(')[0]))
    syms.sort(); tabs[k] = ([a for a, _ in syms], syms)
NOISE = {'data_start', 'thread_trampoline', '_start', 'main', '_init', '_fini'}
for l in open(log, errors='replace'):
    m = re.match(r'\[kzbt\] tid=(\d+) comm=([^:]*):(.*)$', l)
    if not m: continue
    names = []
    for h in m.group(3).split():
        if '+' in h:                      # a shared library frame: lib+off[:symbol]
            lib, rest = h.split('+', 1)
            n = rest.split(':', 1)[1] if ':' in rest else lib
            if not (names and names[-1] == n): names.append(n)
            continue
        k, off = h[0], h[1:]
        if k not in tabs: continue
        addrs, syms = tabs[k]
        i = bisect.bisect_right(addrs, int(off, 16)) - 1
        n = syms[i][1] if i >= 0 else '?'
        if n in NOISE or (names and names[-1] == n): continue
        names.append(n)
    print(f'{m.group(1)} [{m.group(2)}]: ' + ' < '.join(names[:40]))
