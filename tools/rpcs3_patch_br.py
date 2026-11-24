import sys, collections
sys.path.insert(0, sys.argv[1]); from rpcs3_patch_list import load
e = load(sys.argv[2])[sys.argv[3]]
def flat(lst):
    for it in lst:
        if it[0] == 'load': yield from flat(it[1])
        else: yield it
items = list(flat(e[sys.argv[4]]['Patch']))
c = collections.Counter(); ex = {}
reloc = 0; left = 0
for i in items:
    if i[0] == 'calloc': left = int(i[2]); continue
    w = int(i[2]); op = w >> 26
    incave = left > 0 and int(i[1]) == 0
    if incave: left -= 1
    if op in (16, 18):
        k = ('cave' if incave else 'text', op, 'AA' if w & 2 else 'rel', 'LK' if w & 1 else '')
        c[k] += 1; ex.setdefault(k, (hex(int(i[1])), hex(w)))
    elif not incave and int(i[1]) == 0: c['zero-off outside cave'] += 1
for k, v in c.items(): print(k, v, ex.get(k))
