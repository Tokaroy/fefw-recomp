# SPDX-License-Identifier: GPL-3.0-or-later
"""program.Workers: a lifter worker that dies does not hang the lift (issue #105). Run: python3 recomp/tests/lift_workers_test.py"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
from nxrecomp import program  # noqa: E402

_spec = None


def init(spec):
    global _spec
    _spec = spec


def die():
    os._exit(1)                         # like a native crash: the process is gone, nothing is raised


def square(i):
    return i * i + _spec['add']


def die_once(i):
    """Task 5 kills its worker `deaths` times in all (one marker file per death), then works."""
    if i == 5:
        for k in range(_spec['deaths']):
            try:
                os.close(os.open(os.path.join(_spec['dir'], 'died%d' % k), os.O_CREAT | os.O_EXCL))
            except FileExistsError:
                continue
            die()
    return square(i)


def always_die(i):
    if i == 5:
        die()
    return square(i)


def fail(i):
    if i == 5:
        raise ValueError('task 5')
    return i


def main():
    want = [i * i + 1 for i in range(40)]
    for jobs, deaths, left in ((1, 0, 1), (8, 0, 8), (8, 1, 6), (8, 3, 3)):
        with tempfile.TemporaryDirectory() as d:
            lines = []
            w = program.Workers(jobs, {'add': 1, 'dir': d, 'deaths': deaths}, lines.append, init)
            try:
                assert w.map(die_once, range(40)) == want, (jobs, deaths)
                assert w.jobs == left and len(lines) == deaths, (w.jobs, lines)
                assert all(s.startswith('nxrecomp: warning:') for s in lines), lines    # what fefw_build.py passes on
                w.reset({'add': 2})
                assert w.map(square, range(40)) == [i + 1 for i in want] and w.jobs == left
            finally:
                w.close()

    for fn, error in ((always_die, RuntimeError), (fail, ValueError)):
        w = program.Workers(4, {'add': 1}, lambda s: None, init)
        try:
            w.map(fn, range(40))
            raise AssertionError('%s: no error' % fn.__name__)
        except error:
            pass
        finally:
            w.close()
    print('lift_workers_test: ok')


if __name__ == '__main__':
    main()
