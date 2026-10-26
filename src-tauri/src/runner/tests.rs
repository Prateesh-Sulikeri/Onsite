//! Runs real solutions through the installed toolchains. A language that isn't installed is skipped.

use super::*;
use crate::bank::Bank;
use std::path::Path;

fn bank() -> Bank {
    Bank::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts"))
}

async fn have(lang: Lang) -> bool {
    toolchains().await.iter().any(|t| t.lang == lang && t.version.is_some())
}

async fn run_all(id: &str, lang: Lang, code: &str) -> Report {
    let b = bank();
    let q = b.get(id).unwrap();
    let tests: Vec<(&Test, Kind)> = q.tests.iter().map(|t| (t, if t.example { Kind::Example } else { Kind::Hidden })).collect();
    run(q, lang, code, &tests).await.unwrap()
}

const TWO_SUM: [(Lang, &str); 4] = [
    (
        Lang::Java,
        "class Solution {\n    public int[] twoSum(int[] nums, int target) {\n        Map<Integer, Integer> m = new HashMap<>();\n        for (int i = 0; i < nums.length; i++) {\n            Integer j = m.get(target - nums[i]);\n            if (j != null) return new int[]{j, i};\n            m.put(nums[i], i);\n        }\n        return null;\n    }\n}\n",
    ),
    (
        Lang::Python,
        "class Solution:\n    def twoSum(self, nums: List[int], target: int) -> List[int]:\n        seen = {}\n        for i, x in enumerate(nums):\n            if target - x in seen:\n                return [seen[target - x], i]\n            seen[x] = i\n",
    ),
    (
        Lang::Cpp,
        "class Solution {\npublic:\n    vector<int> twoSum(vector<int>& nums, int target) {\n        unordered_map<int, int> m;\n        for (int i = 0; i < (int)nums.size(); i++) {\n            auto it = m.find(target - nums[i]);\n            if (it != m.end()) return {it->second, i};\n            m[nums[i]] = i;\n        }\n        return {};\n    }\n};\n",
    ),
    (
        Lang::Js,
        "var twoSum = function(nums, target) {\n    const m = new Map();\n    for (let i = 0; i < nums.length; i++) {\n        if (m.has(target - nums[i])) return [m.get(target - nums[i]), i];\n        m.set(nums[i], i);\n    }\n};\n",
    ),
];

#[tokio::test]
async fn correct_two_sum_is_accepted_in_every_language() {
    for (lang, code) in TWO_SUM {
        if !have(lang).await {
            eprintln!("skipping {lang:?}: not installed");
            continue;
        }
        let r = run_all("two-sum", lang, code).await;
        assert_eq!(r.status, Status::Accepted, "{lang:?}: {:#?}", r);
        assert_eq!(r.passed, r.total);
    }
}

#[tokio::test]
#[ignore = "slow: compiles every language; run with --ignored"]
async fn starter_code_compiles_and_fails_cleanly() {
    let b = bank();
    for lang in Lang::ALL {
        if !have(lang).await {
            continue;
        }
        for q in &b.questions {
            let code = starter(q, lang);
            let tests: Vec<(&Test, Kind)> = q.examples().map(|t| (t, Kind::Example)).collect();
            let r = run(q, lang, &code, &tests).await.unwrap();
            // An empty body is a compile error in Java/C++ (missing return) or a wrong answer.
            assert_ne!(r.status, Status::Accepted, "{} {lang:?}", q.id);
            if let Some(e) = &r.compile_error {
                assert!(
                    e.contains("return") || e.contains("expected an indented block") || e.contains("IndentationError"),
                    "{} {lang:?} unexpected compile error: {e}",
                    q.id
                );
            }
        }
    }
}

#[tokio::test]
async fn wrong_answer_compile_error_runtime_error_and_tle() {
    if have(Lang::Python).await {
        let r = run_all("two-sum", Lang::Python, "class Solution:\n    def twoSum(self, nums, target):\n        print('debug', len(nums))\n        return [0, 1]\n").await;
        assert_eq!(r.status, Status::WrongAnswer);
        let bad = r.cases.iter().find(|c| !c.passed).unwrap();
        assert_eq!(bad.kind, Kind::Example, "example 2 fails first");
        assert!(bad.stdout.contains("debug 3"), "prints stay with their test: {:?}", bad.stdout);
        assert!(r.summary("Run").contains("expected [1,2], got [0,1]"), "{}", r.summary("Run"));

        let r = run_all("two-sum", Lang::Python, "class Solution:\n    def twoSum(self, nums, target)\n        return []\n").await;
        assert_eq!(r.status, Status::CompileError);
        assert!(r.compile_error.unwrap().contains("SyntaxError"));

        let r = run_all("two-sum", Lang::Python, "class Solution:\n    def twoSum(self, nums, target):\n        return nums[99]\n").await;
        assert_eq!(r.status, Status::RuntimeError);
        let e = r.cases[0].error.as_deref().unwrap();
        assert!(e.contains("IndexError") && e.contains("line 3"), "{e}");
        assert!(!e.contains("driver.py"), "driver frames hidden: {e}");
    }
    if have(Lang::Java).await {
        let r = run_all("two-sum", Lang::Java, "class Solution {\n    public int[] twoSum(int[] nums, int target) {\n        return nums.lengthh;\n    }\n}\n").await;
        assert_eq!(r.status, Status::CompileError);
        let e = r.compile_error.unwrap();
        assert!(e.contains("Solution.java:3"), "line numbers match the editor: {e}");
        assert!(!e.contains("Temp") && !e.contains("AppData"), "temp path hidden: {e}");

        let r = run_all("two-sum", Lang::Java, "class Solution {\n    public int[] twoSum(int[] nums, int target) {\n        while (true) {}\n    }\n}\n").await;
        assert_eq!(r.status, Status::TimeLimit, "{r:#?}");
    }
    if have(Lang::Cpp).await {
        let r = run_all("two-sum", Lang::Cpp, "class Solution {\npublic:\n    vector<int> twoSum(vector<int>& nums, int target) {\n        int* p = nullptr; return {*p};\n    }\n};\n").await;
        assert_eq!(r.status, Status::RuntimeError, "{r:#?}");
        assert!(r.cases[0].error.as_deref().unwrap().contains("crashed"));
    }
}

#[tokio::test]
#[ignore = "slow: compiles every language; run with --ignored"]
async fn nested_types_and_doubles() {
    let b = bank();
    if have(Lang::Java).await {
        let islands = "class Solution {\n    public int numIslands(char[][] g) {\n        int c = 0;\n        for (int i = 0; i < g.length; i++) for (int j = 0; j < g[0].length; j++) if (g[i][j] == '1') { c++; sink(g, i, j); }\n        return c;\n    }\n    void sink(char[][] g, int i, int j) {\n        if (i < 0 || j < 0 || i >= g.length || j >= g[0].length || g[i][j] != '1') return;\n        g[i][j] = '0'; sink(g, i + 1, j); sink(g, i - 1, j); sink(g, i, j + 1); sink(g, i, j - 1);\n    }\n}\n";
        let q = b.get("number-of-islands").unwrap();
        let stress = stress_tests(q, 1).await;
        let mut tests: Vec<(&Test, Kind)> = q.tests.iter().map(|t| (t, Kind::Hidden)).collect();
        tests.extend(stress.iter().map(|t| (t, Kind::Stress)));
        let r = run(q, Lang::Java, islands, &tests).await.unwrap();
        assert_eq!(r.status, Status::Accepted, "recursive DFS on the snake grid needs the big stack: {r:#?}");

        let groups = "class Solution {\n    public List<List<String>> groupAnagrams(String[] strs) {\n        Map<String, List<String>> m = new HashMap<>();\n        for (String s : strs) { char[] c = s.toCharArray(); Arrays.sort(c); m.computeIfAbsent(new String(c), k -> new ArrayList<>()).add(s); }\n        return new ArrayList<>(m.values());\n    }\n}\n";
        assert_eq!(run_all("group-anagrams", Lang::Java, groups).await.status, Status::Accepted);
    }
    if have(Lang::Cpp).await {
        let median = "class Solution {\npublic:\n    double findMedianSortedArrays(vector<int>& a, vector<int>& b) {\n        vector<int> c(a); c.insert(c.end(), b.begin(), b.end()); sort(c.begin(), c.end());\n        int n = c.size(); return n % 2 ? c[n / 2] : (c[n / 2 - 1] + c[n / 2]) / 2.0;\n    }\n};\n";
        assert_eq!(run_all("median-of-two-sorted-arrays", Lang::Cpp, median).await.status, Status::Accepted);
    }
    if have(Lang::Js).await {
        let three = "var threeSum = function(nums) {\n  nums.sort((a, b) => a - b); const out = [];\n  for (let i = 0; i < nums.length - 2; i++) {\n    if (i && nums[i] === nums[i - 1]) continue;\n    let lo = i + 1, hi = nums.length - 1;\n    while (lo < hi) {\n      const s = nums[i] + nums[lo] + nums[hi];\n      if (s < 0) lo++; else if (s > 0) hi--;\n      else { out.push([nums[i], nums[lo], nums[hi]]); while (lo < hi && nums[lo] === nums[lo + 1]) lo++; lo++; hi--; }\n    }\n  }\n  return out;\n};\n";
        assert_eq!(run_all("3sum", Lang::Js, three).await.status, Status::Accepted);
    }
}

#[tokio::test]
#[ignore = "slow: compiles every language; run with --ignored"]
async fn quadratic_solution_fails_the_stress_test() {
    if !have(Lang::Python).await {
        return;
    }
    let b = bank();
    let q = b.get("daily-temperatures").unwrap();
    let stress = stress_tests(q, 1).await;
    assert_eq!(stress.len(), 1, "generator + reference produce a full-size test");
    assert_eq!(stress[0].args[0].as_array().unwrap().len(), 100_000);
    let slow = "class Solution:\n    def dailyTemperatures(self, t):\n        out = [0] * len(t)\n        for i in range(len(t)):\n            for j in range(i + 1, len(t)):\n                if t[j] > t[i]:\n                    out[i] = j - i\n                    break\n        return out\n";
    let tests: Vec<(&Test, Kind)> = stress.iter().map(|t| (t, Kind::Stress)).collect();
    let r = run(q, Lang::Python, slow, &tests).await.unwrap();
    assert_eq!(r.status, Status::TimeLimit, "{}", r.summary("Submit"));
}

#[tokio::test]
#[ignore = "slow: compiles every language; run with --ignored"]
async fn bank_tests_agree_with_reference_solutions() {
    if !have(Lang::Python).await {
        return;
    }
    let b = bank();
    for q in &b.questions {
        let tests: Vec<(&Test, Kind)> = q.tests.iter().map(|t| (t, Kind::Hidden)).collect();
        let r = run(q, Lang::Python, &q.reference, &tests).await.unwrap();
        assert_eq!(r.status, Status::Accepted, "{}: {}", q.id, r.summary("Check"));
    }
}
