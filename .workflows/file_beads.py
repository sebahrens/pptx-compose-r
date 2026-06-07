import json, os, re, subprocess, sys

beads = json.load(open('/tmp/wf_result.json'))['beads']
env = {**os.environ, 'BD_NON_INTERACTIVE': '1'}
keymap = {}  # local key -> real bd id

def run(args):
    return subprocess.run(args, capture_output=True, text=True, env=env, cwd='/Users/seb/projects/pptx-compose')

# 1) create
for b in beads:
    r = run(['bd', 'create',
             '--title', b['title'],
             '--type', b['type'],
             '--priority', str(b['priority']),
             '--description', b['description'],
             '--acceptance', b['acceptance']])
    m = re.search(r'(pptx-compose-[a-z0-9]+)', r.stdout + r.stderr)
    if not m:
        print('CREATE FAILED for', b['key'], '\nSTDOUT:', r.stdout, '\nSTDERR:', r.stderr)
        sys.exit(1)
    keymap[b['key']] = m.group(1)
    print(f"created {b['key']:22} -> {m.group(1)}  {b['title'][:60]}")

# 2) deps: bead depends_on=[blockers] -> bd dep add <bead> <blocker>
dep_count = 0
for b in beads:
    for blk in b['depends_on']:
        if blk not in keymap:
            print('  ! unknown dep key', blk, 'for', b['key']); continue
        r = run(['bd', 'dep', 'add', keymap[b['key']], keymap[blk]])
        ok = r.returncode == 0
        dep_count += ok
        print(f"  dep {keymap[b['key']]} <- {keymap[blk]} ({'ok' if ok else 'FAIL: '+r.stderr.strip()[:80]})")

json.dump(keymap, open('/tmp/bead_keymap.json', 'w'))
print(f"\nDONE: {len(keymap)} beads, {dep_count} dependency edges. keymap -> /tmp/bead_keymap.json")
