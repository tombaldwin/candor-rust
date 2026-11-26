#!/usr/bin/env python3
"""run.py — the cfg-twin runner (soundness/cfgtwin/run.py) pointed at THIS directory's fx/ and expect.tsv.

    python3 soundness/macropath/run.py <candor-scan> [--col pub|main|cand] [--cells 'l1b*'] [--gates] [--record]
    python3 soundness/macropath/run.py --exec [--record]        # build + run every probe: executed ground truth

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
if __name__ == '__main__':
    sys.exit(base.main())
