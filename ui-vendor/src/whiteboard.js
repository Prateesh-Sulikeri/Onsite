// The call room's whiteboard: Excalidraw, plus a text reading of the drawing for the interviewer.
// Exposed as window.OnsiteBoard.mount(parent, {onChange}) -> {text(), reset(), refresh()}.
import { createElement as h } from "react";
import { createRoot } from "react-dom/client";
import { Excalidraw } from "@excalidraw/excalidraw";
import "@excalidraw/excalidraw/index.css";

const SHAPES = new Set(["rectangle", "ellipse", "diamond", "frame", "magicframe", "image"]);
const KIND = { rectangle: "box", ellipse: "ellipse", diamond: "diamond", frame: "frame", magicframe: "frame", image: "image" };
const clean = s => String(s || "").replace(/\s+/g, " ").trim();

function bbox(e) {
  if (e.points && e.points.length) {
    const xs = e.points.map(p => e.x + p[0]), ys = e.points.map(p => e.y + p[1]);
    return { x1: Math.min(...xs), y1: Math.min(...ys), x2: Math.max(...xs), y2: Math.max(...ys) };
  }
  return { x1: e.x, y1: e.y, x2: e.x + e.width, y2: e.y + e.height };
}
const inside = (a, b, pad = 0) => a.x1 >= b.x1 - pad && a.y1 >= b.y1 - pad && a.x2 <= b.x2 + pad && a.y2 <= b.y2 + pad;
const area = b => (b.x2 - b.x1) * (b.y2 - b.y1);

/** Turns the scene into plain lines the interviewer can read: shapes, what they sit inside, arrows, loose text. */
export function describe(elements) {
  const live = elements.filter(e => !e.isDeleted);
  if (!live.length) return "";
  const byId = new Map(live.map(e => [e.id, e]));
  const labelOf = e => {
    if (e.type === "frame" || e.type === "magicframe") return clean(e.name);
    const b = (e.boundElements || []).find(x => x.type === "text");
    const t = b && byId.get(b.id);
    return t ? clean(t.text) : "";
  };
  const read = (a, b) => a.y - b.y || a.x - b.x;
  const shapes = live.filter(e => SHAPES.has(e.type)).sort(read);
  const names = new Map();
  const count = {};
  for (const s of shapes) {
    const l = labelOf(s);
    count[s.type] = (count[s.type] || 0) + 1;
    names.set(s.id, l ? `[${l}]` : `[unlabelled ${KIND[s.type]} ${count[s.type]}]`);
  }
  const boxes = new Map(shapes.map(s => [s.id, bbox(s)]));
  // The smallest other shape that fully holds this one (a service inside a region, a field inside a class).
  const parentOf = s => {
    if (s.frameId && names.has(s.frameId)) return s.frameId;
    const me = boxes.get(s.id);
    let best = null;
    for (const o of shapes) {
      if (o.id === s.id) continue;
      const ob = boxes.get(o.id);
      if (area(ob) > area(me) && inside(me, ob) && (!best || area(ob) < area(boxes.get(best)))) best = o.id;
    }
    return best;
  };
  // An arrow end that isn't glued to a shape still counts if it lands on one.
  const at = (x, y) => {
    let best = null;
    for (const s of shapes) {
      const b = boxes.get(s.id);
      if (inside({ x1: x, y1: y, x2: x, y2: y }, b, 14) && (!best || area(b) < area(boxes.get(best)))) best = s.id;
    }
    return best;
  };

  const lines = [];
  const shapeLines = shapes.map(s => {
    const p = parentOf(s);
    return `- ${names.get(s.id)} (${KIND[s.type]})${p ? ` inside ${names.get(p)}` : ""}`;
  });
  if (shapeLines.length) lines.push("Shapes:", ...shapeLines);

  const links = live.filter(e => e.type === "arrow" || e.type === "line").sort(read).map(a => {
    const pts = a.points || [[0, 0]], first = pts[0], last = pts[pts.length - 1];
    const from = (a.startBinding && byId.has(a.startBinding.elementId) && a.startBinding.elementId) || at(a.x + first[0], a.y + first[1]);
    const to = (a.endBinding && byId.has(a.endBinding.elementId) && a.endBinding.elementId) || at(a.x + last[0], a.y + last[1]);
    const head = a.type === "line" ? "--" : a.startArrowhead && a.endArrowhead ? "<->" : a.startArrowhead ? "<-" : a.endArrowhead ? "->" : "--";
    const label = labelOf(a);
    const end = id => (id && names.get(id)) || "(nothing)";
    return `- ${end(from)} ${head} ${end(to)}${label ? `: ${label}` : ""}`;
  });
  if (links.length) lines.push("Connections:", ...links);

  const loose = live.filter(e => e.type === "text" && !e.containerId).sort(read).map(e => {
    const near = at(e.x + e.width / 2, e.y + e.height / 2);
    return `- "${clean(e.text)}"${near ? ` (on ${names.get(near)})` : ""}`;
  }).filter(l => l !== '- ""');
  if (loose.length) lines.push("Text on the board:", ...loose);

  const sketches = live.filter(e => e.type === "freedraw").length;
  if (sketches) lines.push(`(${sketches} freehand stroke${sketches > 1 ? "s" : ""} not described)`);
  return lines.join("\n");
}

function mount(parent, { onChange } = {}) {
  let api = null, last = "";
  const root = createRoot(parent);
  root.render(h(Excalidraw, {
    theme: "dark",
    aiEnabled: false,
    excalidrawAPI: a => { api = a; },
    initialData: { appState: { viewBackgroundColor: "transparent", currentItemFontFamily: 5 } },
    UIOptions: {
      canvasActions: { loadScene: false, saveToActiveFile: false, toggleTheme: false, export: false,
        changeViewBackgroundColor: false, clearCanvas: true, saveAsImage: true },
      tools: { image: false },
    },
    onChange: elements => {
      const text = describe(elements);
      if (text !== last) { last = text; onChange && onChange(text); }
    },
  }));
  return {
    text: () => (api ? describe(api.getSceneElements()) : last),
    // Empties the board but keeps its settings (resetScene would bring back the white background).
    reset() {
      last = "";
      if (!api) return;
      api.updateScene({ elements: [] });
      api.history && api.history.clear && api.history.clear();
    },
    refresh() { api && api.refresh(); },
  };
}

window.OnsiteBoard = { mount, describe };
