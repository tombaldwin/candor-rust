#!/usr/bin/env python3
"""run.py — the cfg-twin runner (soundness/cfgtwin/run.py) pointed at THIS directory's fx/ and expect.tsv.

    python3 soundness/macropath/run.py <candor-scan> [--col pub|main|cand] [--cells 'l1b*'] [--gates] [--record]
    python3 soundness/macropath/run.py --exec [--record]        # build + run every probe: executed ground truth
    python3 soundness/macropath/run.py --live <candor-scan> [cells]   # is each chained cell's join LIVE on this arm? (group G)

Not a copy: it imports the cfgtwin runner and rebinds its two path constants, so a fix to the runner is a fix here.
"""
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('cfgtwin_run', os.path.join(HERE, '..', 'cfgtwin', 'run.py'))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.HERE = HERE
base.FX = os.path.join(HERE, 'fx')
base.EXPECT = os.path.join(HERE, 'expect.tsv')


_reading0 = base.reading


def reading(row):
    """The cfgtwin reading, plus `I`: a row with NO effect and NO Unknown that carries an `invisible` disclosure (SPEC 3.3 prefix
    scope does not gate it, and chaining deletes it, so it is not a pass; cfgtwin folds it into `-`)."""
    r = _reading0(row)
    if r == '-' and row is not None and row.get('invisible'):
        return 'I'
    return r


base.reading = reading


def live(binp, pattern, scratch):
    """MATRIX group G, chained arms: show each chained cell's join LIVE before its chained reading is read. A join is live when scanning the
    consumer with CANDOR_DEPS pointing at the dependency's own report changes ANYTHING in the consumer's report (a function row or the
    `coverage` block), by hand at fixture scale (corpus-chained-judge.py's DEAD-JOIN rule). DEAD means the chained reading carries no
    information beyond the standalone one on this arm. Prints `cell LIVE(functions,coverage)` / `cell DEAD`."""
    import glob
    import json
    out = []
    for c in base.cells(pattern):
        ch = c.get('chain')
        if not ch or ch.get('kind') != 'dep':
            continue
        d = os.path.join(base.FX, c['id'])
        _, rows = base.scan(binp, d, scratch, c['id'] + '.L')
        base.scan(binp, os.path.join(d, ch['dep_dir']), scratch, c['id'] + '.Ldep')
        reps = glob.glob(os.path.join(scratch, c['id'] + '.Ldep', 'r.*.scan.json'))
        _, crows = base.scan(binp, d, scratch, c['id'] + '.Lch', {'CANDOR_DEPS': os.pathsep.join(reps)})

        def rep(tag):
            fs = [f for f in glob.glob(os.path.join(scratch, c['id'] + tag, 'r.*.scan.json'))]
            return json.load(open(fs[0])) if fs else {}
        a, b = rep('.L'), rep('.Lch')
        diff = [k for k in ('functions', 'coverage', 'analyzed', 'resolves', 'excluded') if json.dumps(a.get(k), sort_keys=True) != json.dumps(b.get(k), sort_keys=True)]
        out.append('%-10s %s' % (c['id'], ('LIVE(%s)' % ','.join(diff)) if diff else 'DEAD'))
    return out


if __name__ == '__main__':
    if len(sys.argv) > 2 and sys.argv[1] == '--live':
        # python3 run.py --live <candor-scan> [cells-glob]
        import tempfile
        sc = os.environ.get('CFGTWIN_SCRATCH') or tempfile.mkdtemp(prefix='macropath-live-')
        os.makedirs(sc, exist_ok=True)
        print('\n'.join(live(os.path.abspath(sys.argv[2]), sys.argv[3] if len(sys.argv) > 3 else '', sc)))
        sys.exit(0)
    sys.exit(base.main())
