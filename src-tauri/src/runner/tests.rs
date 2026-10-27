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

/// Correct solutions for every new shape: list, cyclic list, list array, in-place list and matrix,
/// tree, graph, design class (bool and double results) and both roundtrips.
const SHAPES: &[(&str, Lang, &str)] = &[
    ("reverse-linked-list", Lang::Java, "class Solution {\n    public ListNode reverseList(ListNode head) {\n        ListNode prev = null;\n        while (head != null) { ListNode n = head.next; head.next = prev; prev = head; head = n; }\n        return prev;\n    }\n}\n"),
    ("reverse-linked-list", Lang::Cpp, "class Solution {\npublic:\n    ListNode* reverseList(ListNode* head) {\n        ListNode* prev = nullptr;\n        while (head) { ListNode* n = head->next; head->next = prev; prev = head; head = n; }\n        return prev;\n    }\n};\n"),
    ("reverse-linked-list", Lang::Js, "var reverseList = function(head) {\n    let prev = null;\n    while (head) { const n = head.next; head.next = prev; prev = head; head = n; }\n    return prev;\n};\n"),
    ("linked-list-cycle", Lang::Java, "class Solution {\n    public boolean hasCycle(ListNode head) {\n        ListNode s = head, f = head;\n        while (f != null && f.next != null) { s = s.next; f = f.next.next; if (s == f) return true; }\n        return false;\n    }\n}\n"),
    ("linked-list-cycle", Lang::Cpp, "class Solution {\npublic:\n    bool hasCycle(ListNode* head) {\n        ListNode *s = head, *f = head;\n        while (f && f->next) { s = s->next; f = f->next->next; if (s == f) return true; }\n        return false;\n    }\n};\n"),
    ("linked-list-cycle", Lang::Js, "var hasCycle = function(head) {\n    let s = head, f = head;\n    while (f && f.next) { s = s.next; f = f.next.next; if (s === f) return true; }\n    return false;\n};\n"),
    ("merge-k-sorted-lists", Lang::Java, "class Solution {\n    public ListNode mergeKLists(ListNode[] lists) {\n        PriorityQueue<ListNode> pq = new PriorityQueue<>((a, b) -> a.val - b.val);\n        for (ListNode l : lists) if (l != null) pq.add(l);\n        ListNode d = new ListNode(), t = d;\n        while (!pq.isEmpty()) { ListNode n = pq.poll(); t.next = n; t = n; if (n.next != null) pq.add(n.next); }\n        return d.next;\n    }\n}\n"),
    ("merge-k-sorted-lists", Lang::Cpp, "class Solution {\npublic:\n    ListNode* mergeKLists(vector<ListNode*>& lists) {\n        auto cmp = [](ListNode* a, ListNode* b) { return a->val > b->val; };\n        priority_queue<ListNode*, vector<ListNode*>, decltype(cmp)> pq(cmp);\n        for (auto l : lists) if (l) pq.push(l);\n        ListNode d; ListNode* t = &d;\n        while (!pq.empty()) { auto n = pq.top(); pq.pop(); t->next = n; t = n; if (n->next) pq.push(n->next); }\n        return d.next;\n    }\n};\n"),
    ("merge-k-sorted-lists", Lang::Js, "var mergeKLists = function(lists) {\n    const vals = [];\n    for (let l of lists) for (; l; l = l.next) vals.push(l.val);\n    vals.sort((a, b) => a - b);\n    const d = new ListNode(); let t = d;\n    for (const v of vals) { t.next = new ListNode(v); t = t.next; }\n    return d.next;\n};\n"),
    ("reorder-list", Lang::Java, "class Solution {\n    public void reorderList(ListNode head) {\n        List<ListNode> a = new ArrayList<>();\n        for (ListNode p = head; p != null; p = p.next) a.add(p);\n        int i = 0, j = a.size() - 1;\n        while (i < j) { a.get(i).next = a.get(j); i++; if (i == j) break; a.get(j).next = a.get(i); j--; }\n        a.get(i).next = null;\n    }\n}\n"),
    ("reorder-list", Lang::Cpp, "class Solution {\npublic:\n    void reorderList(ListNode* head) {\n        vector<ListNode*> a;\n        for (auto p = head; p; p = p->next) a.push_back(p);\n        int i = 0, j = a.size() - 1;\n        while (i < j) { a[i]->next = a[j]; i++; if (i == j) break; a[j]->next = a[i]; j--; }\n        a[i]->next = nullptr;\n    }\n};\n"),
    ("reorder-list", Lang::Js, "var reorderList = function(head) {\n    const a = [];\n    for (let p = head; p; p = p.next) a.push(p);\n    let i = 0, j = a.length - 1;\n    while (i < j) { a[i].next = a[j]; i++; if (i === j) break; a[j].next = a[i]; j--; }\n    a[i].next = null;\n};\n"),
    ("set-matrix-zeroes", Lang::Java, "class Solution {\n    public void setZeroes(int[][] m) {\n        Set<Integer> r = new HashSet<>(), c = new HashSet<>();\n        for (int i = 0; i < m.length; i++) for (int j = 0; j < m[0].length; j++) if (m[i][j] == 0) { r.add(i); c.add(j); }\n        for (int i = 0; i < m.length; i++) for (int j = 0; j < m[0].length; j++) if (r.contains(i) || c.contains(j)) m[i][j] = 0;\n    }\n}\n"),
    ("set-matrix-zeroes", Lang::Cpp, "class Solution {\npublic:\n    void setZeroes(vector<vector<int>>& m) {\n        set<int> r, c;\n        for (int i = 0; i < (int)m.size(); i++) for (int j = 0; j < (int)m[0].size(); j++) if (!m[i][j]) { r.insert(i); c.insert(j); }\n        for (int i = 0; i < (int)m.size(); i++) for (int j = 0; j < (int)m[0].size(); j++) if (r.count(i) || c.count(j)) m[i][j] = 0;\n    }\n};\n"),
    ("set-matrix-zeroes", Lang::Js, "var setZeroes = function(m) {\n    const r = new Set(), c = new Set();\n    m.forEach((row, i) => row.forEach((x, j) => { if (x === 0) { r.add(i); c.add(j); } }));\n    m.forEach((row, i) => row.forEach((_, j) => { if (r.has(i) || c.has(j)) row[j] = 0; }));\n};\n"),
    ("invert-binary-tree", Lang::Java, "class Solution {\n    public TreeNode invertTree(TreeNode root) {\n        if (root == null) return null;\n        TreeNode l = invertTree(root.left); root.left = invertTree(root.right); root.right = l;\n        return root;\n    }\n}\n"),
    ("invert-binary-tree", Lang::Cpp, "class Solution {\npublic:\n    TreeNode* invertTree(TreeNode* root) {\n        if (!root) return nullptr;\n        swap(root->left, root->right); invertTree(root->left); invertTree(root->right);\n        return root;\n    }\n};\n"),
    ("invert-binary-tree", Lang::Js, "var invertTree = function(root) {\n    if (!root) return null;\n    [root.left, root.right] = [invertTree(root.right), invertTree(root.left)];\n    return root;\n};\n"),
    ("maximum-depth-of-binary-tree", Lang::Js, "var maxDepth = function(root) {\n    return root ? 1 + Math.max(maxDepth(root.left), maxDepth(root.right)) : 0;\n};\n"),
    ("maximum-depth-of-binary-tree", Lang::Java, "class Solution {\n    public int maxDepth(TreeNode root) {\n        return root == null ? 0 : 1 + Math.max(maxDepth(root.left), maxDepth(root.right));\n    }\n}\n"),
    ("maximum-depth-of-binary-tree", Lang::Cpp, "class Solution {\npublic:\n    int maxDepth(TreeNode* root) {\n        return root ? 1 + max(maxDepth(root->left), maxDepth(root->right)) : 0;\n    }\n};\n"),
    ("clone-graph", Lang::Java, "class Solution {\n    Map<Node, Node> m = new HashMap<>();\n    public Node cloneGraph(Node node) {\n        if (node == null) return null;\n        if (m.containsKey(node)) return m.get(node);\n        Node c = new Node(node.val); m.put(node, c);\n        for (Node n : node.neighbors) c.neighbors.add(cloneGraph(n));\n        return c;\n    }\n}\n"),
    ("clone-graph", Lang::Cpp, "class Solution {\n    unordered_map<Node*, Node*> m;\npublic:\n    Node* cloneGraph(Node* node) {\n        if (!node) return nullptr;\n        if (m.count(node)) return m[node];\n        Node* c = new Node(node->val); m[node] = c;\n        for (Node* n : node->neighbors) c->neighbors.push_back(cloneGraph(n));\n        return c;\n    }\n};\n"),
    ("clone-graph", Lang::Js, "var cloneGraph = function(node, m = new Map()) {\n    if (!node) return null;\n    if (m.has(node)) return m.get(node);\n    const c = new _Node(node.val); m.set(node, c);\n    for (const n of node.neighbors) c.neighbors.push(cloneGraph(n, m));\n    return c;\n};\n"),
    ("implement-trie-prefix-tree", Lang::Java, "class Trie {\n    Map<String, Boolean> words = new HashMap<>(); Set<String> pre = new HashSet<>();\n    public Trie() {}\n    public void insert(String w) { words.put(w, true); for (int i = 0; i <= w.length(); i++) pre.add(w.substring(0, i)); }\n    public boolean search(String w) { return words.containsKey(w); }\n    public boolean startsWith(String p) { return pre.contains(p); }\n}\n"),
    ("implement-trie-prefix-tree", Lang::Cpp, "class Trie {\n    set<string> words, pre;\npublic:\n    Trie() {}\n    void insert(string w) { words.insert(w); for (size_t i = 0; i <= w.size(); i++) pre.insert(w.substr(0, i)); }\n    bool search(string w) { return words.count(w); }\n    bool startsWith(string p) { return pre.count(p); }\n};\n"),
    ("implement-trie-prefix-tree", Lang::Js, "class Trie {\n    constructor() { this.words = new Set(); this.pre = new Set(); }\n    insert(w) { this.words.add(w); for (let i = 0; i <= w.length; i++) this.pre.add(w.slice(0, i)); }\n    search(w) { return this.words.has(w); }\n    startsWith(p) { return this.pre.has(p); }\n}\n"),
    ("find-median-from-data-stream", Lang::Java, "class MedianFinder {\n    PriorityQueue<Integer> lo = new PriorityQueue<>(Collections.reverseOrder()), hi = new PriorityQueue<>();\n    public MedianFinder() {}\n    public void addNum(int x) { lo.add(x); hi.add(lo.poll()); if (hi.size() > lo.size()) lo.add(hi.poll()); }\n    public double findMedian() { return lo.size() > hi.size() ? lo.peek() : (lo.peek() + hi.peek()) / 2.0; }\n}\n"),
    ("find-median-from-data-stream", Lang::Cpp, "class MedianFinder {\n    priority_queue<int> lo; priority_queue<int, vector<int>, greater<int>> hi;\npublic:\n    MedianFinder() {}\n    void addNum(int x) { lo.push(x); hi.push(lo.top()); lo.pop(); if (hi.size() > lo.size()) { lo.push(hi.top()); hi.pop(); } }\n    double findMedian() { return lo.size() > hi.size() ? lo.top() : (lo.top() + (double)hi.top()) / 2; }\n};\n"),
    ("find-median-from-data-stream", Lang::Js, "var MedianFinder = function() { this.a = []; };\nMedianFinder.prototype.addNum = function(x) {\n    let lo = 0, hi = this.a.length;\n    while (lo < hi) { const m = (lo + hi) >> 1; if (this.a[m] < x) lo = m + 1; else hi = m; }\n    this.a.splice(lo, 0, x);\n};\nMedianFinder.prototype.findMedian = function() {\n    const a = this.a, n = a.length;\n    return n % 2 ? a[n >> 1] : (a[n / 2 - 1] + a[n / 2]) / 2;\n};\n"),
    ("serialize-and-deserialize-binary-tree", Lang::Java, "public class Codec {\n    public String serialize(TreeNode r) { StringBuilder sb = new StringBuilder(); ser(r, sb); return sb.toString(); }\n    void ser(TreeNode r, StringBuilder sb) { if (r == null) { sb.append(\"#,\"); return; } sb.append(r.val).append(','); ser(r.left, sb); ser(r.right, sb); }\n    int i;\n    public TreeNode deserialize(String d) { i = 0; return de(d.split(\",\")); }\n    TreeNode de(String[] t) { String v = t[i++]; if (v.equals(\"#\")) return null; TreeNode n = new TreeNode(Integer.parseInt(v)); n.left = de(t); n.right = de(t); return n; }\n}\n"),
    ("serialize-and-deserialize-binary-tree", Lang::Cpp, "class Codec {\npublic:\n    string serialize(TreeNode* r) { if (!r) return \"#,\"; return to_string(r->val) + \",\" + serialize(r->left) + serialize(r->right); }\n    TreeNode* deserialize(string d) { stringstream ss(d); return de(ss); }\n    TreeNode* de(stringstream& ss) { string v; getline(ss, v, ','); if (v == \"#\") return nullptr; auto n = new TreeNode(stoi(v)); n->left = de(ss); n->right = de(ss); return n; }\n};\n"),
    ("serialize-and-deserialize-binary-tree", Lang::Js, "var serialize = function(root) { return JSON.stringify(root); };\nvar deserialize = function(data) { return JSON.parse(data); };\n"),
    ("encode-and-decode-strings", Lang::Java, "public class Codec {\n    public String encode(List<String> strs) { StringBuilder sb = new StringBuilder(); for (String s : strs) sb.append(s.length()).append('#').append(s); return sb.toString(); }\n    public List<String> decode(String s) { List<String> out = new ArrayList<>(); int i = 0; while (i < s.length()) { int j = s.indexOf('#', i); int n = Integer.parseInt(s.substring(i, j)); out.add(s.substring(j + 1, j + 1 + n)); i = j + 1 + n; } return out; }\n}\n"),
    ("encode-and-decode-strings", Lang::Cpp, "class Codec {\npublic:\n    string encode(vector<string>& strs) { string o; for (auto& s : strs) o += to_string(s.size()) + \"#\" + s; return o; }\n    vector<string> decode(string s) { vector<string> out; size_t i = 0; while (i < s.size()) { size_t j = s.find('#', i); int n = stoi(s.substr(i, j - i)); out.push_back(s.substr(j + 1, n)); i = j + 1 + n; } return out; }\n};\n"),
    ("encode-and-decode-strings", Lang::Js, "var encode = function(strs) { return strs.map(s => s.length + '#' + s).join(''); };\nvar decode = function(s) { const out = []; let i = 0; while (i < s.length) { const j = s.indexOf('#', i); const n = +s.slice(i, j); out.push(s.slice(j + 1, j + 1 + n)); i = j + 1 + n; } return out; };\n"),
];

#[tokio::test]
#[ignore = "slow: compiles every language; run with --ignored"]
async fn new_shapes_run_in_every_language() {
    let b = bank();
    let mut checked = 0;
    for &(id, lang, code) in SHAPES {
        if !have(lang).await {
            continue;
        }
        let q = b.get(id).unwrap();
        let stress = stress_tests(q, 1).await;
        let mut tests: Vec<(&Test, Kind)> = q.tests.iter().map(|t| (t, Kind::Hidden)).collect();
        tests.extend(stress.iter().map(|t| (t, Kind::Stress)));
        let r = run(q, lang, code, &tests).await.unwrap();
        assert_eq!(r.status, Status::Accepted, "{id} {lang:?}: {}", r.summary("Submit"));
        checked += 1;
    }
    // Python: the reference solutions themselves.
    for id in SHAPES.iter().map(|s| s.0).collect::<std::collections::BTreeSet<_>>() {
        if !have(Lang::Python).await {
            break;
        }
        let q = b.get(id).unwrap();
        let stress = stress_tests(q, 1).await;
        assert_eq!(stress.len(), usize::from(!q.stress.is_empty()), "{id}: stress test built");
        let mut tests: Vec<(&Test, Kind)> = q.tests.iter().map(|t| (t, Kind::Hidden)).collect();
        tests.extend(stress.iter().map(|t| (t, Kind::Stress)));
        let r = run(q, Lang::Python, &q.reference, &tests).await.unwrap();
        assert_eq!(r.status, Status::Accepted, "{id} Python: {}", r.summary("Submit"));
        checked += 1;
    }
    eprintln!("{checked} solutions accepted");
}

#[tokio::test]
#[ignore = "slow: compiles every language; run with --ignored"]
async fn returning_the_original_graph_is_caught() {
    let b = bank();
    let q = b.get("clone-graph").unwrap();
    let cheats = [
        (Lang::Java, "class Solution {\n    public Node cloneGraph(Node node) { return node; }\n}\n"),
        (Lang::Cpp, "class Solution {\npublic:\n    Node* cloneGraph(Node* node) { return node; }\n};\n"),
        (Lang::Js, "var cloneGraph = function(node) { return node; };\n"),
        (Lang::Python, "class Solution:\n    def cloneGraph(self, node):\n        return node\n"),
    ];
    for (lang, code) in cheats {
        if !have(lang).await {
            continue;
        }
        let tests: Vec<(&Test, Kind)> = q.examples().map(|t| (t, Kind::Example)).collect();
        let r = run(q, lang, code, &tests).await.unwrap();
        assert_eq!(r.status, Status::RuntimeError, "{lang:?}: {r:#?}");
        assert!(r.cases[0].error.as_deref().unwrap_or("").to_lowercase().contains("not a copy"), "{lang:?}: {r:#?}");
    }
}
