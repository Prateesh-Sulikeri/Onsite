//! Starter code and test drivers for each language, generated from the question's signature.

use super::Lang;
use crate::bank::{ty_name, Method, Mode, Question, Ty};
use serde_json::json;

// ---------- starter code (what the editor opens with) ----------

fn java_ty(t: &Ty) -> String {
    match t {
        Ty::Int => "int".into(),
        Ty::Long => "long".into(),
        Ty::Double => "double".into(),
        Ty::Bool => "boolean".into(),
        Ty::Str => "String".into(),
        Ty::Char => "char".into(),
        Ty::Arr(e) => format!("{}[]", java_ty(e)),
        Ty::List(e) => format!("List<{}>", java_boxed(e)),
        Ty::ListNode => "ListNode".into(),
        Ty::TreeNode => "TreeNode".into(),
        Ty::Node => "Node".into(),
        Ty::Void => "void".into(),
        Ty::Any => "Object".into(),
    }
}

fn java_boxed(t: &Ty) -> String {
    match t {
        Ty::Int => "Integer".into(),
        Ty::Long => "Long".into(),
        Ty::Double => "Double".into(),
        Ty::Bool => "Boolean".into(),
        Ty::Char => "Character".into(),
        other => java_ty(other),
    }
}

fn cpp_ty(t: &Ty) -> String {
    match t {
        Ty::Int => "int".into(),
        Ty::Long => "long long".into(),
        Ty::Double => "double".into(),
        Ty::Bool => "bool".into(),
        Ty::Str => "string".into(),
        Ty::Char => "char".into(),
        Ty::Arr(e) | Ty::List(e) => format!("vector<{}>", cpp_ty(e)),
        Ty::ListNode => "ListNode*".into(),
        Ty::TreeNode => "TreeNode*".into(),
        Ty::Node => "Node*".into(),
        Ty::Void => "void".into(),
        Ty::Any => "string".into(),
    }
}

fn py_ty(t: &Ty) -> String {
    match t {
        Ty::Int | Ty::Long => "int".into(),
        Ty::Double => "float".into(),
        Ty::Bool => "bool".into(),
        Ty::Str | Ty::Char => "str".into(),
        Ty::Arr(e) | Ty::List(e) => format!("List[{}]", py_ty(e)),
        Ty::ListNode => "Optional[ListNode]".into(),
        Ty::TreeNode => "Optional[TreeNode]".into(),
        Ty::Node => "Optional['Node']".into(),
        Ty::Void => "None".into(),
        Ty::Any => "Any".into(),
    }
}

fn js_ty(t: &Ty) -> String {
    match t {
        Ty::Int | Ty::Long | Ty::Double => "number".into(),
        Ty::Bool => "boolean".into(),
        Ty::Str | Ty::Char => "string".into(),
        Ty::Arr(e) | Ty::List(e) => format!("{}[]", js_ty(e)),
        Ty::ListNode => "ListNode".into(),
        Ty::TreeNode => "TreeNode".into(),
        Ty::Node => "_Node".into(),
        Ty::Void => "void".into(),
        Ty::Any => "*".into(),
    }
}

/// Every type in the question's signature, methods included.
fn all_types(q: &Question) -> Vec<&Ty> {
    let method_tys = q.methods.iter().flat_map(|m| m.params.iter().map(|p| &p.ty).chain([&m.returns]));
    q.params.iter().map(|p| &p.ty).chain([&q.returns]).chain(method_tys).collect()
}

/// Whether the question uses `node` anywhere (alone or inside arrays and lists).
fn uses(q: &Question, node: &Ty) -> bool {
    fn has(t: &Ty, node: &Ty) -> bool {
        match t {
            Ty::Arr(e) | Ty::List(e) => has(e, node),
            t => t == node,
        }
    }
    all_types(q).into_iter().any(|t| has(t, node))
}

const NODES: [Ty; 3] = [Ty::ListNode, Ty::TreeNode, Ty::Node];

/// LeetCode's commented-out definitions of the node classes this question uses.
fn node_docs(q: &Question, lang: Lang) -> String {
    NODES
        .iter()
        .filter(|n| uses(q, n))
        .map(|n| match (n, lang) {
            (Ty::ListNode, Lang::Java) => "/**\n * Definition for singly-linked list.\n * public class ListNode {\n *     int val;\n *     ListNode next;\n *     ListNode() {}\n *     ListNode(int val) { this.val = val; }\n *     ListNode(int val, ListNode next) { this.val = val; this.next = next; }\n * }\n */\n",
            (Ty::TreeNode, Lang::Java) => "/**\n * Definition for a binary tree node.\n * public class TreeNode {\n *     int val;\n *     TreeNode left;\n *     TreeNode right;\n *     TreeNode() {}\n *     TreeNode(int val) { this.val = val; }\n *     TreeNode(int val, TreeNode left, TreeNode right) { this.val = val; this.left = left; this.right = right; }\n * }\n */\n",
            (_, Lang::Java) => "/*\n// Definition for a Node.\nclass Node {\n    public int val;\n    public List<Node> neighbors;\n    public Node() { val = 0; neighbors = new ArrayList<Node>(); }\n    public Node(int _val) { val = _val; neighbors = new ArrayList<Node>(); }\n    public Node(int _val, ArrayList<Node> _neighbors) { val = _val; neighbors = _neighbors; }\n}\n*/\n",
            (Ty::ListNode, Lang::Cpp) => "/**\n * Definition for singly-linked list.\n * struct ListNode {\n *     int val;\n *     ListNode *next;\n *     ListNode() : val(0), next(nullptr) {}\n *     ListNode(int x) : val(x), next(nullptr) {}\n *     ListNode(int x, ListNode *next) : val(x), next(next) {}\n * };\n */\n",
            (Ty::TreeNode, Lang::Cpp) => "/**\n * Definition for a binary tree node.\n * struct TreeNode {\n *     int val;\n *     TreeNode *left;\n *     TreeNode *right;\n *     TreeNode() : val(0), left(nullptr), right(nullptr) {}\n *     TreeNode(int x) : val(x), left(nullptr), right(nullptr) {}\n *     TreeNode(int x, TreeNode *left, TreeNode *right) : val(x), left(left), right(right) {}\n * };\n */\n",
            (_, Lang::Cpp) => "/*\n// Definition for a Node.\nclass Node {\npublic:\n    int val;\n    vector<Node*> neighbors;\n    Node() { val = 0; neighbors = vector<Node*>(); }\n    Node(int _val) { val = _val; neighbors = vector<Node*>(); }\n    Node(int _val, vector<Node*> _neighbors) { val = _val; neighbors = _neighbors; }\n};\n*/\n",
            (Ty::ListNode, Lang::Python) => "# Definition for singly-linked list.\n# class ListNode:\n#     def __init__(self, val=0, next=None):\n#         self.val = val\n#         self.next = next\n",
            (Ty::TreeNode, Lang::Python) => "# Definition for a binary tree node.\n# class TreeNode:\n#     def __init__(self, val=0, left=None, right=None):\n#         self.val = val\n#         self.left = left\n#         self.right = right\n",
            (_, Lang::Python) => "\"\"\"\n# Definition for a Node.\nclass Node:\n    def __init__(self, val = 0, neighbors = None):\n        self.val = val\n        self.neighbors = neighbors if neighbors is not None else []\n\"\"\"\n",
            (Ty::ListNode, Lang::Js) => "/**\n * Definition for singly-linked list.\n * function ListNode(val, next) {\n *     this.val = (val===undefined ? 0 : val)\n *     this.next = (next===undefined ? null : next)\n * }\n */\n",
            (Ty::TreeNode, Lang::Js) => "/**\n * Definition for a binary tree node.\n * function TreeNode(val, left, right) {\n *     this.val = (val===undefined ? 0 : val)\n *     this.left = (left===undefined ? null : left)\n *     this.right = (right===undefined ? null : right)\n * }\n */\n",
            (_, Lang::Js) => "/**\n * // Definition for a _Node.\n * function _Node(val, neighbors) {\n *    this.val = val === undefined ? 0 : val;\n *    this.neighbors = neighbors === undefined ? [] : neighbors;\n * };\n */\n",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cpp_param(t: &Ty, name: &str) -> String {
    match t {
        Ty::Arr(_) | Ty::List(_) => format!("{}& {name}", cpp_ty(t)),
        _ => format!("{} {name}", cpp_ty(t)),
    }
}

/// LeetCode-style starting code for `lang`.
pub fn starter(q: &Question, lang: Lang) -> String {
    let docs = node_docs(q, lang);
    let docs = if docs.is_empty() { docs } else { format!("{docs}\n") };
    let body = match q.mode {
        Mode::Function => function_starter(q, lang),
        Mode::Design => class_starter(&q.function, &q.methods, true, lang),
        Mode::Roundtrip => class_starter(&q.function, &q.methods, false, lang),
    };
    format!("{docs}{body}")
}

/// A class with empty methods. JavaScript roundtrips are two plain functions, as on LeetCode.
fn class_starter(class: &str, methods: &[Method], with_ctor: bool, lang: Lang) -> String {
    let params = |m: &Method, f: &dyn Fn(&Ty, &str) -> String| m.params.iter().map(|p| f(&p.ty, &p.name)).collect::<Vec<_>>().join(", ");
    match lang {
        Lang::Java => {
            let ctor = if with_ctor { format!("\n    public {class}() {{\n        \n    }}\n") } else { String::new() };
            let ms = methods
                .iter()
                .map(|m| format!("\n    public {} {}({}) {{\n        \n    }}\n", java_ty(&m.returns), m.name, params(m, &|t, n| format!("{} {n}", java_ty(t)))))
                .collect::<String>();
            format!("class {class} {{\n{ctor}{ms}}}\n")
        }
        Lang::Cpp => {
            let ctor = if with_ctor { format!("    {class}() {{\n        \n    }}\n\n") } else { String::new() };
            let ms = methods
                .iter()
                .map(|m| format!("    {} {}({}) {{\n        \n    }}\n", cpp_ty(&m.returns), m.name, params(m, &cpp_param)))
                .collect::<Vec<_>>()
                .join("\n");
            format!("class {class} {{\npublic:\n{ctor}{ms}}};\n")
        }
        Lang::Python => {
            let ctor = if with_ctor { "\n    def __init__(self):\n        \n".to_string() } else { String::new() };
            let ms = methods
                .iter()
                .map(|m| {
                    let ps = m.params.iter().map(|p| format!(", {}: {}", p.name, py_ty(&p.ty))).collect::<String>();
                    format!("\n    def {}(self{ps}) -> {}:\n        \n", m.name, py_ty(&m.returns))
                })
                .collect::<String>();
            format!("class {class}:\n{ctor}{ms}")
        }
        Lang::Js if !with_ctor => methods
            .iter()
            .map(|m| {
                let docs = m.params.iter().map(|p| format!(" * @param {{{}}} {}\n", js_ty(&p.ty), p.name)).collect::<String>();
                let ps = m.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
                format!("/**\n{docs} * @return {{{}}}\n */\nvar {} = function({ps}) {{\n    \n}};\n", js_ty(&m.returns), m.name)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Lang::Js => {
            let ms = methods
                .iter()
                .map(|m| {
                    let docs = m.params.iter().map(|p| format!(" * @param {{{}}} {}\n", js_ty(&p.ty), p.name)).collect::<String>();
                    let ps = m.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
                    format!("\n/**\n{docs} * @return {{{}}}\n */\n{class}.prototype.{} = function({ps}) {{\n    \n}};\n", js_ty(&m.returns), m.name)
                })
                .collect::<String>();
            format!("var {class} = function() {{\n    \n}};\n{ms}")
        }
    }
}

fn function_starter(q: &Question, lang: Lang) -> String {
    let f = &q.function;
    match lang {
        Lang::Java => {
            let ps = q.params.iter().map(|p| format!("{} {}", java_ty(&p.ty), p.name)).collect::<Vec<_>>().join(", ");
            format!("class Solution {{\n    public {} {f}({ps}) {{\n        \n    }}\n}}\n", java_ty(&q.returns))
        }
        Lang::Cpp => {
            let ps = q.params.iter().map(|p| cpp_param(&p.ty, &p.name)).collect::<Vec<_>>().join(", ");
            format!("class Solution {{\npublic:\n    {} {f}({ps}) {{\n        \n    }}\n}};\n", cpp_ty(&q.returns))
        }
        Lang::Python => {
            let ps = q.params.iter().map(|p| format!(", {}: {}", p.name, py_ty(&p.ty))).collect::<String>();
            format!("class Solution:\n    def {f}(self{ps}) -> {}:\n        \n", py_ty(&q.returns))
        }
        Lang::Js => {
            let docs = q.params.iter().map(|p| format!(" * @param {{{}}} {}\n", js_ty(&p.ty), p.name)).collect::<String>();
            let ps = q.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
            format!("/**\n{docs} * @return {{{}}}\n */\nvar {f} = function({ps}) {{\n    \n}};\n", js_ty(&q.returns))
        }
    }
}

// ---------- Python ----------

/// LeetCode's node classes and JSON <-> object conversion, shared by the driver and the stress maker.
/// Every call goes through `call(ns, sig, args)`, where `sig` is `sig.json` (see `runner::sig_json`).
const PY_HARNESS: &str = r#"import json
class ListNode:
    def __init__(self, val=0, next=None):
        self.val = val
        self.next = next
class TreeNode:
    def __init__(self, val=0, left=None, right=None):
        self.val = val
        self.left = left
        self.right = right
class Node:
    def __init__(self, val=0, neighbors=None):
        self.val = val
        self.neighbors = neighbors if neighbors is not None else []

LIMIT = 10**6
BANNED = set()   # ids of input graph nodes: Clone Graph must not return them

def elem(t):
    if t.endswith("[]"): return t[:-2]
    if t.startswith("list<"): return t[5:-1]
    return None

def has_nodes(t):
    return "Node" in t

def to_list(v):
    pos = -1
    if v and isinstance(v[0], list):
        v, pos = v
    dummy = cur = ListNode()
    nodes = []
    for x in v:
        cur.next = ListNode(x)
        cur = cur.next
        nodes.append(cur)
    if pos >= 0 and nodes:
        cur.next = nodes[pos]
    return dummy.next

def from_list(h):
    out = []
    while h is not None:
        if len(out) >= LIMIT: raise ValueError("The returned list has a cycle or is too long.")
        out.append(h.val)
        h = h.next
    return out

def to_tree(v):
    if not v or v[0] is None: return None
    root = TreeNode(v[0])
    q, qi, i = [root], 0, 1
    while qi < len(q) and i < len(v):
        n = q[qi]; qi += 1
        if v[i] is not None:
            n.left = TreeNode(v[i]); q.append(n.left)
        i += 1
        if i < len(v) and v[i] is not None:
            n.right = TreeNode(v[i]); q.append(n.right)
        i += 1
    return root

def from_tree(r):
    q, i = [r], 0
    while i < len(q):
        n = q[i]; i += 1
        if n is not None:
            if len(q) > 2 * LIMIT: raise ValueError("The returned tree has a cycle or is too big.")
            q.append(n.left); q.append(n.right)
    out = [None if n is None else n.val for n in q]
    while out and out[-1] is None: out.pop()
    return out

def to_graph(adj):
    nodes = [Node(i + 1) for i in range(len(adj))]
    for i, ns in enumerate(adj):
        nodes[i].neighbors = [nodes[j - 1] for j in ns]
    BANNED.update(id(n) for n in nodes)
    return nodes[0] if nodes else None

def from_graph(start):
    if start is None: return []
    seen, q, i = {id(start)}, [start], 0
    while i < len(q):
        n = q[i]; i += 1
        if id(n) in BANNED: raise ValueError("You returned a node of the original graph, not a copy.")
        for m in n.neighbors:
            if id(m) not in seen:
                seen.add(id(m)); q.append(m)
    adj = [[] for _ in range(max((n.val for n in q), default=0))]
    for n in q:
        if n.val >= 1: adj[n.val - 1] = [m.val for m in n.neighbors]
    return adj

def conv_in(t, v):
    if v is None or not has_nodes(t): return v
    if t == "ListNode": return to_list(v)
    if t == "TreeNode": return to_tree(v)
    if t == "Node": return to_graph(v)
    return [conv_in(elem(t), x) for x in v]

def conv_out(t, v):
    if not has_nodes(t): return v
    if t == "ListNode": return from_list(v)
    if t == "TreeNode": return from_tree(v)
    if t == "Node": return from_graph(v)
    return None if v is None else [conv_out(elem(t), x) for x in v]

def call(ns, sig, args):
    BANNED.clear()
    mode, name = sig["mode"], sig["name"]
    if mode == "function":
        ins = [conv_in(t, a) for t, a in zip(sig["params"], args)]
        out = getattr(ns["Solution"](), name)(*ins)
        if sig["returns"] == "void":
            return conv_out(sig["params"][0], ins[0])
        return conv_out(sig["returns"], out)
    if mode == "design":
        ops, argl = args
        methods = {m["name"]: m for m in sig["methods"]}
        obj, res = ns[name](), [None]
        for op, a in list(zip(ops, argl))[1:]:
            m = methods.get(op)
            if m is None: raise ValueError(f"Unknown operation {op}.")
            out = getattr(obj, op)(*[conv_in(t, x) for t, x in zip(m["params"], a)])
            res.append(None if m["returns"] == "void" else conv_out(m["returns"], out))
        return res
    enc, dec = sig["methods"]
    mid = getattr(ns[name](), enc["name"])(conv_in(enc["params"][0], args[0]))
    if enc["returns"] == "string" and not isinstance(mid, str):
        raise TypeError(f"{enc['name']} must return a string, not {type(mid).__name__}.")
    return conv_out(dec["returns"], getattr(ns[name](), dec["name"])(mid))
"#;

/// `driver.py Solution.py sig.json tests.json`
pub fn python_driver() -> String {
    format!("{PY_HARNESS}{PY_MAIN}")
}

const PY_MAIN: &str = r#"
import sys, time, threading, traceback

PRELUDE = """from typing import *
import collections, heapq, math, bisect, itertools, functools, string, re, random, operator
from collections import *
from heapq import *
from bisect import *
from functools import *
from itertools import *
from math import inf, gcd, sqrt, ceil, floor, log2
"""

def emit(o):
    sys.stdout.write("\x1e" + json.dumps(o, default=list) + "\n")
    sys.stdout.flush()

def user_trace(e):
    frames = [f for f in traceback.extract_tb(e.__traceback__) if f.filename == "Solution.py"]
    lines = [f"  line {f.lineno}, in {f.name}\n    {(f.line or '').strip()}" for f in frames[-6:]]
    return "\n".join(lines + [f"{type(e).__name__}: {e}"])

def main():
    sol_path, sig_path, tests_path = sys.argv[1:4]
    sig = json.load(open(sig_path, encoding="utf-8"))
    ns = {"__name__": "solution", "ListNode": ListNode, "TreeNode": TreeNode, "Node": Node}
    exec(PRELUDE, ns)
    try:
        src = open(sol_path, encoding="utf-8").read()
        exec(compile(src, "Solution.py", "exec"), ns)
    except SyntaxError as e:
        emit({"compile": "".join(traceback.format_exception_only(type(e), e))})
        return
    except Exception as e:
        emit({"compile": user_trace(e)})
        return
    name = sig["name"]
    if sig["mode"] == "function":
        if "Solution" not in ns or not hasattr(ns["Solution"], name):
            emit({"compile": f"Define class Solution with a method {name}."})
            return
    elif name not in ns:
        emit({"compile": f"Define class {name}."})
        return
    tests = json.load(open(tests_path, encoding="utf-8"))
    for i, args in enumerate(tests):
        emit({"begin": i})
        t0 = time.perf_counter()
        try:
            out = call(ns, sig, args)
            emit({"i": i, "out": out, "ms": (time.perf_counter() - t0) * 1000})
        except BaseException as e:
            emit({"i": i, "err": user_trace(e), "ms": (time.perf_counter() - t0) * 1000})

# Deep recursion (DFS on a 300x300 grid) needs a big stack. Windows caps it, so try smaller sizes.
for mb in (512, 256, 128, 64):
    try:
        threading.stack_size(mb * 1024 * 1024)
        break
    except ValueError:
        pass
sys.setrecursionlimit(1_000_000)
t = threading.Thread(target=main)
t.start()
t.join()
"#;

/// Builds stress tests: `make.py <count>` prints one `{"args","expected"}` per line,
/// running `reference.py` on what `stress.py`'s `gen(rng)` returns.
pub fn stress_maker() -> String {
    format!("{PY_HARNESS}{MAKER_MAIN}")
}

const MAKER_MAIN: &str = r#"
import sys, random, copy, threading
from typing import *
def main():
    count = int(sys.argv[1])
    sig = json.load(open("sig.json", encoding="utf-8"))
    ns = {"ListNode": ListNode, "TreeNode": TreeNode, "Node": Node}
    exec("from typing import *\nimport collections, heapq, math, bisect\nfrom collections import *\nfrom heapq import *\nfrom bisect import *\nfrom functools import *", ns)
    exec(open("reference.py", encoding="utf-8").read(), ns)
    gen = {}
    exec(open("stress.py", encoding="utf-8").read(), gen)
    for seed in range(1, count + 1):
        args = gen["gen"](random.Random(seed))
        out = call(ns, sig, copy.deepcopy(args))
        print(json.dumps({"args": args, "expected": out}, separators=(",", ":")))
for mb in (512, 256, 128, 64):
    try:
        threading.stack_size(mb * 1024 * 1024)
        break
    except ValueError:
        pass
sys.setrecursionlimit(1_000_000)
t = threading.Thread(target=main); t.start(); t.join()
"#;

/// What the Python and JavaScript drivers need to know about the signature.
pub fn sig_json(q: &Question) -> String {
    let tys = |ps: &[crate::bank::Param]| ps.iter().map(|p| ty_name(&p.ty)).collect::<Vec<_>>();
    json!({
        "mode": q.mode,
        "name": q.function,
        "params": tys(&q.params),
        "returns": ty_name(&q.returns),
        "methods": q.methods.iter().map(|m| json!({"name": m.name, "params": tys(&m.params), "returns": ty_name(&m.returns)})).collect::<Vec<_>>(),
    })
    .to_string()
}

// ---------- JavaScript ----------

/// `node driver.js solution.js sig.json tests.json`
pub const JS_DRIVER: &str = r#"// Runs in a worker: its stack can be made big enough for deep recursion (a 10^4-node skewed tree),
// which the main thread's ~1 MB stack on Windows is not.
const { Worker, isMainThread } = require("worker_threads");
if (isMainThread) {
  const w = new Worker(__filename, { argv: process.argv.slice(2), resourceLimits: { stackSizeMb: 256 } });
  w.on("error", e => { console.error(String((e && e.stack) || e)); process.exitCode = 1; });
  w.on("exit", code => { if (code) process.exitCode = code; });
  return;
}
const fs = require("fs");
const [solPath, sigPath, testsPath] = process.argv.slice(2);
const sig = JSON.parse(fs.readFileSync(sigPath, "utf8"));
const emit = o => process.stdout.write("\x1e" + JSON.stringify(o) + "\n");
const now = () => Number(process.hrtime.bigint()) / 1e6;

function ListNode(val, next) { this.val = val === undefined ? 0 : val; this.next = next === undefined ? null : next; }
function TreeNode(val, left, right) { this.val = val === undefined ? 0 : val; this.left = left === undefined ? null : left; this.right = right === undefined ? null : right; }
function _Node(val, neighbors) { this.val = val === undefined ? 0 : val; this.neighbors = neighbors === undefined ? [] : neighbors; }
Object.assign(globalThis, { ListNode, TreeNode, _Node, Node: _Node });

const LIMIT = 1e6;
const BANNED = new Set();
const elem = t => t.endsWith("[]") ? t.slice(0, -2) : t.slice(5, -1);
const hasNodes = t => t.includes("Node");
function toList(v) {
  let pos = -1;
  if (v.length && Array.isArray(v[0])) [v, pos] = v;
  const dummy = new ListNode(); let cur = dummy; const nodes = [];
  for (const x of v) { cur.next = new ListNode(x); cur = cur.next; nodes.push(cur); }
  if (pos >= 0 && nodes.length) cur.next = nodes[pos];
  return dummy.next;
}
function fromList(h) {
  const out = [];
  for (; h; h = h.next) { if (out.length >= LIMIT) throw new Error("The returned list has a cycle or is too long."); out.push(h.val); }
  return out;
}
function toTree(v) {
  if (!v.length || v[0] === null) return null;
  const root = new TreeNode(v[0]); const q = [root]; let qi = 0, i = 1;
  while (qi < q.length && i < v.length) {
    const n = q[qi++];
    if (v[i] !== null) { n.left = new TreeNode(v[i]); q.push(n.left); } i++;
    if (i < v.length && v[i] !== null) { n.right = new TreeNode(v[i]); q.push(n.right); } i++;
  }
  return root;
}
function fromTree(r) {
  const q = [r];
  for (let i = 0; i < q.length; i++) if (q[i]) { if (q.length > 2 * LIMIT) throw new Error("The returned tree has a cycle or is too big."); q.push(q[i].left, q[i].right); }
  const out = q.map(n => n ? n.val : null);
  while (out.length && out[out.length - 1] === null) out.pop();
  return out;
}
function toGraph(adj) {
  const nodes = adj.map((_, i) => new _Node(i + 1));
  adj.forEach((ns, i) => { nodes[i].neighbors = ns.map(j => nodes[j - 1]); });
  nodes.forEach(n => BANNED.add(n));
  return nodes.length ? nodes[0] : null;
}
function fromGraph(start) {
  if (!start) return [];
  const seen = new Set([start]), q = [start];
  for (let i = 0; i < q.length; i++) {
    if (BANNED.has(q[i])) throw new Error("You returned a node of the original graph, not a copy.");
    for (const m of q[i].neighbors) if (!seen.has(m)) { seen.add(m); q.push(m); }
  }
  const adj = Array.from({ length: Math.max(0, ...q.map(n => n.val)) }, () => []);
  for (const n of q) if (n.val >= 1) adj[n.val - 1] = n.neighbors.map(m => m.val);
  return adj;
}
function convIn(t, v) {
  if (v === null || v === undefined || !hasNodes(t)) return v;
  if (t === "ListNode") return toList(v);
  if (t === "TreeNode") return toTree(v);
  if (t === "Node") return toGraph(v);
  return v.map(x => convIn(elem(t), x));
}
function convOut(t, v) {
  if (v === undefined) v = null;
  if (!hasNodes(t)) return v;
  if (t === "ListNode") return fromList(v);
  if (t === "TreeNode") return fromTree(v);
  if (t === "Node") return fromGraph(v);
  return v === null ? null : v.map(x => convOut(elem(t), x));
}

let user;
try {
  const src = fs.readFileSync(solPath, "utf8");
  const names = [sig.name, "Solution", ...sig.methods.map(m => m.name)];
  const pick = names.map(n => `${JSON.stringify(n)}: (typeof ${n} === "undefined" ? undefined : ${n})`).join(", ");
  user = new Function(src + `\n;return {${pick}};`)();
} catch (e) {
  emit({ compile: String((e && e.stack) || e).split("\n").slice(0, 6).join("\n") });
  process.exit(0);
}
let fn;
if (sig.mode === "function") {
  fn = typeof user[sig.name] === "function" ? user[sig.name]
    : typeof user.Solution === "function" && typeof user.Solution.prototype[sig.name] === "function" ? (s => s[sig.name].bind(s))(new user.Solution()) : undefined;
  if (!fn) { emit({ compile: `Define a function named ${sig.name}.` }); process.exit(0); }
} else if (sig.mode === "design" && typeof user[sig.name] !== "function") {
  emit({ compile: `Define ${sig.name}.` }); process.exit(0);
} else if (sig.mode === "roundtrip" && !sig.methods.every(m => typeof user[m.name] === "function")) {
  emit({ compile: `Define the functions ${sig.methods.map(m => m.name).join(" and ")}.` }); process.exit(0);
}

function call(args) {
  BANNED.clear();
  if (sig.mode === "function") {
    const ins = sig.params.map((t, i) => convIn(t, args[i]));
    const out = fn(...ins);
    return sig.returns === "void" ? convOut(sig.params[0], ins[0]) : convOut(sig.returns, out);
  }
  if (sig.mode === "design") {
    const [ops, argl] = args; const obj = new user[sig.name](); const res = [null];
    for (let k = 1; k < ops.length; k++) {
      const m = sig.methods.find(m => m.name === ops[k]);
      if (!m || typeof obj[m.name] !== "function") throw new Error(`${sig.name} has no method ${ops[k]}.`);
      const out = obj[m.name](...m.params.map((t, i) => convIn(t, argl[k][i])));
      res.push(m.returns === "void" ? null : convOut(m.returns, out));
    }
    return res;
  }
  const [enc, dec] = sig.methods;
  const mid = user[enc.name](convIn(enc.params[0], args[0]));
  if (enc.returns === "string" && typeof mid !== "string") throw new Error(`${enc.name} must return a string.`);
  return convOut(dec.returns, user[dec.name](mid));
}

const tests = JSON.parse(fs.readFileSync(testsPath, "utf8"));
for (let i = 0; i < tests.length; i++) {
  emit({ begin: i });
  const t0 = now();
  try {
    emit({ i, out: call(tests[i]), ms: now() - t0 });
  } catch (e) {
    emit({ i, err: String((e && e.stack) || e).split("\n").slice(0, 6).join("\n"), ms: now() - t0 });
  }
}
"#;

// ---------- Java ----------

/// Prepended on line 1 of Solution.java (LeetCode imports these for you).
pub const JAVA_IMPORTS: &str = "import java.util.*; import java.util.function.*; import java.util.stream.*; ";

/// Name of the generated converter for a type, e.g. `c_int_arr_arr`.
fn conv_name(t: &Ty) -> String {
    match t {
        Ty::Int => "c_int".into(),
        Ty::Long => "c_long".into(),
        Ty::Double => "c_double".into(),
        Ty::Bool => "c_bool".into(),
        Ty::Str => "c_str".into(),
        Ty::Char => "c_char".into(),
        Ty::Arr(e) => format!("{}_arr", conv_name(e)),
        Ty::List(e) => format!("{}_list", conv_name(e)),
        Ty::ListNode => "c_ListNode".into(),
        Ty::TreeNode => "c_TreeNode".into(),
        Ty::Node => "c_Node".into(),
        Ty::Void | Ty::Any => "c_any".into(),
    }
}

/// Java converters from parsed JSON (`Object`) to `t`, plus everything they call.
fn java_converters(t: &Ty, out: &mut Vec<(String, String)>) {
    let name = conv_name(t);
    if out.iter().any(|(n, _)| *n == name) {
        return;
    }
    let body = match t {
        Ty::Int => "return ((Number) o).intValue();".to_string(),
        Ty::Long => "return ((Number) o).longValue();".to_string(),
        Ty::Double => "return ((Number) o).doubleValue();".to_string(),
        Ty::Bool => "return (Boolean) o;".to_string(),
        Ty::Str => "return (String) o;".to_string(),
        Ty::Char => "return ((String) o).charAt(0);".to_string(),
        Ty::Arr(e) => {
            java_converters(e, out);
            // new int[n] / new int[n][]: the element type with its first [] sized.
            let alloc = match java_ty(e).split_once('[') {
                Some((base, rest)) => format!("{base}[l.size()][{rest}"),
                None => format!("{}[l.size()]", java_ty(e)),
            };
            format!("List<Object> l = (List<Object>) o; {} r = new {alloc}; for (int i = 0; i < r.length; i++) r[i] = {}(l.get(i)); return r;", java_ty(t), conv_name(e))
        }
        Ty::List(e) => {
            java_converters(e, out);
            format!("List<Object> l = (List<Object>) o; {} r = new ArrayList<>(); for (Object x : l) r.add({}(x)); return r;", java_ty(t), conv_name(e))
        }
        Ty::ListNode => "List<Object> l = (List<Object>) o; int pos = -1; \
            if (!l.isEmpty() && l.get(0) instanceof List) { pos = ((Number) l.get(1)).intValue(); l = (List<Object>) l.get(0); } \
            ListNode d = new ListNode(), c = d; List<ListNode> ns = new ArrayList<>(); \
            for (Object x : l) { c.next = new ListNode(((Number) x).intValue()); c = c.next; ns.add(c); } \
            if (pos >= 0 && !ns.isEmpty()) c.next = ns.get(pos); return d.next;"
            .to_string(),
        Ty::TreeNode => "List<Object> l = (List<Object>) o; if (l.isEmpty() || l.get(0) == null) return null; \
            TreeNode root = new TreeNode(((Number) l.get(0)).intValue()); ArrayDeque<TreeNode> q = new ArrayDeque<>(); q.add(root); int i = 1; \
            while (!q.isEmpty() && i < l.size()) { TreeNode n = q.poll(); Object x = l.get(i++); \
            if (x != null) { n.left = new TreeNode(((Number) x).intValue()); q.add(n.left); } \
            if (i < l.size()) { x = l.get(i++); if (x != null) { n.right = new TreeNode(((Number) x).intValue()); q.add(n.right); } } } return root;"
            .to_string(),
        Ty::Node => "List<Object> l = (List<Object>) o; if (l.isEmpty()) return null; Node[] ns = new Node[l.size()]; \
            for (int i = 0; i < ns.length; i++) ns[i] = new Node(i + 1); \
            for (int i = 0; i < ns.length; i++) for (Object j : (List<Object>) l.get(i)) ns[i].neighbors.add(ns[((Number) j).intValue() - 1]); \
            BANNED.addAll(Arrays.asList(ns)); return ns[0];"
            .to_string(),
        Ty::Void | Ty::Any => "return o;".to_string(),
    };
    out.push((name.clone(), format!("    static {} {name}(Object o) {{ {body} }}\n", java_ty(t))));
}

/// Top-level node classes (in Main.java, so Solution.java sees them) and their JSON writers.
/// Only the ones the question uses, so a candidate's own `class Node` elsewhere never clashes.
fn java_nodes(q: &Question) -> (String, String) {
    let mut classes = String::new();
    let mut writers = String::new();
    if uses(q, &Ty::ListNode) {
        classes.push_str("class ListNode { int val; ListNode next; ListNode() {} ListNode(int val) { this.val = val; } ListNode(int val, ListNode next) { this.val = val; this.next = next; } }\n");
        writers.push_str("        if (o instanceof ListNode) { sb.append('['); int n = 0; for (ListNode p = (ListNode) o; p != null; p = p.next) { if (++n > 1000000) throw new RuntimeException(\"The returned list has a cycle or is too long.\"); if (n > 1) sb.append(','); sb.append(p.val); } sb.append(']'); return; }\n");
    }
    if uses(q, &Ty::TreeNode) {
        classes.push_str("class TreeNode { int val; TreeNode left; TreeNode right; TreeNode() {} TreeNode(int val) { this.val = val; } TreeNode(int val, TreeNode left, TreeNode right) { this.val = val; this.left = left; this.right = right; } }\n");
        writers.push_str("        if (o instanceof TreeNode) { List<TreeNode> q = new ArrayList<>(); q.add((TreeNode) o); for (int i = 0; i < q.size(); i++) { TreeNode n = q.get(i); if (n == null) continue; if (q.size() > 2000000) throw new RuntimeException(\"The returned tree has a cycle or is too big.\"); q.add(n.left); q.add(n.right); } int end = q.size(); while (end > 0 && q.get(end - 1) == null) end--; sb.append('['); for (int i = 0; i < end; i++) { if (i > 0) sb.append(','); TreeNode n = q.get(i); if (n == null) sb.append(\"null\"); else sb.append(n.val); } sb.append(']'); return; }\n");
    }
    if uses(q, &Ty::Node) {
        classes.push_str("class Node { public int val; public List<Node> neighbors; public Node() { val = 0; neighbors = new ArrayList<Node>(); } public Node(int _val) { val = _val; neighbors = new ArrayList<Node>(); } public Node(int _val, ArrayList<Node> _neighbors) { val = _val; neighbors = _neighbors; } }\n");
        writers.push_str("        if (o instanceof Node) { Map<Node, Boolean> seen = new IdentityHashMap<>(); List<Node> q = new ArrayList<>(); q.add((Node) o); seen.put((Node) o, true); int max = 0; for (int i = 0; i < q.size(); i++) { Node n = q.get(i); if (BANNED.contains(n)) throw new RuntimeException(\"You returned a node of the original graph, not a copy.\"); max = Math.max(max, n.val); for (Node m : n.neighbors) if (seen.put(m, true) == null) q.add(m); } List<List<Integer>> adj = new ArrayList<>(); for (int i = 0; i < max; i++) adj.add(new ArrayList<>()); for (Node n : q) if (n.val >= 1) for (Node m : n.neighbors) adj.get(n.val - 1).add(m.val); w(sb, adj); return; }\n");
    }
    (classes, writers)
}

/// An absent node result is an empty list or tree on LeetCode, `[]`, not `null`.
fn java_null_fix(t: &Ty) -> &'static str {
    if t.has_nodes() { "                if (out == null) out = Collections.emptyList();\n" } else { "" }
}

pub fn java_main(q: &Question) -> String {
    let mut convs = Vec::new();
    q.params.iter().for_each(|p| java_converters(&p.ty, &mut convs));
    q.methods.iter().flat_map(|m| &m.params).for_each(|p| java_converters(&p.ty, &mut convs));
    let convs: String = convs.into_iter().map(|(_, c)| c).collect();
    let decls = |ps: &[crate::bank::Param]| {
        ps.iter().enumerate().map(|(i, p)| format!("                {} p{i} = {}(a.get({i}));\n", java_ty(&p.ty), conv_name(&p.ty))).collect::<String>()
    };
    let args = |n: usize| (0..n).map(|i| format!("p{i}")).collect::<Vec<_>>().join(", ");
    let run = match q.mode {
        Mode::Function => {
            let call = format!("new Solution().{}({})", q.function, args(q.params.len()));
            let out = if q.returns == Ty::Void { format!("                {call};\n                Object out = p0;\n") } else { format!("                Object out = {call};\n") };
            format!("{}{out}{}", decls(&q.params), java_null_fix(q.output_ty()))
        }
        Mode::Roundtrip => {
            let (enc, dec) = (&q.methods[0], &q.methods[1]);
            format!(
                "{}                {c} s1 = new {c}(), s2 = new {c}();\n                {} mid = s1.{}(p0);\n                Object out = s2.{}(mid);\n{}",
                decls(&q.params),
                java_ty(&enc.returns),
                enc.name,
                dec.name,
                java_null_fix(&dec.returns),
                c = q.function,
            )
        }
        Mode::Design => {
            let cases = q
                .methods
                .iter()
                .map(|m| {
                    let ps = (0..m.params.len()).map(|i| format!("{}(x.get({i}))", conv_name(&m.params[i].ty))).collect::<Vec<_>>().join(", ");
                    let call = format!("obj.{}({ps})", m.name);
                    let body = if m.returns == Ty::Void { format!("{call}; res.add(null);") } else { format!("res.add({call});") };
                    format!("                        case \"{}\": {body} break;\n", m.name)
                })
                .collect::<String>();
            format!(
                "                List<Object> ops = (List<Object>) a.get(0), opArgs = (List<Object>) a.get(1);\n\
                 \x20               List<Object> res = new ArrayList<>(); res.add(null);\n\
                 \x20               {c} obj = new {c}();\n\
                 \x20               for (int k = 1; k < ops.size(); k++) {{\n\
                 \x20                   List<Object> x = (List<Object>) opArgs.get(k);\n\
                 \x20                   switch ((String) ops.get(k)) {{\n{cases}\
                 \x20                       default: throw new RuntimeException(\"Unknown operation \" + ops.get(k));\n\
                 \x20                   }}\n\
                 \x20               }}\n\
                 \x20               Object out = res;\n",
                c = q.function,
            )
        }
    };
    let (classes, writers) = java_nodes(q);
    JAVA_MAIN.replace("/*CLASSES*/", &classes).replace("/*NODE_W*/", &writers).replace("/*CONVERTERS*/", &convs).replace("/*RUN*/", &run)
}

const JAVA_MAIN: &str = r#"import java.util.*;
import java.nio.file.*;
import java.nio.charset.StandardCharsets;

/*CLASSES*/
@SuppressWarnings("unchecked")
public class Main {
    // Nodes of the input graph (Clone Graph): the answer must not reuse them.
    static Set<Object> BANNED = Collections.newSetFromMap(new IdentityHashMap<>());
    // ---- minimal JSON reader: numbers, strings, booleans, null, arrays ----
    static String src; static int pos;
    static Object parse() {
        ws();
        char c = src.charAt(pos);
        if (c == '[') {
            pos++; List<Object> l = new ArrayList<>(); ws();
            if (src.charAt(pos) == ']') { pos++; return l; }
            while (true) { l.add(parse()); ws(); char d = src.charAt(pos++); if (d == ']') return l; }
        }
        if (c == '"') {
            pos++; StringBuilder sb = new StringBuilder();
            while (true) {
                char d = src.charAt(pos++);
                if (d == '"') return sb.toString();
                if (d == '\\') {
                    char e = src.charAt(pos++);
                    switch (e) {
                        case 'n': sb.append('\n'); break; case 't': sb.append('\t'); break;
                        case 'r': sb.append('\r'); break; case 'b': sb.append('\b'); break; case 'f': sb.append('\f'); break;
                        case 'u': sb.append((char) Integer.parseInt(src.substring(pos, pos + 4), 16)); pos += 4; break;
                        default: sb.append(e);
                    }
                } else sb.append(d);
            }
        }
        if (src.startsWith("true", pos)) { pos += 4; return Boolean.TRUE; }
        if (src.startsWith("false", pos)) { pos += 5; return Boolean.FALSE; }
        if (src.startsWith("null", pos)) { pos += 4; return null; }
        int s = pos;
        while (pos < src.length() && "+-0123456789.eE".indexOf(src.charAt(pos)) >= 0) pos++;
        String n = src.substring(s, pos);
        if (n.contains(".") || n.contains("e") || n.contains("E")) return Double.parseDouble(n);
        return Long.parseLong(n);
    }
    static void ws() { while (pos < src.length() && Character.isWhitespace(src.charAt(pos))) pos++; }

    // ---- JSON writer ----
    static void str(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '"' || c == '\\') sb.append('\\').append(c);
            else if (c < 0x20) sb.append(String.format("\\u%04x", (int) c));
            else sb.append(c);
        }
        sb.append('"');
    }
    static void w(StringBuilder sb, Object o) {
        if (o == null) { sb.append("null"); return; }
/*NODE_W*/
        if (o instanceof Double || o instanceof Float) {
            double d = ((Number) o).doubleValue();
            if (Double.isNaN(d) || Double.isInfinite(d)) sb.append("null"); else sb.append(d);
            return;
        }
        if (o instanceof Number || o instanceof Boolean) { sb.append(o); return; }
        if (o instanceof Character) { str(sb, o.toString()); return; }
        if (o instanceof String) { str(sb, (String) o); return; }
        if (o instanceof int[]) { int[] a = (int[]) o; sb.append('['); for (int i = 0; i < a.length; i++) { if (i > 0) sb.append(','); sb.append(a[i]); } sb.append(']'); return; }
        if (o instanceof long[]) { long[] a = (long[]) o; sb.append('['); for (int i = 0; i < a.length; i++) { if (i > 0) sb.append(','); sb.append(a[i]); } sb.append(']'); return; }
        if (o instanceof double[]) { double[] a = (double[]) o; sb.append('['); for (int i = 0; i < a.length; i++) { if (i > 0) sb.append(','); w(sb, a[i]); } sb.append(']'); return; }
        if (o instanceof boolean[]) { boolean[] a = (boolean[]) o; sb.append('['); for (int i = 0; i < a.length; i++) { if (i > 0) sb.append(','); sb.append(a[i]); } sb.append(']'); return; }
        if (o instanceof char[]) { char[] a = (char[]) o; sb.append('['); for (int i = 0; i < a.length; i++) { if (i > 0) sb.append(','); str(sb, String.valueOf(a[i])); } sb.append(']'); return; }
        if (o instanceof Object[]) { Object[] a = (Object[]) o; sb.append('['); for (int i = 0; i < a.length; i++) { if (i > 0) sb.append(','); w(sb, a[i]); } sb.append(']'); return; }
        if (o instanceof Iterable) { sb.append('['); boolean first = true; for (Object x : (Iterable<?>) o) { if (!first) sb.append(','); first = false; w(sb, x); } sb.append(']'); return; }
        str(sb, o.toString());
    }

    static String trace(Throwable e) {
        StringBuilder sb = new StringBuilder(e.toString());
        int shown = 0;
        for (StackTraceElement f : e.getStackTrace()) {
            if (!"Solution.java".equals(f.getFileName())) continue;
            sb.append("\n  at ").append(f.getMethodName()).append(" (line ").append(f.getLineNumber()).append(')');
            if (++shown == 6) break;
        }
        return sb.toString();
    }

/*CONVERTERS*/
    static void emit(String json) { System.out.print("\u001e" + json + "\n"); System.out.flush(); }

    public static void main(String[] argv) throws Exception {
        src = new String(Files.readAllBytes(Paths.get(argv[0])), StandardCharsets.UTF_8); pos = 0;
        List<Object> tests = (List<Object>) parse();
        for (int i = 0; i < tests.size(); i++) {
            List<Object> a = (List<Object>) tests.get(i);
            emit("{\"begin\":" + i + "}");
            BANNED.clear();
            long t0 = System.nanoTime();
            try {
/*RUN*/
                double ms = (System.nanoTime() - t0) / 1e6;
                StringBuilder sb = new StringBuilder("{\"i\":" + i + ",\"ms\":" + ms + ",\"out\":");
                w(sb, out);
                emit(sb.append('}').toString());
            } catch (Throwable e) {
                double ms = (System.nanoTime() - t0) / 1e6;
                StringBuilder sb = new StringBuilder("{\"i\":" + i + ",\"ms\":" + ms + ",\"err\":");
                str(sb, trace(e));
                emit(sb.append('}').toString());
            }
        }
    }
}
"#;

// ---------- C++ ----------

/// Node structs (before the candidate's code) and their JSON readers and writers (after the
/// generic ones, so `w` on the adjacency list resolves).
fn cpp_nodes(q: &Question) -> (String, String) {
    let mut structs = String::new();
    let mut io = String::new();
    if uses(q, &Ty::ListNode) {
        structs.push_str("struct ListNode { int val; ListNode *next; ListNode() : val(0), next(nullptr) {} ListNode(int x) : val(x), next(nullptr) {} ListNode(int x, ListNode *next) : val(x), next(next) {} };\n");
        io.push_str(r#"static void cv(const J& j, ListNode*& h) {
    const J* v = &j; int pos = -1;
    if (!j.a.empty() && j.a[0].k == 5) { v = &j.a[0]; pos = (int)j.a[1].i; }
    ListNode d; ListNode* c = &d; vector<ListNode*> ns;
    for (const J& x : v->a) { c->next = new ListNode((int)x.i); c = c->next; ns.push_back(c); }
    if (pos >= 0 && !ns.empty()) c->next = ns[pos];
    h = d.next;
}
static void w(string& o, ListNode* h) {
    o += '['; int n = 0;
    for (; h; h = h->next) { if (++n > 1000000) throw runtime_error("the returned list has a cycle or is too long"); if (n > 1) o += ','; o += to_string(h->val); }
    o += ']';
}
"#);
    }
    if uses(q, &Ty::TreeNode) {
        structs.push_str("struct TreeNode { int val; TreeNode *left; TreeNode *right; TreeNode() : val(0), left(nullptr), right(nullptr) {} TreeNode(int x) : val(x), left(nullptr), right(nullptr) {} TreeNode(int x, TreeNode *left, TreeNode *right) : val(x), left(left), right(right) {} };\n");
        io.push_str(r#"static void cv(const J& j, TreeNode*& r) {
    r = nullptr;
    if (j.a.empty() || j.a[0].k == 0) return;
    r = new TreeNode((int)j.a[0].i);
    deque<TreeNode*> q{r}; size_t i = 1;
    while (!q.empty() && i < j.a.size()) {
        TreeNode* n = q.front(); q.pop_front();
        if (j.a[i].k != 0) { n->left = new TreeNode((int)j.a[i].i); q.push_back(n->left); }
        i++;
        if (i < j.a.size()) { if (j.a[i].k != 0) { n->right = new TreeNode((int)j.a[i].i); q.push_back(n->right); } i++; }
    }
}
static void w(string& o, TreeNode* r) {
    vector<TreeNode*> q{r};
    for (size_t i = 0; i < q.size(); i++) if (q[i]) { if (q.size() > 2000000) throw runtime_error("the returned tree has a cycle or is too big"); q.push_back(q[i]->left); q.push_back(q[i]->right); }
    size_t end = q.size(); while (end && !q[end - 1]) end--;
    o += '[';
    for (size_t i = 0; i < end; i++) { if (i) o += ','; o += q[i] ? to_string(q[i]->val) : string("null"); }
    o += ']';
}
"#);
    }
    if uses(q, &Ty::Node) {
        structs.push_str("class Node { public: int val; vector<Node*> neighbors; Node() { val = 0; neighbors = vector<Node*>(); } Node(int _val) { val = _val; neighbors = vector<Node*>(); } Node(int _val, vector<Node*> _neighbors) { val = _val; neighbors = _neighbors; } };\n");
        io.push_str(r#"static unordered_set<Node*> BANNED;  // input graph nodes: the copy must not reuse them
static void cv(const J& j, Node*& r) {
    r = nullptr; if (j.a.empty()) return;
    vector<Node*> ns; for (size_t i = 0; i < j.a.size(); i++) ns.push_back(new Node((int)i + 1));
    for (size_t i = 0; i < j.a.size(); i++) for (const J& x : j.a[i].a) ns[i]->neighbors.push_back(ns[x.i - 1]);
    BANNED.insert(ns.begin(), ns.end());
    r = ns[0];
}
static void w(string& o, Node* s) {
    if (!s) { o += "[]"; return; }
    unordered_set<Node*> seen{s}; vector<Node*> q{s}; int mx = 0;
    for (size_t i = 0; i < q.size(); i++) {
        if (BANNED.count(q[i])) throw runtime_error("you returned a node of the original graph, not a copy");
        mx = max(mx, q[i]->val);
        for (Node* m : q[i]->neighbors) if (seen.insert(m).second) q.push_back(m);
    }
    vector<vector<int>> adj(mx);
    for (Node* n : q) if (n->val >= 1) for (Node* m : n->neighbors) adj[n->val - 1].push_back(m->val);
    w(o, adj);
}
"#);
    }
    (structs, io)
}

pub fn cpp_main(q: &Question) -> String {
    let decls = |ps: &[crate::bank::Param]| {
        ps.iter().enumerate().map(|(i, p)| format!("            {} a{i}; cv(t.a[{i}], a{i});\n", cpp_ty(&p.ty))).collect::<String>()
    };
    let args = |n: usize| (0..n).map(|i| format!("a{i}")).collect::<Vec<_>>().join(", ");
    let run = match q.mode {
        Mode::Function => {
            let call = format!("sol.{}({})", q.function, args(q.params.len()));
            let out = if q.returns == Ty::Void { format!("            {call};\n            w(res, a0);\n") } else { format!("            auto out = {call};\n            w(res, out);\n") };
            format!("{}            Solution sol;\n{out}", decls(&q.params))
        }
        Mode::Roundtrip => format!(
            "{}            {c} s1, s2;\n            auto mid = s1.{}(a0);\n            auto out = s2.{}(mid);\n            w(res, out);\n",
            decls(&q.params),
            q.methods[0].name,
            q.methods[1].name,
            c = q.function,
        ),
        Mode::Design => {
            let cases = q
                .methods
                .iter()
                .map(|m| {
                    let ps = m.params.iter().enumerate().map(|(i, p)| format!("{} p{i}; cv(x.a[{i}], p{i}); ", cpp_ty(&p.ty))).collect::<String>();
                    let call = format!("obj->{}({})", m.name, (0..m.params.len()).map(|i| format!("p{i}")).collect::<Vec<_>>().join(", "));
                    let body = if m.returns == Ty::Void { format!("{call}; res += \"null\";") } else { format!("w(res, {call});") };
                    format!("                else if (op == \"{}\") {{ {ps}{body} }}\n", m.name)
                })
                .collect::<String>();
            format!(
                "            const J& ops = t.a[0]; const J& opArgs = t.a[1];\n\
                 \x20           {c}* obj = new {c}(); res = \"[null\";\n\
                 \x20           for (size_t k = 1; k < ops.a.size(); k++) {{\n\
                 \x20               const string& op = ops.a[k].s; const J& x = opArgs.a[k]; res += ',';\n\
                 \x20               if (false) {{}}\n{cases}\
                 \x20               else throw runtime_error(\"unknown operation \" + op);\n\
                 \x20           }}\n\
                 \x20           res += ']';\n",
                c = q.function,
            )
        }
    };
    let (structs, io) = cpp_nodes(q);
    CPP_MAIN.replace("/*STRUCTS*/", &structs).replace("/*NODE_IO*/", &io).replace("/*RUN*/", &run)
}

const CPP_MAIN: &str = r#"#include <bits/stdc++.h>
using namespace std;
/*STRUCTS*/
#include "solution.cpp"

// ---- minimal JSON reader ----
struct J { int k = 0; long long i = 0; double d = 0; bool b = false; string s; vector<J> a; };
static string SRC; static size_t P = 0;
static void ws() { while (P < SRC.size() && isspace((unsigned char)SRC[P])) P++; }
static J parse() {
    ws(); J j; char c = SRC[P];
    if (c == '[') {
        P++; j.k = 5; ws();
        if (SRC[P] == ']') { P++; return j; }
        while (true) { j.a.push_back(parse()); ws(); char d = SRC[P++]; if (d == ']') return j; }
    }
    if (c == '"') {
        P++; j.k = 4;
        while (true) {
            char d = SRC[P++];
            if (d == '"') return j;
            if (d == '\\') {
                char e = SRC[P++];
                if (e == 'n') j.s += '\n'; else if (e == 't') j.s += '\t'; else if (e == 'r') j.s += '\r';
                else if (e == 'u') { j.s += (char)stoi(SRC.substr(P, 4), nullptr, 16); P += 4; }
                else j.s += e;
            } else j.s += d;
        }
    }
    if (SRC.compare(P, 4, "true") == 0) { P += 4; j.k = 3; j.b = true; return j; }
    if (SRC.compare(P, 5, "false") == 0) { P += 5; j.k = 3; return j; }
    if (SRC.compare(P, 4, "null") == 0) { P += 4; return j; }
    size_t s = P;
    while (P < SRC.size() && strchr("+-0123456789.eE", SRC[P])) P++;
    string n = SRC.substr(s, P - s);
    if (n.find_first_of(".eE") != string::npos) { j.k = 2; j.d = stod(n); }
    else { j.k = 1; j.i = stoll(n); j.d = (double)j.i; }
    return j;
}

// ---- JSON -> C++ ----
static void cv(const J& j, int& x) { x = (int)j.i; }
static void cv(const J& j, long long& x) { x = j.i; }
static void cv(const J& j, double& x) { x = j.d; }
static void cv(const J& j, bool& x) { x = j.b; }
static void cv(const J& j, string& x) { x = j.s; }
static void cv(const J& j, char& x) { x = j.s.empty() ? '\0' : j.s[0]; }
template <class T> static void cv(const J& j, vector<T>& v) {
    v.resize(j.a.size());
    for (size_t i = 0; i < j.a.size(); i++) { T e; cv(j.a[i], e); v[i] = e; }
}

// ---- C++ -> JSON ----
static void wstr(string& o, const string& s) {
    o += '"';
    for (char c : s) {
        if (c == '"' || c == '\\') { o += '\\'; o += c; }
        else if ((unsigned char)c < 0x20) { char b[8]; snprintf(b, sizeof b, "\\u%04x", c); o += b; }
        else o += c;
    }
    o += '"';
}
static void w(string& o, int x) { o += to_string(x); }
static void w(string& o, long x) { o += to_string(x); }
static void w(string& o, long long x) { o += to_string(x); }
static void w(string& o, unsigned x) { o += to_string(x); }
static void w(string& o, unsigned long long x) { o += to_string(x); }
static void w(string& o, double x) { if (!isfinite(x)) { o += "null"; return; } char b[40]; snprintf(b, sizeof b, "%.17g", x); o += b; }
static void w(string& o, bool x) { o += x ? "true" : "false"; }
static void w(string& o, char x) { wstr(o, string(1, x)); }
static void w(string& o, const string& x) { wstr(o, x); }
static void w(string& o, const char* x) { wstr(o, x); }
template <class T> static void w(string& o, const vector<T>& v) {
    o += '[';
    for (size_t i = 0; i < v.size(); i++) { if (i) o += ','; w(o, v[i]); }
    o += ']';
}
/*NODE_IO*/
static void emit(const string& s) { cout << "\x1e" << s << "\n" << flush; }

int main(int argc, char** argv) {
    ifstream f(argv[1], ios::binary);
    stringstream ss; ss << f.rdbuf(); SRC = ss.str();
    J tests = parse();
    for (size_t i = 0; i < tests.a.size(); i++) {
        const J& t = tests.a[i];
        emit("{\"begin\":" + to_string(i) + "}");
        auto t0 = chrono::steady_clock::now();
        try {
            string res;
/*RUN*/
            double ms = chrono::duration<double, milli>(chrono::steady_clock::now() - t0).count();
            emit("{\"i\":" + to_string(i) + ",\"ms\":" + to_string(ms) + ",\"out\":" + res + "}");
        } catch (const exception& e) {
            double ms = chrono::duration<double, milli>(chrono::steady_clock::now() - t0).count();
            string o = "{\"i\":" + to_string(i) + ",\"ms\":" + to_string(ms) + ",\"err\":";
            wstr(o, string("exception: ") + e.what());
            emit(o + "}");
        } catch (...) {
            string o = "{\"i\":" + to_string(i) + ",\"err\":\"unknown exception\"}";
            emit(o);
        }
    }
}
"#;
