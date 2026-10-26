//! Starter code and test drivers for each language, generated from the question's signature.

use super::Lang;
use crate::bank::{Question, Ty};

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
    }
}

fn py_ty(t: &Ty) -> String {
    match t {
        Ty::Int | Ty::Long => "int".into(),
        Ty::Double => "float".into(),
        Ty::Bool => "bool".into(),
        Ty::Str | Ty::Char => "str".into(),
        Ty::Arr(e) | Ty::List(e) => format!("List[{}]", py_ty(e)),
    }
}

fn js_ty(t: &Ty) -> String {
    match t {
        Ty::Int | Ty::Long | Ty::Double => "number".into(),
        Ty::Bool => "boolean".into(),
        Ty::Str | Ty::Char => "string".into(),
        Ty::Arr(e) | Ty::List(e) => format!("{}[]", js_ty(e)),
    }
}

/// LeetCode-style starting code for `lang`.
pub fn starter(q: &Question, lang: Lang) -> String {
    let f = &q.function;
    match lang {
        Lang::Java => {
            let ps = q.params.iter().map(|p| format!("{} {}", java_ty(&p.ty), p.name)).collect::<Vec<_>>().join(", ");
            format!("class Solution {{\n    public {} {f}({ps}) {{\n        \n    }}\n}}\n", java_ty(&q.returns))
        }
        Lang::Cpp => {
            let ps = q
                .params
                .iter()
                .map(|p| match p.ty {
                    Ty::Arr(_) | Ty::List(_) => format!("{}& {}", cpp_ty(&p.ty), p.name),
                    _ => format!("{} {}", cpp_ty(&p.ty), p.name),
                })
                .collect::<Vec<_>>()
                .join(", ");
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

pub const PYTHON_DRIVER: &str = r#"import sys, json, time, threading, traceback

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
    sol_path, fname, tests_path = sys.argv[1:4]
    ns = {"__name__": "solution"}
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
    if "Solution" not in ns or not hasattr(ns["Solution"], fname):
        emit({"compile": f"Define class Solution with a method {fname}."})
        return
    tests = json.load(open(tests_path, encoding="utf-8"))
    for i, args in enumerate(tests):
        emit({"begin": i})
        t0 = time.perf_counter()
        try:
            out = getattr(ns["Solution"](), fname)(*args)
            ms = (time.perf_counter() - t0) * 1000
            emit({"i": i, "out": out, "ms": ms})
        except BaseException as e:
            ms = (time.perf_counter() - t0) * 1000
            emit({"i": i, "err": user_trace(e), "ms": ms})

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

/// Builds stress tests: `make.py <function> <count>` prints one `{"args","expected"}` per line.
pub const STRESS_MAKER: &str = r#"import sys, json, random, copy, threading
def main():
    fname, count = sys.argv[1], int(sys.argv[2])
    ns = {}
    exec(open("reference.py", encoding="utf-8").read(), ns)
    gen = {}
    exec(open("stress.py", encoding="utf-8").read(), gen)
    for seed in range(1, count + 1):
        args = gen["gen"](random.Random(seed))
        out = getattr(ns["Solution"](), fname)(*copy.deepcopy(args))
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

// ---------- JavaScript ----------

pub const JS_DRIVER: &str = r#"const fs = require("fs");
const [solPath, fname, testsPath] = process.argv.slice(2);
const emit = o => process.stdout.write("\x1e" + JSON.stringify(o) + "\n");
const now = () => Number(process.hrtime.bigint()) / 1e6;
let fn;
try {
  const src = fs.readFileSync(solPath, "utf8");
  fn = new Function(src + `
;return (typeof ${fname} === "function") ? ${fname}
  : (typeof Solution === "function" ? (s => s.${fname}.bind(s))(new Solution()) : undefined);`)();
} catch (e) {
  emit({ compile: String((e && e.stack) || e).split("\n").slice(0, 6).join("\n") });
  process.exit(0);
}
if (typeof fn !== "function") {
  emit({ compile: `Define a function named ${fname}.` });
  process.exit(0);
}
const tests = JSON.parse(fs.readFileSync(testsPath, "utf8"));
for (let i = 0; i < tests.length; i++) {
  emit({ begin: i });
  const t0 = now();
  try {
    let out = fn(...tests[i]);
    if (out === undefined) out = null;
    emit({ i, out, ms: now() - t0 });
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
    };
    out.push((name.clone(), format!("    static {} {name}(Object o) {{ {body} }}\n", java_ty(t))));
}

pub fn java_main(q: &Question) -> String {
    let mut convs = Vec::new();
    q.params.iter().for_each(|p| java_converters(&p.ty, &mut convs));
    let convs: String = convs.into_iter().map(|(_, c)| c).collect();
    let call_args = q.params.iter().enumerate().map(|(i, p)| format!("{}(a.get({i}))", conv_name(&p.ty))).collect::<Vec<_>>().join(", ");
    JAVA_MAIN.replace("/*CONVERTERS*/", &convs).replace("/*CALL*/", &format!("new Solution().{}({call_args})", q.function))
}

const JAVA_MAIN: &str = r#"import java.util.*;
import java.nio.file.*;
import java.nio.charset.StandardCharsets;

@SuppressWarnings("unchecked")
public class Main {
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
            long t0 = System.nanoTime();
            try {
                Object out = /*CALL*/;
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

pub fn cpp_main(q: &Question) -> String {
    let decls = q.params.iter().enumerate().map(|(i, p)| format!("        {} a{i}; cv(t.a[{i}], a{i});\n", cpp_ty(&p.ty))).collect::<String>();
    let call = format!("sol.{}({})", q.function, (0..q.params.len()).map(|i| format!("a{i}")).collect::<Vec<_>>().join(", "));
    CPP_MAIN.replace("/*DECLS*/", &decls).replace("/*CALL*/", &call)
}

const CPP_MAIN: &str = r#"#include <bits/stdc++.h>
using namespace std;
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

static void emit(const string& s) { cout << "\x1e" << s << "\n" << flush; }

int main(int argc, char** argv) {
    ifstream f(argv[1], ios::binary);
    stringstream ss; ss << f.rdbuf(); SRC = ss.str();
    J tests = parse();
    for (size_t i = 0; i < tests.a.size(); i++) {
        const J& t = tests.a[i];
        emit("{\"begin\":" + to_string(i) + "}");
/*DECLS*/
        Solution sol;
        auto t0 = chrono::steady_clock::now();
        try {
            auto out = /*CALL*/;
            double ms = chrono::duration<double, milli>(chrono::steady_clock::now() - t0).count();
            string o = "{\"i\":" + to_string(i) + ",\"ms\":" + to_string(ms) + ",\"out\":";
            w(o, out);
            emit(o + "}");
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
