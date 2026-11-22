# SPDX-License-Identifier: GPL-3.0-or-later
"""apply_exefs_patches: records in the alignment gaps widen the segment before. Run: python game-build/release/exefs_gap_test.py"""
import hashlib
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fefw_build as f  # noqa: E402


def make(segs, body):
    h = bytearray(0x100)
    h[0:4] = b'NSO0'
    for i, (fo, mo, sz) in enumerate(segs):
        h[0x10 + 0x10 * i:0x14 + 0x10 * i] = fo.to_bytes(4, 'little')
        h[0x14 + 0x10 * i:0x18 + 0x10 * i] = mo.to_bytes(4, 'little')
        h[0x18 + 0x10 * i:0x1C + 0x10 * i] = sz.to_bytes(4, 'little')
        h[0x60 + 4 * i:0x64 + 4 * i] = sz.to_bytes(4, 'little')
    return bytes(h) + body


def seg(o, i):
    fo, mo, sz = (int.from_bytes(o[0x10 + 0x10 * i + k:0x14 + 0x10 * i + k], 'little') for k in (0, 4, 8))
    return fo, mo, sz


def run(recs):
    segs = [(0x100, 0, 0xD00), (0x100 + 0xD00, 0x1000, 0x800), (0x100 + 0x1500, 0x2000, 0x400)]
    p = tempfile.mktemp()
    open(p, 'wb').write(make(segs, b'T' * 0xD00 + b'R' * 0x800 + b'D' * 0x400 + b'TAIL'))
    f.apply_exefs_patches(p, [('m', 'w', recs)])
    o = open(p, 'rb').read()
    os.remove(p)
    return o


def main():
    o = run([(0x10, b'\xcc')])                                   # in-segment only: nothing widens
    assert seg(o, 0) == (0x100, 0, 0xD00) and len(o) == 0x100 + 0x1900 + 4
    o = run([(0xE00, b'\xaa' * 4), (0x1FFC, b'\xbb' * 8)])       # both gaps; the second record runs on into data
    assert seg(o, 0) == (0x100, 0, 0x1000) and seg(o, 1) == (0x1100, 0x1000, 0x1000) and seg(o, 2) == (0x2100, 0x2000, 0x400)
    assert o[-4:] == b'TAIL'
    assert o[0x100 + 0xE00:0x100 + 0xE04] == b'\xaa' * 4 and o[0x100 + 0x1FFC:0x100 + 0x2004] == b'\xbb' * 8
    for i in range(3):
        fo, _, sz = seg(o, i)
        assert o[0xA0 + 0x20 * i:0xC0 + 0x20 * i] == hashlib.sha256(o[fo:fo + sz]).digest()
    try:
        run([(0x2500, b'x')])                                    # past data: still an error
        assert False
    except f.Fail:
        pass
    print('exefs_gap_test: ok')


if __name__ == '__main__':
    main()
