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
    }
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
    pub function: String,
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
    /// `nums = [2,7,11,15], target = 9`
    pub fn input_text(&self, args: &[Value]) -> String {
        self.params.iter().zip(args).map(|(p, a)| format!("{} = {}", p.name, a)).collect::<Vec<_>>().join(", ")
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

fn parse_tests(s: &str, params: &[Param], returns: &Ty) -> Result<Vec<Test>, String> {
    let raw: Vec<Value> = serde_json::from_str(s).map_err(|e| format!("tests are not valid JSON: {e}"))?;
    let mut out = Vec::new();
    for (i, t) in raw.iter().enumerate() {
        let args = t["args"].as_array().ok_or_else(|| format!("test {} has no args array", i + 1))?;
        if args.len() != params.len() {
            return Err(format!("test {} has {} args, the function takes {}", i + 1, args.len(), params.len()));
        }
        if let Some((p, _)) = params.iter().zip(args).find(|(p, a)| !p.ty.accepts(a)) {
            return Err(format!("test {}: '{}' is not a valid {}", i + 1, p.name, ty_name(&p.ty)));
        }
        if !returns.accepts(&t["expected"]) {
            return Err(format!("test {}: expected is not a valid {}", i + 1, ty_name(returns)));
        }
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
    let params = parse_params(&col("params"))?;
    let returns = Ty::parse(&col("returns"))?;
    let compare = Compare::parse(&col("compare"))?;
    let tests = parse_tests(&col("tests"), &params, &returns)?;
    Ok(Question {
        title: col("title"),
        difficulty,
        levels: split(&col("levels"), ';'),
        topics: split(&col("topics"), ';'),
        companies: split(&col("companies"), ';'),
        function,
        params,
        returns,
        compare,
        statement: col("statement"),
        constraints: split(&col("constraints"), '\n'),
        tests,
        hints: split(&col("hints"), '\n'),
        time: col("time"),
        space: col("space"),
        follow_ups: split(&col("follow_ups"), '\n'),
        reference: col("reference"),
        stress: col("stress"),
        id,
    })
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
        assert!(b.questions.len() >= 20);
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
        assert!(Ty::parse("ListNode").is_err());
        assert!(Ty::Int.accepts(&json!(5)) && !Ty::Int.accepts(&json!(3_000_000_000i64)));
        assert!(Ty::Long.accepts(&json!(3_000_000_000i64)));
        assert!(Ty::Char.accepts(&json!("1")) && !Ty::Char.accepts(&json!("10")));
        assert!(Ty::parse("char[][]").unwrap().accepts(&json!([["1", "0"]])));
        assert!(!Ty::parse("int[]").unwrap().accepts(&json!([1, "2"])));
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
        assert_eq!(q.id, "number-of-islands");
        used.insert(q.id.clone());
        let q2 = b.pick("Medium", &["graph".into()], "", &used, 1).unwrap();
        assert_ne!(q2.id, "number-of-islands");
        assert_eq!(q2.difficulty, "Medium");
        assert!(b.pick("Impossible", &[], "", &used, 1).is_none());
    }
}
