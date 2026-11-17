# SPDX-License-Identifier: GPL-3.0-or-later
"""Turns a recording of your own play into a route script with host-time waits (docs/PERF_ROUTES.md, "Record your own route").

    python -I tools/perf/record_to_route.py <rec dir> [-o tools/perf/routes/my_route.tt] [--scene name:start_s:end_s ...]

<rec dir> is the [test] out folder of a recording run ([test] record = <dir>/input.nxin, [test] out = <dir>): input.nxin holds the pad of
every frame, trace.jsonl the wall time between frames. Each change of the buttons or the left / right stick becomes a step at the same
time from the start as in the recording (`hold` / `release` / `stick`, with `@sleep` between), so the route repeats what you did at the
same moments whatever the frame rate. Mouse movement is not carried over (play the route with a controller).

--scene name:start:end adds `@log SCENE start/end` at those seconds from the start (e.g. --scene map_pan:62:75); without one the script
measures nothing and you add the SCENE lines by hand.
"""
import argparse
import json
import os
import sys

BUTTONS = ['A', 'B', 'X', 'Y', 'LSTICK', 'RSTICK', 'L', 'R', 'ZL', 'ZR', 'PLUS', 'MINUS', 'LEFT', 'UP', 'RIGHT', 'DOWN']
STICK_DEAD = 4000      # hid units (of 32767): smaller deflections are rounded to 0 so stick noise does not become steps


def read_nxin(path):
    """[(frame, buttons, lx, ly, rx, ry)], state from that frame on."""
    out = []
    with open(path, encoding='utf-8') as f:
        for line in f:
            p = line.split()
            if not p or p[0].startswith('#') or p[0] == 'end':
                continue
            fr = int(p[0])
            b = int(p[1], 16) & 0xFFFF      # the 16 real buttons; the stick-as-button bits above follow from the sticks
            lx, ly, rx, ry = (int(float(x)) for x in p[2:6])
            out.append((fr, b, lx, ly, rx, ry))
    return out


def read_times(path):
    """frame -> ms since the first frame (trace.jsonl: per-frame "ms" = time since the previous frame)."""
    t = {}
    acc = 0.0
    with open(path, encoding='utf-8') as f:
        for line in f:
            try:
                j = json.loads(line)
            except ValueError:
                continue
            if 'f' not in j:
                continue
            acc += float(j.get('ms', 0.0)) if j['f'] > 0 else 0.0
            t[j['f']] = acc
    return t


def q(v):
    return 0 if abs(v) < STICK_DEAD else v


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('rec')
    ap.add_argument('-o', '--out', default='')
    ap.add_argument('--scene', action='append', default=[], help='name:start_s:end_s')
    ap.add_argument('--min-hold-ms', type=float, default=120.0,
                    help='a button is held at least this long (default 120): a quick tap can fall between two game updates and be missed')
    a = ap.parse_args()
    nx = os.path.join(a.rec, 'input.nxin') if os.path.isdir(a.rec) else a.rec
    tr = os.path.join(os.path.dirname(nx), 'trace.jsonl')
    if not os.path.exists(nx) or not os.path.exists(tr):
        print(f'need {nx} and {tr}', file=sys.stderr)
        return 2
    frames = read_nxin(nx)
    times = read_times(tr)
    if not times:
        print('trace.jsonl has no frame times', file=sys.stderr)
        return 2
    last_f = max(times)

    def at(fr):
        while fr not in times and fr > 0:
            fr -= 1
        return times.get(fr, 0.0)

    events = []     # (ms, order, text)
    prev = (0, 0, 0, 0, 0)
    down_at = {}    # button bit -> ms it went down
    for fr, b, lx, ly, rx, ry in frames:
        t = at(fr)
        pb, plx, ply, prx, pry = prev
        up = pb & ~b
        down = b & ~pb
        for i in range(len(BUTTONS)):
            if up >> i & 1:   # released no sooner than min_hold after it went down
                events.append((max(t, down_at.pop(i, t) + a.min_hold_ms), 0, 'release ' + BUTTONS[i]))
        if down:
            for i in range(len(BUTTONS)):
                if down >> i & 1:
                    down_at[i] = t
            events.append((t, 1, 'hold ' + '+'.join(n for i, n in enumerate(BUTTONS) if down >> i & 1)))
        for name, (x, y), (px, py) in (('left', (lx, ly), (plx, ply)), ('right', (rx, ry), (prx, pry))):
            x, y = q(x), q(y)
            if (x, y) != (q(px), q(py)):
                events.append((t, 2, f'stick {name} {x / 32767:.3f} {y / 32767:.3f}'))
        prev = (b, lx, ly, rx, ry)
    for sc in a.scene:
        name, s0, s1 = sc.split(':')
        events.append((float(s0) * 1000.0, 3, f'@log SCENE start {name}'))
        events.append((float(s1) * 1000.0, -1, f'@log SCENE end {name}'))
    events.sort(key=lambda e: (e[0], e[1]))

    lines = ['# Route recorded from play (tools/perf/record_to_route.py ' + ' '.join(sys.argv[1:]) + ').',
             f'# {len(frames)} pad changes over {last_f + 1} frames, {at(last_f) / 1000.0:.1f} s. Waits are host time.', '', '@timeout 0']
    now = 0.0
    for t, _, text in events:
        d = int(round(t - now))
        if d > 0:
            lines.append(f'@sleep {d}')
            now += d
        lines.append(text)
    tail = int(round(at(last_f) - now))
    if tail > 0:
        lines.append(f'@sleep {tail}')
    lines += ['release all', 'stick left 0 0', 'stick right 0 0', '@pass', '']
    text = '\n'.join(lines)
    if a.out:
        with open(a.out, 'w', encoding='utf-8', newline='\n') as f:
            f.write(text)
        print(f'{a.out}: {len(events)} steps, {now / 1000.0:.1f} s')
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
