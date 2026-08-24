import sqlite3
from collections import defaultdict
con = sqlite3.connect(".borescope/index.db")

TRIVIAL = set("""clone as_str as_ref as_mut to_string to_owned len is_empty iter into_iter
get get_mut insert push pop unwrap unwrap_or unwrap_or_else unwrap_or_default expect ok
map map_err and_then or_else filter collect join split trim contains starts_with ends_with
borrow borrow_mut lock read write elapsed now from into try_into try_from default fmt eq ne
next value key values keys entry or_insert with drain extend deref deref_mut cmp partial_cmp
hash add sub as_bytes to_vec from_utf8 from_utf8_lossy parse set_index get_index byte_length
store load set get_or_init call new_from_unsigned is_null is_undefined to_object""".split())

sym={}; 
for sid,name,path in con.execute("SELECT s.id,s.name,f.path FROM symbols s JOIN files f ON f.id=s.file_id WHERE f.path LIKE 'src/%'"):
    sym[sid]=(name,path.replace("src/",""))
adj=defaultdict(list)
for a,b in con.execute("SELECT from_id,to_id FROM edges WHERE confidence>=0.3"):
    if a in sym and b in sym: adj[a].append(b)

def find(name, filehint):
    for sid,(n,p) in sym.items():
        if n==name and filehint in p: return sid
    return None

# Three subsystem roots (the user's hypothesis: router / loading / serving isolates)
ROOTS = [
    ("router_front",   find("dispatch_to_worker_pool","router.rs")),
    ("app_loading",    find("start_server_with_config","server.rs")),
    ("isolate_serving",find("with_source_backend_and_env","pool.rs")),
]
MAXDEPTH=9
folded=defaultdict(int)
def dfs(sid, stack, instack, depth):
    n=sym[sid][0]
    if n in TRIVIAL: return
    stack.append(n)
    kids=[]; names=set()
    for k in adj.get(sid,[]):
        kn=sym[k][0]
        if kn in TRIVIAL or kn in instack or kn in names: continue
        names.add(kn); kids.append(k)
    if not kids or depth>=MAXDEPTH:
        folded[";".join(stack)]+=1
    else:
        for k in kids: dfs(k, stack, instack|{n}, depth+1)
    stack.pop()

for label,root in ROOTS:
    if root is None: 
        print("MISSING root:",label); continue
    dfs(root,[label],{label},0)

import os
os.makedirs("reports/static-profile", exist_ok=True)
out="reports/static-profile/serving.folded"
with open(out,"w") as f:
    for k,v in sorted(folded.items()): f.write(f"{k} {v}\n")
print("folded stacks:",len(folded),"→",out)
# hottest functions across all three subsystems
frame=defaultdict(int)
for k,v in folded.items():
    for fr in set(k.split(";")[1:]): frame[fr]+=v
print("=== TOP static-hot functions across serving lifecycle ===")
for fr,c in sorted(frame.items(),key=lambda x:-x[1])[:22]:
    print(f"{c:4d}  {fr}")
