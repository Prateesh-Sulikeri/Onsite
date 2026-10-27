//! Local DSA question bank: `prompts/questions.csv`, one LeetCode-style problem per row.
//! Tests live in the CSV as JSON; the model only ever picks a question id, never writes tests.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

pub const FILE: &str = "questions.csv";

/// Parameter and return types the runner can build in every language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int,
    Long,
    Double,
    Bool,
    Str,
    Char,
    /// `int[]`, `int[][]`: arrays in Java, vectors in C++.
    Arr(Box<Ty>),
    /// `list<int>`, `list<list<string>>`: `List<Integer>` in Java, vectors in C++.
    List(Box<Ty>),
    /// LeetCode's singly linked list. JSON: `[1,2,3]`, or `[[3,2,0,-4],1]` for a list whose tail
    /// links back to index 1 (Linked List Cycle).
    ListNode,
    /// LeetCode's binary tree. JSON: level order with nulls, `[1,null,2,3]`.
    TreeNode,
    /// LeetCode's graph node (Clone Graph). JSON: adjacency list, node `i+1` at index `i`.
    Node,
    /// Return type of an in-place method; the result is the first argument after the call.
    Void,
    /// Any JSON. Only for the arguments column of a design question.
    Any,
}

impl Ty {
    pub fn parse(s: &str) -> Result<Ty, String> {
        let s = s.trim();
        let ty = if let Some(inner) = s.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
            Ty::List(Box::new(Ty::parse(inner)?))
        } else if let Some(inner) = s.strip_suffix("[]") {
            Ty::Arr(Box::new(Ty::parse(inner)?))
        } else {
            match s {
                "int" => Ty::Int,
                "long" => Ty::Long,
                "double" => Ty::Double,
                "bool" | "boolean" => Ty::Bool,
                "string" => Ty::Str,
                "char" => Ty::Char,
                "ListNode" => Ty::ListNode,
                "TreeNode" => Ty::TreeNode,
                "Node" => Ty::Node,
                "void" => Ty::Void,
                "any" => Ty::Any,
                _ => return Err(format!("unknown type '{s}'")),
            }
        };
        if ty.depth() > 2 {
            return Err(format!("'{s}' nests too deep (two levels at most)"));
        }
        Ok(ty)
    }

    pub fn depth(&self) -> usize {
        match self {
            Ty::Arr(t) | Ty::List(t) => 1 + t.depth(),
            _ => 0,
        }
    }

    /// The element type of an array or list.
    pub fn elem(&self) -> Option<&Ty> {
        match self {
            Ty::Arr(t) | Ty::List(t) => Some(t),
            _ => None,
        }
    }

    /// Whether `v` is a valid value of this type.
    pub fn accepts(&self, v: &Value) -> bool {
        match self {
            Ty::Int => v.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
            Ty::Long => v.as_i64().is_some(),
            Ty::Double => v.is_number(),
            Ty::Bool => v.is_boolean(),
            Ty::Str => v.is_string(),
            Ty::Char => v.as_str().is_some_and(|s| s.chars().count() == 1),
            Ty::Arr(t) | Ty::List(t) => v.as_array().is_some_and(|a| a.iter().all(|x| t.accepts(x))),
            Ty::ListNode => match v.as_array().map(Vec::as_slice) {
                Some([vals, pos]) if vals.is_array() => {
                    let n = vals.as_array().map_or(0, Vec::len) as i64;
                    Ty::Arr(Box::new(Ty::Int)).accepts(vals) && pos.as_i64().is_some_and(|p| p >= -1 && p < n)
                }
                _ => Ty::Arr(Box::new(Ty::Int)).accepts(v),
            },
            Ty::TreeNode => v.as_array().is_some_and(|a| {
                a.first().is_none_or(|r| !r.is_null()) && a.iter().all(|x| x.is_null() || Ty::Int.accepts(x))
            }),
            Ty::Node => v.as_array().is_some_and(|a| {
                let n = a.len() as i64;
                a.iter().all(|ns| ns.as_array().is_some_and(|ns| ns.iter().all(|j| j.as_i64().is_some_and(|j| j >= 1 && j <= n))))
            }),
            Ty::Void => v.is_null(),
            Ty::Any => true,
        }
    }

    /// Types the runner builds from JSON into objects (and back), rather than passing through.
    pub fn has_nodes(&self) -> bool {
        match self {
            Ty::ListNode | Ty::TreeNode | Ty::Node => true,
            Ty::Arr(t) | Ty::List(t) => t.has_nodes(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    Exact,
    /// A list whose order doesn't matter.
    AnyOrder,
    /// A list of lists where neither level's order matters (3Sum, Group Anagrams).
    NestedAnyOrder,
    /// Numbers within 1e-5.
    Float,
}

impl Compare {
    fn parse(s: &str) -> Result<Compare, String> {
        Ok(match s.trim() {
            "" | "exact" => Compare::Exact,
            "any_order" => Compare::AnyOrder,
            "nested_any_order" => Compare::NestedAnyOrder,
            "float" => Compare::Float,
            o => return Err(format!("unknown compare '{o}'")),
        })
    }

    pub fn equal(self, expected: &Value, got: &Value) -> bool {
        match self {
            Compare::Exact => num_eq(expected, got, 0.0),
            Compare::Float => num_eq(expected, got, 1e-5),
            Compare::AnyOrder => sorted(expected) == sorted(got),
            Compare::NestedAnyOrder => {
                let norm = |v: &Value| match v.as_array() {
                    Some(a) => sorted(&Value::Array(a.iter().map(sorted).collect())),
                    None => v.clone(),
                };
                norm(expected) == norm(got)
            }
        }
    }
}

/// Equality where numbers compare by value (1 == 1.0) within `tol`.
fn num_eq(a: &Value, b: &Value, tol: f64) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= tol.max(tol * x.abs()) || x == y,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| num_eq(a, b, tol)),
        _ => a == b,
    }
}

/// An array with its items sorted by their JSON text; anything else unchanged.
fn sorted(v: &Value) -> Value {
    match v.as_array() {
        Some(a) => {
            let mut items = a.clone();
            items.sort_by_cached_key(|x| x.to_string());
            Value::Array(items)
        }
        None => v.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Param {
    pub name: String,
    #[serde(serialize_with = "ser_ty")]
    pub ty: Ty,
}

fn ser_ty<S: serde::Serializer>(t: &Ty, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&ty_name(t))
}

pub fn ty_name(t: &Ty) -> String {
    match t {
        Ty::Int => "int".into(),
        Ty::Long => "long".into(),
        Ty::Double => "double".into(),
        Ty::Bool => "bool".into(),
        Ty::Str => "string".into(),
        Ty::Char => "char".into(),
        Ty::Arr(t) => format!("{}[]", ty_name(t)),
        Ty::List(t) => format!("list<{}>", ty_name(t)),
        Ty::ListNode => "ListNode".into(),
        Ty::TreeNode => "TreeNode".into(),
        Ty::Node => "Node".into(),
        Ty::Void => "void".into(),
        Ty::Any => "any".into(),
    }
}

/// How a question is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// `Solution.function(params)`.
    Function,
    /// A class driven by LeetCode's operation list: args are `[["Trie","insert",...], [[],["apple"],...]]`
    /// and the result is one value per operation (null for the constructor and void methods).
    Design,
    /// Serialize then deserialize with two fresh instances (Codec): the result must equal the input.
    Roundtrip,
}

/// One method of a design or roundtrip class: `insert(word: string) -> void`.
#[derive(Debug, Clone, Serialize)]
pub struct Method {
    pub name: String,
    pub params: Vec<Param>,
    #[serde(serialize_with = "ser_ty")]
    pub returns: Ty,
}

fn parse_method(line: &str) -> Result<Method, String> {
    let bad = || format!("method '{line}' must look like 'name(a: int, b: string) -> bool'");
    let (head, ret) = line.split_once("->").ok_or_else(bad)?;
    let (name, rest) = head.split_once('(').ok_or_else(bad)?;
    let inner = rest.trim().strip_suffix(')').ok_or_else(bad)?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("bad method name '{name}'"));
    }
    let params = parse_params(&inner.replace(',', ";"))?;
    Ok(Method { name: name.to_string(), params, returns: Ty::parse(ret)? })
}

#[derive(Debug, Clone, Serialize)]
pub struct Test {
    pub args: Vec<Value>,
    pub expected: Value,
    /// Shown on the Problem tab and run by Run; the rest only by Submit.
    pub example: bool,
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub difficulty: String,
    pub levels: Vec<String>,
    pub topics: Vec<String>,
    pub companies: Vec<String>,
    /// The method name, or the class name for design and roundtrip questions.
    pub function: String,
    pub mode: Mode,
    /// Design and roundtrip only. Roundtrip has exactly two: encode, then decode.
    pub methods: Vec<Method>,
    /// Design questions get `operations: list<string>; arguments: any`; roundtrip the encoder's parameter.
    pub params: Vec<Param>,
    #[serde(serialize_with = "ser_ty")]
    pub returns: Ty,
    pub compare: Compare,
    pub statement: String,
    pub constraints: Vec<String>,
    pub tests: Vec<Test>,
    pub hints: Vec<String>,
    pub time: String,
    pub space: String,
    pub follow_ups: Vec<String>,
    /// Python `class Solution`, used to check the tests and to answer generated stress tests.
    pub reference: String,
    /// Python `def gen(rng)` returning one argument list at full constraint size.
    pub stress: String,
}

impl Question {
    /// `nums = [2,7,11,15], target = 9`. A cyclic list shows as `head = [3,2,0,-4], pos = 1`.
    pub fn input_text(&self, args: &[Value]) -> String {
        self.params
            .iter()
            .zip(args)
            .map(|(p, a)| match (&p.ty, a.as_array().map(Vec::as_slice)) {
                (Ty::ListNode, Some([vals, pos])) if vals.is_array() => format!("{} = {vals}, pos = {pos}", p.name),
                _ => format!("{} = {}", p.name, a),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The type of the value a test expects: an in-place method's first argument.
    pub fn output_ty(&self) -> &Ty {
        match (&self.returns, self.params.first()) {
            (Ty::Void, Some(p)) => &p.ty,
            (t, _) => t,
        }
    }

    /// Checks one test's arguments and expected value against the signature.
    pub fn check(&self, args: &[Value], expected: &Value) -> Result<(), String> {
        if self.mode == Mode::Design {
            return self.check_design(args, expected);
        }
        if args.len() != self.params.len() {
            return Err(format!("has {} args, the function takes {}", args.len(), self.params.len()));
        }
        if let Some((p, _)) = self.params.iter().zip(args).find(|(p, a)| !p.ty.accepts(a)) {
            return Err(format!("'{}' is not a valid {}", p.name, ty_name(&p.ty)));
        }
        if !self.output_ty().accepts(expected) {
            return Err(format!("expected is not a valid {}", ty_name(self.output_ty())));
        }
        Ok(())
    }

    fn check_design(&self, args: &[Value], expected: &Value) -> Result<(), String> {
        let (Some(ops), Some(argl), Some(exp)) = (args.first().and_then(Value::as_array), args.get(1).and_then(Value::as_array), expected.as_array()) else {
            return Err("needs args [[operations], [arguments]] and an expected list".into());
        };
        if ops.len() != argl.len() || ops.len() != exp.len() {
            return Err("operations, arguments and expected differ in length".into());
        }
        if ops.first().and_then(Value::as_str) != Some(self.function.as_str()) || argl[0].as_array().is_none_or(|a| !a.is_empty()) {
            return Err(format!("the first operation must be {}, with no arguments", self.function));
        }
        for (k, ((op, a), e)) in ops.iter().zip(argl).zip(exp).enumerate().skip(1) {
            let op = op.as_str().unwrap_or_default();
            let m = self.methods.iter().find(|m| m.name == op).ok_or_else(|| format!("operation {k}: no method '{op}'"))?;
            let a = a.as_array().filter(|a| a.len() == m.params.len()).ok_or_else(|| format!("operation {k} ({op}): wrong number of arguments"))?;
            if let Some((p, _)) = m.params.iter().zip(a).find(|(p, x)| !p.ty.accepts(x)) {
                return Err(format!("operation {k} ({op}): '{}' is not a valid {}", p.name, ty_name(&p.ty)));
            }
            if !(m.returns == Ty::Void && e.is_null() || m.returns != Ty::Void && m.returns.accepts(e)) {
                return Err(format!("operation {k} ({op}): expected is not a valid {}", ty_name(&m.returns)));
            }
        }
        if !exp[0].is_null() {
            return Err("the constructor's expected value must be null".into());
        }
        Ok(())
    }

    pub fn examples(&self) -> impl Iterator<Item = &Test> {
        self.tests.iter().filter(|t| t.example)
    }
}

fn split(s: &str, sep: char) -> Vec<String> {
    s.split(sep).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

fn parse_params(s: &str) -> Result<Vec<Param>, String> {
    split(s, ';')
        .iter()
        .map(|p| {
            let (name, ty) = p.split_once(':').ok_or_else(|| format!("param '{p}' needs 'name: type'"))?;
            let name = name.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("bad param name '{name}'"));
            }
            Ok(Param { name: name.to_string(), ty: Ty::parse(ty)? })
        })
        .collect()
}

fn parse_tests(s: &str, q: &Question) -> Result<Vec<Test>, String> {
    let raw: Vec<Value> = serde_json::from_str(s).map_err(|e| format!("tests are not valid JSON: {e}"))?;
    let mut out = Vec::new();
    for (i, t) in raw.iter().enumerate() {
        let args = t["args"].as_array().ok_or_else(|| format!("test {} has no args array", i + 1))?;
        q.check(args, &t["expected"]).map_err(|e| format!("test {}: {e}", i + 1))?;
        out.push(Test {
            args: args.clone(),
            expected: t["expected"].clone(),
            example: t["example"].as_bool().unwrap_or(false),
            explanation: t["explanation"].as_str().map(String::from),
        });
    }
    if out.is_empty() {
        return Err("no tests".into());
    }
    if !out.iter().any(|t| t.example) {
        out[0].example = true;
    }
    Ok(out)
}

fn parse_row(r: &csv::StringRecord, h: &csv::StringRecord) -> Result<Question, String> {
    let col = |name: &str| h.iter().position(|x| x.trim() == name).and_then(|i| r.get(i)).unwrap_or("").trim().to_string();
    let id = col("id");
    if id.is_empty() {
        return Err("missing id".into());
    }
    let difficulty = match col("difficulty").to_lowercase().as_str() {
        "easy" => "Easy",
        "medium" => "Medium",
        "hard" => "Hard",
        d => return Err(format!("difficulty '{d}' must be Easy, Medium or Hard")),
    }
    .to_string();
    let function = col("function");
    if function.is_empty() || !function.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("bad function name '{function}'"));
    }
    let mode = match col("kind").as_str() {
        "" | "function" => Mode::Function,
        "design" => Mode::Design,
        "roundtrip" => Mode::Roundtrip,
        k => return Err(format!("kind '{k}' must be function, design or roundtrip")),
    };
    let methods = split(&col("methods"), '\n').iter().map(|m| parse_method(m)).collect::<Result<Vec<_>, _>>()?;
    let (params, returns) = match mode {
        Mode::Function => {
            let params = parse_params(&col("params"))?;
            let returns = Ty::parse(&col("returns"))?;
            if returns == Ty::Void && params.is_empty() {
                return Err("an in-place (void) function needs a parameter to check".into());
            }
            (params, returns)
        }
        Mode::Design => {
            if methods.is_empty() {
                return Err("a design question needs methods".into());
            }
            let any = |n: &str, t: Ty| Param { name: n.into(), ty: t };
            (vec![any("operations", Ty::List(Box::new(Ty::Str))), any("arguments", Ty::Any)], Ty::Any)
        }
        Mode::Roundtrip => match methods.as_slice() {
            [enc, dec] if enc.params.len() == 1 && dec.params.len() == 1 => (enc.params.clone(), dec.returns.clone()),
            _ => return Err("a roundtrip question needs two one-argument methods: encode, then decode".into()),
        },
    };
    let mut q = Question {
        title: col("title"),
        difficulty,
        levels: split(&col("levels"), ';'),
        topics: split(&col("topics"), ';'),
        companies: split(&col("companies"), ';'),
        function,
        mode,
        methods,
        params,
        returns,
        compare: Compare::parse(&col("compare"))?,
        statement: col("statement"),
        constraints: split(&col("constraints"), '\n'),
        tests: vec![],
        hints: split(&col("hints"), '\n'),
        time: col("time"),
        space: col("space"),
        follow_ups: split(&col("follow_ups"), '\n'),
        reference: col("reference"),
        stress: col("stress"),
        id,
    };
    q.tests = parse_tests(&col("tests"), &q)?;
    Ok(q)
}

#[derive(Debug, Default)]
pub struct Bank {
    pub questions: Vec<Question>,
    /// One line per rejected row, shown in Settings.
    pub errors: Vec<String>,
}

impl Bank {
    pub fn load(prompts_dir: &Path) -> Bank {
        match std::fs::read(prompts_dir.join(FILE)) {
            Ok(bytes) => Bank::parse(&bytes),
            Err(e) => Bank { questions: vec![], errors: vec![format!("{FILE}: {e}")] },
        }
    }

    pub fn parse(bytes: &[u8]) -> Bank {
        let mut bank = Bank::default();
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
        let header = match rdr.headers() {
            Ok(h) => h.clone(),
            Err(e) => return Bank { questions: vec![], errors: vec![format!("header: {e}")] },
        };
        let mut seen = HashSet::new();
        for (i, rec) in rdr.records().enumerate() {
            let line = i + 2;
            match rec.map_err(|e| e.to_string()).and_then(|r| parse_row(&r, &header)) {
                Ok(q) if !seen.insert(q.id.clone()) => bank.errors.push(format!("row {line}: duplicate id '{}'", q.id)),
                Ok(q) => bank.questions.push(q),
                Err(e) => bank.errors.push(format!("row {line}: {e}")),
            }
        }
        bank
    }

    pub fn get(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|q| q.id == id)
    }

    /// The best unused question of `difficulty`: most topic and company overlap first, then a
    /// stable shuffle by `seed` so repeated loops don't always open with the same problem.
    pub fn pick(&self, difficulty: &str, topics: &[String], company: &str, used: &HashSet<String>, seed: u64) -> Option<&Question> {
        let fresh: Vec<&Question> = self.questions.iter().filter(|q| !used.contains(&q.id)).collect();
        // Every question used already: allow repeats rather than nothing.
        let pool = if fresh.is_empty() { self.questions.iter().collect() } else { fresh };
        let want: HashSet<String> = topics.iter().map(|t| t.to_lowercase()).collect();
        pool.into_iter()
            .filter(|q| q.difficulty.eq_ignore_ascii_case(difficulty))
            .max_by_key(|q| {
                let overlap = q.topics.iter().filter(|t| want.contains(&t.to_lowercase())).count();
                let company_hit = q.companies.iter().any(|c| c.eq_ignore_ascii_case(company));
                (overlap * 2 + company_hit as usize, mix(seed, &q.id))
            })
    }
}

fn mix(seed: u64, id: &str) -> u64 {
    id.bytes().fold(seed ^ 0x9E37_79B9_7F4A_7C15, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01B3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repo() -> Bank {
        Bank::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts"))
    }

    #[test]
    fn seed_bank_loads_clean() {
        let b = repo();
        assert!(b.errors.is_empty(), "{:#?}", b.errors);
        assert_eq!(b.questions.len(), 81);
        for d in ["Easy", "Medium", "Hard"] {
            assert!(b.questions.iter().filter(|q| q.difficulty == d).count() >= 5, "{d}");
        }
        let q = b.get("two-sum").unwrap();
        assert_eq!(q.input_text(&q.tests[0].args), "nums = [2,7,11,15], target = 9");
        assert_eq!(q.examples().count(), 3);
        assert!(q.tests.iter().any(|t| !t.example));
    }

    #[test]
    fn types_parse_and_check() {
        assert_eq!(Ty::parse("int[][]").unwrap(), Ty::Arr(Box::new(Ty::Arr(Box::new(Ty::Int)))));
        assert_eq!(Ty::parse("list<list<string>>").unwrap().depth(), 2);
        assert!(Ty::parse("int[][][]").is_err());
        assert!(Ty::parse("LinkedList").is_err());
        assert!(Ty::Int.accepts(&json!(5)) && !Ty::Int.accepts(&json!(3_000_000_000i64)));
        assert!(Ty::Long.accepts(&json!(3_000_000_000i64)));
        assert!(Ty::Char.accepts(&json!("1")) && !Ty::Char.accepts(&json!("10")));
        assert!(Ty::parse("char[][]").unwrap().accepts(&json!([["1", "0"]])));
        assert!(!Ty::parse("int[]").unwrap().accepts(&json!([1, "2"])));
    }

    #[test]
    fn node_types_accept_leetcode_json() {
        assert!(Ty::ListNode.accepts(&json!([1, 2, 3])) && Ty::ListNode.accepts(&json!([])));
        assert!(Ty::ListNode.accepts(&json!([[3, 2, 0, -4], 1])) && Ty::ListNode.accepts(&json!([[], -1])));
        assert!(!Ty::ListNode.accepts(&json!([[3, 2], 2])), "pos past the end");
        assert!(Ty::TreeNode.accepts(&json!([1, null, 2, 3])) && Ty::TreeNode.accepts(&json!([])));
        assert!(!Ty::TreeNode.accepts(&json!([null, 1])), "a tree can't start with a gap");
        assert!(Ty::Node.accepts(&json!([[2, 4], [1, 3], [2, 4], [1, 3]])) && Ty::Node.accepts(&json!([[]])));
        assert!(!Ty::Node.accepts(&json!([[2]])), "neighbor 2 doesn't exist");
        assert!(Ty::parse("ListNode[]").unwrap().has_nodes() && !Ty::parse("int[][]").unwrap().has_nodes());
    }

    #[test]
    fn every_mode_loads_from_the_bank() {
        let b = repo();
        let trie = b.get("implement-trie-prefix-tree").unwrap();
        assert_eq!((trie.mode, trie.methods.len()), (Mode::Design, 3));
        assert_eq!(trie.input_text(&trie.tests[0].args).split(", arguments").next().unwrap(), r#"operations = ["Trie","insert","search","search","startsWith","insert","search"]"#);
        let codec = b.get("serialize-and-deserialize-binary-tree").unwrap();
        assert_eq!((codec.mode, codec.params[0].ty.clone(), codec.returns.clone()), (Mode::Roundtrip, Ty::TreeNode, Ty::TreeNode));
        let reorder = b.get("reorder-list").unwrap();
        assert_eq!((reorder.returns.clone(), reorder.output_ty()), (Ty::Void, &Ty::ListNode));
        let cycle = b.get("linked-list-cycle").unwrap();
        assert_eq!(cycle.input_text(&cycle.tests[0].args), "head = [3,2,0,-4], pos = 1");
    }

    #[test]
    fn design_tests_are_checked_against_the_methods() {
        let b = repo();
        let trie = b.get("implement-trie-prefix-tree").unwrap();
        let ok = |ops: Value, args: Value, exp: Value| trie.check(&[ops, args], &exp);
        assert!(ok(json!(["Trie", "insert", "search"]), json!([[], ["a"], ["a"]]), json!([null, null, true])).is_ok());
        assert!(ok(json!(["Trie", "remove"]), json!([[], ["a"]]), json!([null, null])).is_err(), "no such method");
        assert!(ok(json!(["Trie", "search"]), json!([[], [1]]), json!([null, true])).is_err(), "wrong argument type");
        assert!(ok(json!(["Trie", "insert"]), json!([[], ["a"]]), json!([null, true])).is_err(), "void returns null");
        assert!(ok(json!(["Nope", "search"]), json!([[], ["a"]]), json!([null, true])).is_err(), "starts with the constructor");
    }

    #[test]
    fn compare_modes() {
        assert!(Compare::AnyOrder.equal(&json!([0, 1]), &json!([1, 0])));
        assert!(!Compare::Exact.equal(&json!([0, 1]), &json!([1, 0])));
        assert!(Compare::NestedAnyOrder.equal(&json!([[-1, -1, 2], [-1, 0, 1]]), &json!([[1, 0, -1], [2, -1, -1]])));
        assert!(!Compare::NestedAnyOrder.equal(&json!([[1, 2]]), &json!([[1, 2], [1, 2]])));
        assert!(Compare::Float.equal(&json!(2.5), &json!(2.500001)));
        assert!(!Compare::Float.equal(&json!(2.5), &json!(2.6)));
        assert!(Compare::Exact.equal(&json!(2), &json!(2.0)), "ints from Java/C++ may print as 2.0");
    }

    #[test]
    fn bad_rows_are_reported_not_fatal() {
        let csv = "id,title,difficulty,function,params,returns,tests\n\
                   a,A,Easy,f,x: int,int,\"[{\"\"args\"\":[1],\"\"expected\"\":1}]\"\n\
                   b,B,Weird,f,x: int,int,[]\n\
                   c,C,Easy,f,x: int,int,\"[{\"\"args\"\":[1,2],\"\"expected\"\":1}]\"\n\
                   d,D,Easy,f,x: Node,int,[]\n";
        let b = Bank::parse(csv.as_bytes());
        assert_eq!(b.questions.len(), 1);
        assert_eq!(b.errors.len(), 3, "{:?}", b.errors);
        assert!(b.questions[0].tests[0].example, "first test becomes the example");
    }

    #[test]
    fn pick_prefers_topics_and_skips_used() {
        let b = repo();
        let mut used = HashSet::new();
        let q = b.pick("Medium", &["graph".into()], "", &used, 1).unwrap();
        assert!(q.topics.iter().any(|t| t == "graph"), "{}", q.id);
        used.insert(q.id.clone());
        let q2 = b.pick("Medium", &["graph".into()], "", &used, 1).unwrap();
        assert_ne!(q2.id, q.id);
        assert_eq!(q2.difficulty, "Medium");
        assert!(b.pick("Impossible", &[], "", &used, 1).is_none());
    }
}
