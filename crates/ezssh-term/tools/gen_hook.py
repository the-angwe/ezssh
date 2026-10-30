#!/usr/bin/env python3
"""Print the forwarding methods of src/hook.rs from vte's `Handler` trait.

Every `Handler` method defaults to a no-op, so a hand-written forwarding
list could silently drop one. Run after a vte upgrade and paste the output
below the "generated" marker in src/hook.rs:

    python3 tools/gen_hook.py [path/to/vte/src/ansi.rs]

Without an argument it uses the vte 0.15 sources in ~/.cargo/registry.
Methods overridden by hand in hook.rs are listed in OVERRIDDEN.
"""
import glob
import os
import re
import sys

OVERRIDDEN = {"clipboard_store", "dynamic_color_sequence"}

if len(sys.argv) > 1:
    path = sys.argv[1]
else:
    found = sorted(glob.glob(os.path.expanduser("~/.cargo/registry/src/*/vte-0.15.*/src/ansi.rs")))
    if not found:
        sys.exit("vte 0.15 sources not found in ~/.cargo/registry; pass the path to ansi.rs")
    path = found[-1]

src = open(path).read()
body = re.search(r"pub trait Handler \{(.*?)\n\}", src, re.S).group(1)
count = 0
for m in re.finditer(r"fn (\w+)\(&mut self(.*?)\)(\s*->\s*[^{]+)?\s*\{\s*\}", body, re.S):
    name, params, ret = m.group(1), m.group(2), m.group(3)
    if ret:
        sys.exit(f"{name} returns a value; extend the generator")
    count += 1
    if name in OVERRIDDEN:
        continue
    args = [p.strip() for p in params.split(",") if p.strip()]
    sig = [f"a{i}: {a.split(':', 1)[1].strip()}" for i, a in enumerate(args)]
    call = [f"a{i}" for i in range(len(args))]
    print(f"    fn {name}({', '.join(['&mut self'] + sig)}) {{\n        self.term.{name}({', '.join(call)})\n    }}\n")
print(f"// {count} Handler methods in {path}; {len(OVERRIDDEN)} overridden by hand.", file=sys.stderr)
