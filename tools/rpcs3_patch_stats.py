import sys, collections
sys.path.insert(0, sys.argv[1]); from rpcs3_patch_list import load
d = load(sys.argv[2]); e = d[sys.argv[3]]
def flat(lst):
    for it in lst:
        if it[0] == 'load': yield from flat(it[1])
        else: yield it
for name in sys.argv[4:]:
    p = e[name]; items = list(flat(p['Patch']))
    t = collections.Counter(i[0] for i in items)
    cal = [i for i in items if i[0] == 'calloc']
    strs = collections.Counter(i[2] for i in items if isinstance(i[2], str) and i[0] not in ('utf8','c_utf8'))
    print(name, len(items), dict(t)); print('  calloc count', len(cal), 'instr total', sum(int(i[2]) for i in cal))
    print('  string values', dict(strs)); print('  config', p.get('Config Values'))
    nz = sorted({int(i[1]) for i in items if not isinstance(i[1], str) and int(i[1])})
    print('  addr range', hex(nz[0]), hex(nz[-1]), len(nz))
