import sys
gt = [l.strip() for l in open("ground-truth.txt") if l.strip()]
got = [l.strip() for l in open(sys.argv[1]) if l.strip()]
def hit(g):
    return any(x == g or x.endswith("::" + g) or g.endswith("::" + x) for x in got)
print(sum(1 for g in gt if hit(g)))
