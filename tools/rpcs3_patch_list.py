#!/usr/bin/env python3
"""List the RPCS3 patches for one executable hash. usage: rpcs3_patch_list.py <patch.yml> <PPU-hash>"""
import sys, yaml
class L(yaml.SafeLoader):
    """patch.yml redefines anchors (yaml-cpp keeps the latest); PyYAML refuses unless told."""
    def compose_node(self, parent, index):
        if not self.check_event(yaml.AliasEvent):
            a = self.peek_event().anchor
            if a is not None: self.anchors.pop(a, None)
        return super().compose_node(parent, index)
def load(path): return yaml.load(open(path, encoding='utf-8'), Loader=L)
if __name__ == '__main__':
    e = load(sys.argv[1])[sys.argv[2]]
    for k, v in e.items():
        print(repr(k), '|', v.get('Group'), '|', (v.get('Notes') or '')[:160].replace('\n', ' '),
              '| cfg:', list((v.get('Config Values') or {}).keys()))
