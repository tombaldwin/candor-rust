#!/usr/bin/env python3
"""run.py — re-render the cfg-twin matrix's readings for ONE candor-scan binary and diff them against expect.tsv.

    python3 soundness/cfgtwin/run.py <candor-scan>                    # a fix lane's per-round check (column: cand)
    python3 soundness/cfgtwin/run.py <candor-scan> --col pub --record # record this binary as the `pub` column
    python3 soundness/cfgtwin/run.py --exec                           # rebuild every fixture, run every probe, compare/record exec
    python3 soundness/cfgtwin/run.py <bin> --cells 'd3*' --gates      # a subset, with gate exit codes

What is held constant across arms (MATRIX.md section 0): the fixture bytes (read-only here), a FRESH CANDOR_CACHE_DIR per
(arm, cell), default flags, `--out <scratch>/r` so nothing lands in the fixture tree.

Readings are the probe's row in functions[]: `-` ABSENT or no effect and no Unknown; `E` a concrete effect; `U` Unknown;
`E+U` both. Gate column: `U<rc>F<rc>` = exit code of `deny Unknown <probe>` / `deny Fs <probe>` (SPEC 3.3 prefix scope).

Chained cells add probe rows named `<probe>@chained` (and, for the cache cell, `@rescan` / `@rescan-incr`); the join is proven LIVE
by the chained row (or the consumer's coverage) differing from the unchained scan, never assumed.

expect.tsv columns: cell probe exec pub main cand gate_pub gate_main gate_cand. An EMPTY cell means "not recorded". The runner exits
1 when a recorded cell differs from the measured one, 0 when every recorded cell matches. It never edits expect.tsv without --record.
Scratch goes under $CFGTWIN_SCRATCH (default: a mkdtemp under $TMPDIR), removed at exit unless --keep.
"""
import argparse
import concurrent.futures as cf
import fnmatch
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FX = os.path.join(HERE, 'fx')
EXPECT = os.path.join(HERE, 'expect.tsv')
COLS = ['cell', 'probe', 'exec', 'pub', 'main', 'cand', 'gate_pub', 'gate_main', 'gate_cand']
SIDE = ('.callgraph.json', '.hierarchy.json', '.locs.json')


def cells(pattern):
    out = []
    for d in sorted(os.listdir(FX)):
        if pattern and not any(fnmatch.fnmatch(d, p) for p in pattern.split(',')):
            continue
        out.append(json.load(open(os.path.join(FX, d, 'cell.json'))))
    return out


def reading(row):
    if row is None:
        return '-'
    inf = [e for e in (row.get('inferred') or row.get('effects') or [])]
    unk = 'Unknown' in inf
    conc = [e for e in inf if e != 'Unknown']
    if conc and unk:
        return 'E+U'
    if conc:
        return 'E'
    return 'U' if unk else '-'


def rows_of(prefix):
    fs = [f for f in glob.glob(prefix + '.*.json') if not f.endswith(SIDE)]
    if not fs:
        return None
    rows = {}
    for f in fs:
        for r in json.load(open(f)).get('functions', []):
            rows.setdefault(r.get('fn') or r.get('name'), r)
    return rows


def scan(binp, target, scratch, tag, env_extra=None, policy=None, flags=()):
    out = os.path.join(scratch, tag)
    os.makedirs(out, exist_ok=True)
    env = dict(os.environ, CANDOR_CACHE_DIR=os.path.join(scratch, tag + '.cache'))
    env.pop('CANDOR_DEPS', None)
    env.update(env_extra or {})
    cmd = [binp, target, '--out', os.path.join(out, 'r')] + list(flags)
    if policy:
        cmd += ['--policy', policy]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env)
    return p.returncode, rows_of(os.path.join(out, 'r'))


def gate_codes(binp, target, scratch, tag, probe, env_extra=None):
    res = ''
    for letter, eff in (('U', 'Unknown'), ('F', 'Fs')):
        pol = os.path.join(scratch, '%s.%s.pol' % (tag, letter))
        open(pol, 'w').write('deny %s %s\n' % (eff, probe))
        rc, _ = scan(binp, target, scratch, '%s.g%s' % (tag, letter), env_extra, policy=pol)
        res += '%s%d' % (letter, rc)
    return res


def find(rows, probe):
    if rows is None:
        return None
    for k, v in rows.items():
        if k == probe or (k and k.endswith('::' + probe)):
            return v
    return None


def measure_cell(binp, c, scratch, want_gates):
    """-> list of (probe, reading, gate)"""
    cid = c['id']
    d = os.path.join(FX, cid)
    ch = c.get('chain')
    out = []
    edit_dir = os.path.join(d, 'edit')
    if ch and ch['kind'] == 'cache-edit':
        # `edit/` holds the state-B replacement the runner copies in by hand. Inside the scanned directory it is a file no `mod`
        # places, which the scanner still ANALYSES as an orphan module, so its text would be state A's text (R1853's member
        # evidence is keyed on the owner's leaf). Scan a copy without it; `edit_dir` still names the fixture's own.
        d = os.path.join(scratch, cid + '.src')
        shutil.rmtree(d, ignore_errors=True)
        shutil.copytree(os.path.join(FX, cid), d, ignore=shutil.ignore_patterns('target', 'edit'))
    rc, rows = scan(binp, d, scratch, cid)
    if rows is None:
        return [(p['name'], 'NOREPORT(rc=%d)' % rc, '') for p in c['probes']]
    for p in c['probes']:
        g = gate_codes(binp, d, scratch, cid + '.' + p['name'], p['name']) if want_gates else ''
        out.append((p['name'], reading(find(rows, p['name'])), g))
    if ch and ch['kind'] == 'dep':
        dep = os.path.join(d, ch['dep_dir'])
        rc2, drows = scan(binp, dep, scratch, cid + '.dep')
        reps = [f for f in glob.glob(os.path.join(scratch, cid + '.dep', 'r.*.scan.json'))]
        env = {'CANDOR_DEPS': os.pathsep.join(reps)}
        rc3, crows = scan(binp, d, scratch, cid + '.chained', env)
        for p in c['probes']:
            g = gate_codes(binp, d, scratch, cid + '.' + p['name'] + '.ch', p['name'], env) if want_gates else ''
            r = reading(find(crows, p['name'])) if crows is not None else 'NOREPORT'
            # whether the join is live is shown by the e13c control (chained != unchained there), not by this cell
            live = '' if crows is None else ('' if json.dumps(crows, sort_keys=True) != json.dumps(rows, sort_keys=True) else ' (chained rows == unchained rows)')
            out.append((p['name'] + '@chained', r + live, g))
    elif ch and ch['kind'] == 'consumer':
        reps = glob.glob(os.path.join(scratch, cid, 'r.*.scan.json'))
        cons = os.path.join(d, 'cons')
        env = {'CANDOR_DEPS': os.pathsep.join(reps)}
        rcu, urows = scan(binp, cons, scratch, cid + '.consU')           # consumer, UNCHAINED
        rcc, crows = scan(binp, cons, scratch, cid + '.consC', env)      # consumer, chained against this arm's producer report
        cp = ch['consumer_probe']
        r = reading(find(crows, cp)) if crows is not None else 'NOREPORT'
        ru = reading(find(urows, cp)) if urows is not None else 'NOREPORT'
        g = gate_codes(binp, cons, scratch, cid + '.' + cp, cp, env) if want_gates else ''
        live = '' if (crows is not None and json.dumps(crows, sort_keys=True) != json.dumps(urows, sort_keys=True)) else ' (chained rows == unchained rows)'
        out.append((cp + '@chained', r + live + (' [unchained %s]' % ru), g))
    elif ch and ch['kind'] == 'cache-edit':
        # state A scan above used the cache dir <scratch>/<cid>.cache; swap in the edit and rescan with the SAME cache dir
        w = os.path.join(scratch, cid + '.work')
        shutil.copytree(d, w, ignore=shutil.ignore_patterns('target', 'edit'))
        for tag, flags in (('rescan', ()), ('rescan-incr', ('--incremental',))):
            w2 = w + '.' + tag
            shutil.copytree(w, w2)
            cache = os.path.join(scratch, cid + '.' + tag + '.cache')
            env = dict(os.environ, CANDOR_CACHE_DIR=cache)
            o1 = os.path.join(scratch, cid + '.' + tag + '.A')
            subprocess.run([binp, w2, '--out', os.path.join(o1, 'r')] + list(flags), capture_output=True, text=True, env=env)
            shutil.copy(os.path.join(edit_dir, ch['edit_file']), os.path.join(w2, 'src', ch['edit_file']))
            o2 = os.path.join(scratch, cid + '.' + tag + '.B')
            subprocess.run([binp, w2, '--out', os.path.join(o2, 'r')] + list(flags), capture_output=True, text=True, env=env)
            rows2 = rows_of(os.path.join(o2, 'r'))
            rowsA = rows_of(os.path.join(o1, 'r'))
            out.append((c['probes'][0]['name'] + '@' + tag, reading(find(rows2, c['probes'][0]['name'])) if rows2 is not None else 'NOREPORT',
                        '' if not want_gates else ''))
            out.append((c['probes'][0]['name'] + '@' + tag + '.stateA', reading(find(rowsA, c['probes'][0]['name'])) if rowsA is not None else 'NOREPORT', ''))
    return out


# ------------------------------------------------------------------------------------------------ exec ground truth
def exec_truth(cs, scratch):
    """Build every fixture (stable toolchain, ONE shared target dir, distinct package names) and run each probe with MARK set."""
    tgt = os.path.join(scratch, 'target')
    env = dict(os.environ, CARGO_TARGET_DIR=tgt, RUSTFLAGS='')
    env['RUSTUP_TOOLCHAIN'] = os.environ.get('CANDOR_GEN_TOOLCHAIN', 'stable')
    res = {}
    shutil.rmtree(os.path.join(scratch, 'work'), ignore_errors=True)
    shutil.rmtree(os.path.join(scratch, 'marks'), ignore_errors=True)
    for c in cs:
        cid = c['id']
        w = os.path.join(scratch, 'work', cid)
        shutil.copytree(os.path.join(FX, cid), w)

        def build(dirp, name, extra=()):
            p = subprocess.run(['cargo', 'build', '--offline', '-q'] + list(extra), cwd=dirp, capture_output=True, text=True, env=env)
            return p

        def failure(p):
            """`NOCOMPILE(Exxxx)` for a cell whose meta says a rustc error is a RECORDED answer (macropath's must-not-compile cells), else
            BUILD-FAIL (a broken fixture). A lint error has no code: the first `error:` line stands in."""
            if not c.get('nocompile_ok'):
                return 'BUILD-FAIL'
            m = re.search(r'error\[(E\d+)\]', p.stderr)
            if m:
                return 'NOCOMPILE(%s)' % m.group(1)
            m = re.search(r'^error: (.*)$', p.stderr, re.M)
            return 'NOCOMPILE(%s)' % (m.group(1)[:60] if m else '?')

        runs = []   # (probe-name, binary-dir-name, bin crate name, argv probe)
        targets = [(w, 'cf_' + cid, [(p['name'], p['name']) for p in c['probes']])]
        ch = c.get('chain')
        if ch and ch['kind'] == 'consumer':
            targets.append((os.path.join(w, 'cons'), 'cons_' + cid, [(ch['consumer_probe'] + '@chained', ch['consumer_probe'])]))
        for dirp, bname, probes in targets:
            p = build(dirp, bname)
            if p.returncode != 0:
                for pn, _ in probes:
                    res[(cid, pn)] = failure(p)
                if not c.get('nocompile_ok'):
                    sys.stderr.write('BUILD FAIL %s\n%s\n' % (cid, p.stderr[-1500:]))
                else:
                    sys.stderr.write('NOCOMPILE %s: %s\n' % (cid, res[(cid, probes[0][0])]))
                if c.get('features'):
                    for pn, _ in probes:
                        res[(cid, pn + '+std')] = failure(p)
                continue
            if c.get('lib'):
                # a lib cell is BUILD-ONLY: its ground truth is "compiles" (the dependency's binding minted the fn the probe calls)
                for pn, _ in probes:
                    res[(cid, pn)] = 'builds'
                if c.get('features'):
                    p2 = build(dirp, bname, ['--features', ','.join(c['features'])])
                    for pn, _ in probes:
                        res[(cid, pn + '+std')] = 'builds' if p2.returncode == 0 else failure(p2)
                continue
            for pn, argv in probes:
                mark = os.path.join(scratch, 'marks', '%s.%s' % (cid, pn))
                os.makedirs(os.path.dirname(mark), exist_ok=True)
                if os.path.exists(mark):
                    os.remove(mark)
                r = subprocess.run([os.path.join(tgt, 'debug', bname), argv], env=dict(os.environ, MARK=mark), capture_output=True, text=True)
                if r.returncode != 0:
                    res[(cid, pn)] = 'RUN-FAIL'
                else:
                    res[(cid, pn)] = 'Fs' if os.path.exists(mark) else 'none'
        if ch and ch['kind'] == 'dep':
            for p_ in c['probes']:
                res[(cid, p_['name'] + '@chained')] = res.get((cid, p_['name']), '?')
        if ch and ch['kind'] == 'cache-edit':
            # state A (pure, exec none) is the fixture as checked in; state B is the edit, built in a second copy
            w2 = w + '.B'
            shutil.copytree(w, w2)
            shutil.copy(os.path.join(w2, 'edit', ch['edit_file']), os.path.join(w2, 'src', ch['edit_file']))
            p = build(w2, 'cf_' + cid)
            pn = c['probes'][0]['name']
            mark = os.path.join(scratch, 'marks', '%s.%s.B' % (cid, pn))
            if os.path.exists(mark):
                os.remove(mark)
            if p.returncode == 0:
                subprocess.run([os.path.join(tgt, 'debug', 'cf_' + cid), pn], env=dict(os.environ, MARK=mark), capture_output=True)
            for tag in ('rescan', 'rescan-incr'):
                res[(cid, pn + '@' + tag)] = 'Fs' if os.path.exists(mark) else 'none'
                res[(cid, pn + '@' + tag + '.stateA')] = res.get((cid, pn), '?')
        # keep the shared target small
        shutil.rmtree(w, ignore_errors=True)
        shutil.rmtree(w + '.B', ignore_errors=True)
    return res


def load_expect():
    rows = {}
    order = []
    if os.path.exists(EXPECT):
        for l in open(EXPECT):
            if l.startswith('#') or not l.strip() or l.startswith('cell\tprobe\t'):
                continue
            p = l.rstrip('\n').split('\t')
            p += [''] * (len(COLS) - len(p))
            rows[(p[0], p[1])] = dict(zip(COLS, p))
            order.append((p[0], p[1]))
    return rows, order


def save_expect(rows, order):
    with open(EXPECT, 'w') as f:
        f.write('# soundness/cfgtwin/expect.tsv — recorded by run.py (--record / --exec --record). Empty = not recorded (cand is filled by the fix lanes).\n')
        f.write('\t'.join(COLS) + '\n')
        for k in sorted(set(order) | set(rows), key=lambda x: (x[0], x[1])):
            f.write('\t'.join(rows[k].get(c, '') for c in COLS) + '\n')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('binary', nargs='?')
    ap.add_argument('--col', default='cand', choices=['pub', 'main', 'cand'])
    ap.add_argument('--cells', default='')
    ap.add_argument('--gates', action='store_true')
    ap.add_argument('--record', action='store_true')
    ap.add_argument('--exec', action='store_true', help='rebuild + run every fixture and compare/record the exec column')
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--jobs', type=int, default=3)
    a = ap.parse_args()
    cs = cells(a.cells)
    scratch = os.environ.get('CFGTWIN_SCRATCH') or tempfile.mkdtemp(prefix='cfgtwin-run-')
    os.makedirs(scratch, exist_ok=True)
    exp, order = load_expect()
    bad = 0
    try:
        if a.exec:
            truth = exec_truth(cs, scratch)
            for (cid, pn), v in sorted(truth.items()):
                r = exp.setdefault((cid, pn), {'cell': cid, 'probe': pn})
                order.append((cid, pn))
                if a.record:
                    r['exec'] = v
                elif r.get('exec') and r['exec'] != v:
                    print('EXEC DIFF %s %s: expect %s, measured %s' % (cid, pn, r['exec'], v)); bad += 1
                if v in ('BUILD-FAIL', 'RUN-FAIL'):  # NOCOMPILE(..) and `builds` are recorded answers
                    print('BROKEN FIXTURE %s %s: %s' % (cid, pn, v)); bad += 1
            if a.record:
                save_expect(exp, order)
            print('exec: %d probes measured, %d problem(s)' % (len(truth), bad))
            return 1 if bad else 0
        if not a.binary:
            ap.error('a candor-scan binary is required (or --exec)')
        binp = os.path.abspath(a.binary)
        assert os.access(binp, os.X_OK), binp
        with cf.ThreadPoolExecutor(a.jobs) as ex:
            futs = {c['id']: ex.submit(measure_cell, binp, c, scratch, a.gates) for c in cs}
            meas = {cid: f.result() for cid, f in futs.items()}
        unrec = 0
        for cid in sorted(meas):
            for pn, rd_full, g in meas[cid]:
                rd, _, note = rd_full.partition(' ')    # the reading is the first token; the rest is an annotation, never recorded
                r = exp.setdefault((cid, pn), {'cell': cid, 'probe': pn})
                order.append((cid, pn))
                ex_ = r.get('exec', '')
                if a.record:
                    r[a.col] = rd
                    if a.gates:
                        r['gate_' + a.col] = g
                rec = r.get(a.col, '')
                state = 'OK' if rec == rd else ('UNRECORDED' if rec == '' else 'DIFF(expect %s)' % rec)
                if state.startswith('DIFF'):
                    bad += 1
                if state == 'UNRECORDED':
                    unrec += 1
                print('%-10s %-22s exec=%-4s %-4s %s %s' % (cid, pn, ex_ or '?', a.col + '=' + rd, ('gate=' + g) if g else '', state + (' ' + note if note else '')))
        if a.record:
            save_expect(exp, order)
        print('cfgtwin: %d probe rows, %d differ from expect.tsv[%s], %d unrecorded' % (sum(len(v) for v in meas.values()), bad, a.col, unrec))
        return 1 if bad else 0
    finally:
        if not a.keep and not os.environ.get('CFGTWIN_SCRATCH'):
            shutil.rmtree(scratch, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
