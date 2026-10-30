// Fake Tauri backend for docs screenshots (npm run shots). Sample data only, never shipped.
(function(){
  const D = window.__MOCK__;
  const day = 864e5, now = Date.now();
  const rep = (o, dims) => ({overall:o, decision: o>=75?"Hire":o>=65?"Lean hire":"Lean no hire", level_signal:"Near SDE-2 bar",
    headline:"Clear approach and clean code; edge cases came late.",
    dimensions: dims.map(([n,s]) => ({name:n, score:s, evidence:"Sample evidence."})),
    went_well:["Asked about input size first","Explained the hash map choice"], held_back:["Tested the empty array only when asked"],
    fix_first:"List three edge cases out loud before writing code.", moments:[{question:"What if the array is huge?",said:"Use a bigger map.",better:"Talk memory, streaming, sort + two pointers."}],
    practice_next:["Two pointers","Amortised complexity"], metrics:{answers:9,avg_words:48,longest:130,minutes:44,code_shares:4}});
  const H = [
    ["coding","DSA coding",62,[["Problem solving",3],["Code quality",3],["Testing and edge cases",2],["Complexity analysis",2],["Communication",3]],1],
    ["sysdesign","System design",71,[["Requirements and scoping",3],["Architecture",3],["Deep dive",3],["Scale and reliability",2],["Trade-offs",3],["Driving the discussion",2]],2],
    ["behavioral","Behavioral",78,[["Specificity",3],["Ownership",4],["Self-awareness",3],["Collaboration and conflict",3],["Values alignment",3]],3],
    ["coding","DSA coding",70,[["Problem solving",3],["Code quality",3],["Testing and edge cases",2],["Complexity analysis",3],["Communication",3]],5],
    ["fundamentals","Technical fundamentals",74,[["Accuracy",3],["Depth",3],["Applied reasoning",3],["Communication",3]],6],
    ["sysdesign","System design",66,[["Requirements and scoping",3],["Architecture",3],["Deep dive",2],["Scale and reliability",2],["Trade-offs",3],["Driving the discussion",2]],9],
  ].map(([k,n,o,dims,ago],i) => ({round_id:100-i, loop_id:50-i, round_key:k, name:n, company:i%2?"Google":"Indian fintech (Razorpay, PhonePe, Paytm, Cred…)", minutes:45,
      started_at:new Date(now-ago*day).toISOString(), ended_at:new Date(now-ago*day+44*6e4).toISOString(), overall:o, decision:rep(o,dims).decision, report:rep(o,dims)}));
  const settings = {provider:"claude_code", claude:{Wsl:null}, interviewer_model:"sonnet", report_model:"opus",
    voice:{enabled:true, base_url:"http://127.0.0.1:3900", voice:"", steps:8, speed:1, pause_ms:3000, input_device:"", output_device:""}};
  const R = {
    get_catalog: D.catalog, get_panels: D.panels, get_settings: settings, ai_keys: {}, history: H,
    get_goal: {coding:2, sysdesign:1, behavioral:1},
    voice_status: {up:true, loaded:true, device:"cuda", profiles:[{id:"p1",name:"Neha Kapoor"}], installed:true, english:"Ready", whisper:{state:"ready",model:"large-v3"}, version:"0.5.6"},
    list_dossiers: [D.dossier], research_jobs: [], app_version: "0.1.2", get_backdrop: null, ats_last: null,
    audio_devices: {inputs:[{id:"",name:"Default microphone"}], outputs:[{id:"",name:"Default speaker"}]},
    storage_info: {items:[], total:0},
    start_round: {round_id:7, loop_id:1, index:0, count:3, company:"Razorpay", level:"SDE-2 (2–5 yrs)", lang:"Java", voice:true,
      plan:{key:"coding", name:"Problem solving (DSA)", persona:"Karthik", minutes:45, tab:"editor"},
      problem:{id:"two-sum", title:"Two Sum", difficulty:"Easy", topics:["array","hash-map"],
        statement:"Given an integer array `nums` and an integer `target`, return the **indices** of the two numbers that add up to `target`.\n\nYou may assume each input has exactly one solution, and you may not use the same element twice.",
        constraints:["2 <= nums.length <= 10^4","-10^9 <= nums[i] <= 10^9","Only one valid answer exists."],
        examples:[{input:"nums = [2,7,11,15], target = 9", output:[0,1], explanation:"nums[0] + nums[1] == 9"},{input:"nums = [3,3], target = 6", output:[0,1]}],
        hidden:12, runnable:true, starter:"class Solution {\n    public int[] twoSum(int[] nums, int target) {\n        \n    }\n}"}}, list_models: ["sonnet","opus","fable"], voicestudio_version: {installed:"0.5.6", latest:"0.5.6"},
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd) => { if (cmd in R) return JSON.parse(JSON.stringify(R[cmd])); if (cmd === "ui_log") return null; return null; } },
    event: { listen: async () => () => {} },
  };
})();
