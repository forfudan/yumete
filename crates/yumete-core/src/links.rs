//! 雙鏈 — the links between a project's Markdown files, both ways (2026-10-30).
//!
//! Obsidian's rule, and only that rule: every `.md` file in the project is a
//! node, and `[[name]]` written in one is an edge to the file named. A name is
//! looked for across the whole project, **nearest first** — the file sharing
//! the longest path with the one that wrote the link — so `[[安寧]]` reaches
//! `百科/人物/安寧.md` from any chapter, and `[[雨夜]]` from `卷二/` finds
//! `卷二/雨夜.md` before `卷一/雨夜.md`.
//!
//! Everything here is a function of files and text, as [`crate::wiki`] is:
//! the editor decides when to read, when to re-read one file, and what to
//! draw. The wiki marks *names* in prose; this follows *brackets*. The two
//! meet only where a file is named like an entry — and there the brackets
//! win the colour (decided with the design).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One `[[…]]` as written: where, to what, and the line it stands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The line it is on, from zero.
    pub line: usize,
    /// The page named — `[[第三章|那一夜]]` and `[[第三章#雪]]` both name
    /// `第三章`.
    pub target: String,
    /// The line, trimmed, for a panel to show.
    pub excerpt: String,
    /// Where the brackets stand in `excerpt`, as a byte range.
    pub span: std::ops::Range<usize>,
}

/// Every link in the project, by file.
#[derive(Debug, Clone, Default)]
pub struct Index {
    pub root: PathBuf,
    /// Every node, in path order.
    pub files: Vec<PathBuf>,
    /// The links each file writes, by the index in `files`.
    pub out: Vec<Vec<Link>>,
    /// Which files answer to a page name — the file's stem, and its path
    /// without the suffix (`人物/安寧`), both relative to the root.
    by_name: HashMap<String, Vec<usize>>,
    /// Bumped by every change, so a panel knows its rows are stale.
    pub generation: u64,
}

/// The suffixes a node may have.
pub fn is_node(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "md" || e == "markdown")
}

impl Index {
    /// Build from `(path, text)` pairs — read by the caller, so that the
    /// reading can happen on a thread of its own.
    pub fn build(root: &Path, texts: Vec<(PathBuf, String)>) -> Index {
        let mut texts = texts;
        texts.sort_by(|a, b| a.0.cmp(&b.0));
        texts.dedup_by(|a, b| a.0 == b.0);
        let mut index = Index { root: root.to_path_buf(), ..Index::default() };
        for (path, text) in texts {
            if !is_node(&path) {
                continue;
            }
            index.files.push(path);
            index.out.push(scan(&text));
        }
        index.name_them();
        index.generation = 1;
        index
    }

    fn name_them(&mut self) {
        self.by_name.clear();
        for (i, path) in self.files.iter().enumerate() {
            for name in self.names_of(path) {
                self.by_name.entry(name).or_default().push(i);
            }
        }
    }

    /// What `[[…]]` may call this file: `安寧`, and `人物/安寧` when it sits
    /// in a folder.
    fn names_of(&self, path: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(stem) = path.file_stem() {
            out.push(stem.to_string_lossy().into_owned());
        }
        if let Ok(rel) = path.strip_prefix(&self.root) {
            let bare = rel.with_extension("");
            let spelt = bare.to_string_lossy().replace('\\', "/");
            if !out.contains(&spelt) {
                out.push(spelt);
            }
        }
        out
    }

    /// How `[[…]]` should spell a node from anywhere: its name alone when no
    /// other file shares it, else its path from the root.
    pub fn spelling(&self, node: usize) -> String {
        let path = &self.files[node];
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match self.by_name.get(&stem).map(Vec::len) {
            Some(1) => stem,
            _ => path
                .strip_prefix(&self.root)
                .unwrap_or(path)
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/"),
        }
    }

    /// Which node a path is, if it is one.
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.files.iter().position(|p| p == path).or_else(|| {
            let real = std::fs::canonicalize(path).ok()?;
            self.files.iter().position(|p| std::fs::canonicalize(p).ok() == Some(real.clone()))
        })
    }

    /// One file written again: its links re-read, its name learnt if it is
    /// new. Nothing else moves.
    pub fn replace(&mut self, path: &Path, text: &str) {
        if !is_node(path) {
            return;
        }
        match self.index_of(path) {
            Some(i) => self.out[i] = scan(text),
            None => {
                let at = self.files.partition_point(|p| p.as_path() < path);
                self.files.insert(at, path.to_path_buf());
                self.out.insert(at, scan(text));
                self.name_them();
            }
        }
        self.generation += 1;
    }

    /// A file gone from the disk.
    pub fn forget(&mut self, path: &Path) {
        if let Some(i) = self.index_of(path) {
            self.files.remove(i);
            self.out.remove(i);
            self.name_them();
            self.generation += 1;
        }
    }

    /// **The one resolver.** The file `[[target]]` names, as seen from
    /// `from` — nearest first, then the shorter path. `None` when no file
    /// answers to the name; the caller says 「no such page」.
    pub fn resolve(&self, target: &str, from: &Path) -> Option<&Path> {
        let target = target.trim().trim_end_matches(".md").replace('\\', "/");
        let target = target.trim_start_matches("./");
        if let Some(candidates) = self.by_name.get(target) {
            return nearest(candidates.iter().map(|&i| self.files[i].as_path()), from);
        }
        // `[[人物/安寧]]` from anywhere: a path's tail, as Obsidian reads it.
        if !target.contains('/') {
            return None;
        }
        let tail = format!("/{target}");
        let candidates = self.files.iter().filter(|p| {
            let bare = p.with_extension("").to_string_lossy().replace('\\', "/");
            bare.ends_with(&tail)
        });
        nearest(candidates.map(PathBuf::as_path), from)
    }

    /// The links written **to** this node: `(source node, link)`.
    pub fn inbound(&self, node: usize) -> Vec<(usize, &Link)> {
        let mut out = Vec::new();
        for (src, links) in self.out.iter().enumerate() {
            // A file's link to itself is a bookmark, not a neighbour.
            if src == node {
                continue;
            }
            for link in links {
                if self.resolve(&link.target, &self.files[src]) == Some(self.files[node].as_path()) {
                    out.push((src, link));
                }
            }
        }
        out
    }

    /// The links this node writes that reach a file: `(destination, link)`.
    /// A link to a page that is not there is left out here; [`Self::dangling`]
    /// lists those.
    pub fn outbound(&self, node: usize) -> Vec<(usize, &Link)> {
        self.out[node]
            .iter()
            .filter_map(|link| {
                let path = self.resolve(&link.target, &self.files[node])?;
                let dst = self.index_of(path)?;
                (dst != node).then_some((dst, link))
            })
            .collect()
    }

    /// The links this node writes to pages no file answers to.
    pub fn dangling(&self, node: usize) -> Vec<&Link> {
        self.out[node]
            .iter()
            .filter(|link| self.resolve(&link.target, &self.files[node]).is_none())
            .collect()
    }

    /// Every neighbour of a node, nearest first, each once: in-links,
    /// out-links, and whether it is both.
    pub fn neighbours(&self, node: usize) -> Vec<Neighbour> {
        let mut seen: HashMap<usize, Neighbour> = HashMap::new();
        for (src, _) in self.inbound(node) {
            seen.entry(src).or_insert(Neighbour { node: src, inward: false, outward: false }).inward = true;
        }
        for (dst, _) in self.outbound(node) {
            seen.entry(dst).or_insert(Neighbour { node: dst, inward: false, outward: false }).outward = true;
        }
        let mut out: Vec<Neighbour> = seen.into_values().filter(|n| n.node != node).collect();
        out.sort_by_key(|n| (!(n.inward && n.outward), !n.inward, self.files[n.node].clone()));
        out
    }

    /// How many links touch a node, either way.
    pub fn degree(&self, node: usize) -> usize {
        self.neighbours(node).len()
    }

    /// A page of the whole graph: every node a dot, every link a line, laid
    /// out by a small force simulation of its own — no network, no library.
    pub fn html(&self, title: &str) -> String {
        let name = |i: usize| {
            self.files[i]
                .strip_prefix(&self.root)
                .unwrap_or(&self.files[i])
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/")
        };
        let mut nodes = String::new();
        for i in 0..self.files.len() {
            nodes.push_str(&format!("{{\"id\":{i},\"name\":{}}},", json_string(&name(i))));
        }
        let mut edges = String::new();
        for src in 0..self.files.len() {
            for (dst, _) in self.outbound(src) {
                edges.push_str(&format!("[{src},{dst}],"));
            }
        }
        GRAPH_PAGE
            .replace("/*TITLE*/", &json_string(title))
            .replace("/*NODES*/", nodes.trim_end_matches(','))
            .replace("/*EDGES*/", edges.trim_end_matches(','))
    }
}

/// The two faces of the panel: sections of links, or the file among its
/// neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    #[default]
    List,
    Graph,
}

/// One line of another file that writes a file's name without brackets:
/// `(node, line, the line trimmed, where the name is in it)`.
pub type Mention = (usize, usize, String, std::ops::Range<usize>);

/// One neighbour in the local graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Neighbour {
    pub node: usize,
    /// It links here.
    pub inward: bool,
    /// Here links to it.
    pub outward: bool,
}

/// Of several files answering to one name, the one closest to `from`: the
/// longest shared path, then the shorter path, then the name.
fn nearest<'a>(candidates: impl Iterator<Item = &'a Path>, from: &Path) -> Option<&'a Path> {
    let here: Vec<_> = from.parent().map(|p| p.components().collect()).unwrap_or_default();
    candidates
        .map(|path| {
            let shared = path
                .components()
                .zip(here.iter())
                .take_while(|(a, b)| a == *b)
                .count();
            (shared, path)
        })
        .max_by(|(a, pa), (b, pb)| {
            a.cmp(b)
                .then_with(|| pb.components().count().cmp(&pa.components().count()))
                .then_with(|| pb.cmp(pa))
        })
        .map(|(_, path)| path)
}

/// Every `[[…]]` in a text, outside code.
pub fn scan(text: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let mut fence: Option<char> = None;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let mark = ['`', '~'].into_iter().find(|&c| trimmed.starts_with(&c.to_string().repeat(3)));
        match (fence, mark) {
            (None, Some(m)) => {
                fence = Some(m);
                continue;
            }
            (Some(f), Some(m)) if f == m => {
                fence = None;
                continue;
            }
            (Some(_), _) => continue,
            _ => {}
        }
        if !line.contains("[[") {
            continue;
        }
        let excerpt = line.trim();
        let shift = line.len() - line.trim_start().len();
        let spans = crate::markdown::spans(line);
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let byte_at = |c: usize| chars.get(c).map_or(line.len(), |&(b, _)| b);
        let mut constructs: Vec<usize> = Vec::new();
        for span in spans.iter().filter(|s| s.kind == crate::markdown::Kind::WikiLink) {
            if constructs.contains(&span.construct) {
                continue;
            }
            constructs.push(span.construct);
            let Some(link) = crate::markdown::link_at(line, span.start) else { continue };
            if !link.wiki || link.target.is_empty() {
                continue;
            }
            let start = spans
                .iter()
                .filter(|s| s.construct == span.construct)
                .map(|s| s.start)
                .min()
                .unwrap_or(span.start);
            let end = spans
                .iter()
                .filter(|s| s.construct == span.construct)
                .map(|s| s.end)
                .max()
                .unwrap_or(span.end);
            let a = byte_at(start).saturating_sub(shift).min(excerpt.len());
            let b = byte_at(end).saturating_sub(shift).min(excerpt.len());
            out.push(Link { line: n, target: link.target, excerpt: excerpt.to_string(), span: a..b });
        }
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The exported page. Plain script: a spring layout over the nodes, drag to
/// move, wheel to zoom, a click lights a node's neighbours. It is one file
/// and reads offline.
const GRAPH_PAGE: &str = r##"<!doctype html>
<html><head><meta charset="utf-8">
<title>Graph</title>
<style>
html,body{margin:0;height:100%;background:#1a1b26;color:#c0caf5;font:14px/1.4 -apple-system,"PingFang SC","Noto Sans CJK SC",sans-serif;overflow:hidden}
canvas{display:block}
#title{position:fixed;left:14px;top:10px;font-size:15px;opacity:.85}
#hint{position:fixed;left:14px;bottom:10px;font-size:12px;opacity:.55}
#search{position:fixed;right:14px;top:10px;background:#24283b;color:#c0caf5;border:1px solid #3b4261;border-radius:6px;padding:5px 9px;font-size:13px;outline:none;width:180px}
</style></head><body>
<canvas id="c"></canvas>
<div id="title"></div>
<input id="search" placeholder="find…">
<div id="hint">drag · wheel zoom · click: neighbours · double-click: release</div>
<script>
const TITLE=/*TITLE*/;
const NODES=[/*NODES*/];
const EDGES=[/*EDGES*/];
document.getElementById('title').textContent=TITLE;document.title=TITLE;
const c=document.getElementById('c'),g=c.getContext('2d');
let W,H,dpr=devicePixelRatio||1;
function size(){W=innerWidth;H=innerHeight;c.width=W*dpr;c.height=H*dpr;c.style.width=W+'px';c.style.height=H+'px';g.setTransform(dpr,0,0,dpr,0,0)}
size();addEventListener('resize',size);
const deg=NODES.map(()=>0);for(const[a,b]of EDGES){deg[a]++;deg[b]++}
const adj=NODES.map(()=>new Set());for(const[a,b]of EDGES){adj[a].add(b);adj[b].add(a)}
const n=NODES.length;
for(let i=0;i<n;i++){const t=i/n*Math.PI*2,r=Math.min(W,H)*0.35*(0.5+0.5*Math.random());NODES[i].x=W/2+Math.cos(t)*r;NODES[i].y=H/2+Math.sin(t)*r;NODES[i].vx=0;NODES[i].vy=0}
let zoom=1,ox=0,oy=0,drag=null,hover=-1,lit=-1,alpha=1,q='';
function step(){
  const k=Math.sqrt(W*H/Math.max(n,1))*0.9;
  for(let i=0;i<n;i++){const a=NODES[i];if(a.pin)continue;let fx=0,fy=0;
    for(let j=0;j<n;j++){if(i===j)continue;const b=NODES[j];let dx=a.x-b.x,dy=a.y-b.y;let d2=dx*dx+dy*dy+0.01;const f=k*k/d2*0.6;fx+=dx*f/Math.sqrt(d2)*8;fy+=dy*f/Math.sqrt(d2)*8}
    fx+=(W/2-a.x)*0.02;fy+=(H/2-a.y)*0.02;a.vx=(a.vx+fx*0.02)*0.6;a.vy=(a.vy+fy*0.02)*0.6}
  for(const[i,j]of EDGES){const a=NODES[i],b=NODES[j];const dx=b.x-a.x,dy=b.y-a.y;const d=Math.sqrt(dx*dx+dy*dy)+0.01;const f=(d-k*0.9)*0.02;const ux=dx/d*f,uy=dy/d*f;if(!a.pin){a.vx+=ux;a.vy+=uy}if(!b.pin){b.vx-=ux;b.vy-=uy}}
  for(const a of NODES){if(a.pin)continue;a.x+=a.vx*alpha;a.y+=a.vy*alpha}
  alpha=Math.max(0.05,alpha*0.995);
}
function sx(x){return (x-W/2)*zoom+W/2+ox}function sy(y){return (y-H/2)*zoom+H/2+oy}
function draw(){
  g.clearRect(0,0,W,H);
  const focus=lit>=0?lit:hover;
  g.lineWidth=1;
  for(const[i,j]of EDGES){const on=focus<0||i===focus||j===focus;g.strokeStyle=on?(focus<0?'rgba(122,162,247,0.35)':'rgba(122,162,247,0.9)'):'rgba(122,162,247,0.08)';g.beginPath();g.moveTo(sx(NODES[i].x),sy(NODES[i].y));g.lineTo(sx(NODES[j].x),sy(NODES[j].y));g.stroke()}
  for(let i=0;i<n;i++){const a=NODES[i];const r=3+Math.sqrt(deg[i])*2.2;const near=focus<0||i===focus||adj[focus].has(i);const hit=q&&a.name.toLowerCase().includes(q);
    g.fillStyle=hit?'#e0af68':(near?'#7aa2f7':'rgba(122,162,247,0.18)');g.beginPath();g.arc(sx(a.x),sy(a.y),r*Math.sqrt(zoom),0,Math.PI*2);g.fill();
    if(zoom>0.6||near&&focus>=0||hit||deg[i]>=4){g.fillStyle=near||hit?'#c0caf5':'rgba(192,202,245,0.25)';g.font=(i===focus?'bold ':'')+'12px sans-serif';g.fillText(a.name,sx(a.x)+r+4,sy(a.y)+4)}}
  requestAnimationFrame(frame);
}
function frame(){step();draw()}
requestAnimationFrame(frame);
function at(e){const x=e.clientX,y=e.clientY;let best=-1,bd=14*14;for(let i=0;i<n;i++){const dx=sx(NODES[i].x)-x,dy=sy(NODES[i].y)-y;const d=dx*dx+dy*dy;if(d<bd){bd=d;best=i}}return best}
c.addEventListener('mousedown',e=>{const i=at(e);if(i>=0){drag={i,dx:0,dy:0};NODES[i].pin=true}else drag={pan:true,x:e.clientX,y:e.clientY}});
addEventListener('mousemove',e=>{if(drag&&drag.pan){ox+=e.clientX-drag.x;oy+=e.clientY-drag.y;drag.x=e.clientX;drag.y=e.clientY;return}
  if(drag){const a=NODES[drag.i];a.x=(e.clientX-W/2-ox)/zoom+W/2;a.y=(e.clientY-H/2-oy)/zoom+H/2;alpha=Math.max(alpha,0.3);return}hover=at(e);c.style.cursor=hover>=0?'pointer':'default'});
addEventListener('mouseup',()=>{drag=null});
c.addEventListener('click',e=>{const i=at(e);lit=(i===lit)?-1:i});
c.addEventListener('dblclick',e=>{const i=at(e);if(i>=0)NODES[i].pin=false;alpha=1});
c.addEventListener('wheel',e=>{e.preventDefault();const z=Math.exp(-e.deltaY*0.001);zoom=Math.min(6,Math.max(0.2,zoom*z))},{passive:false});
document.getElementById('search').addEventListener('input',e=>{q=e.target.value.trim().toLowerCase()});
</script></body></html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> (PathBuf, Index) {
        static NTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "yumete-links-{}-{}",
            std::process::id(),
            NTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut texts = Vec::new();
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            texts.push((path, text.to_string()));
        }
        let index = Index::build(&dir, texts);
        (dir, index)
    }

    #[test]
    fn links_are_scanned_with_alias_and_anchor_stripped_and_code_skipped() {
        let links = scan("見[[第三章|那一夜]]與[[人物/安寧#幼年]]。\n```\n[[不算]]\n```\n`[[也不算]]` [[算]]\n");
        let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["第三章", "人物/安寧", "算"]);
        assert_eq!(links[0].line, 0);
        assert_eq!(&links[0].excerpt[links[0].span.clone()], "[[第三章|那一夜]]");
        assert_eq!(links[2].line, 4);
    }

    #[test]
    fn a_name_is_found_across_the_project_nearest_first() {
        let (dir, index) = project(&[
            ("卷一/第一章.md", "她去了[[雨夜]]，見[[安寧]]。\n"),
            ("卷一/雨夜.md", "一\n"),
            ("卷二/第四章.md", "再到[[雨夜]]。\n"),
            ("卷二/雨夜.md", "二\n"),
            ("百科/人物/安寧.md", "主角，見[[人物/安寧]]自己與[[第一章]]。\n"),
            ("筆記.txt", "[[不是節點]]\n"),
        ]);
        assert_eq!(index.files.len(), 5, "the .txt is not a node");
        let from = dir.join("卷一/第一章.md");
        assert_eq!(index.resolve("雨夜", &from), Some(dir.join("卷一/雨夜.md").as_path()));
        assert_eq!(index.resolve("雨夜", &dir.join("卷二/第四章.md")), Some(dir.join("卷二/雨夜.md").as_path()));
        assert_eq!(index.resolve("安寧", &from), Some(dir.join("百科/人物/安寧.md").as_path()));
        assert_eq!(index.resolve("人物/安寧", &from), Some(dir.join("百科/人物/安寧.md").as_path()));
        assert_eq!(index.resolve("沒有", &from), None);
        let an = index.index_of(&dir.join("百科/人物/安寧.md")).unwrap();
        let inbound: Vec<&Path> = index.inbound(an).iter().map(|(s, _)| index.files[*s].as_path()).collect();
        assert_eq!(inbound, [dir.join("卷一/第一章.md").as_path()], "a file's link to itself is not inbound");
        let neighbours = index.neighbours(an);
        assert_eq!(neighbours.len(), 1);
        assert!(neighbours[0].inward && neighbours[0].outward, "第一章 links both ways: {neighbours:?}");
        assert_eq!(index.dangling(an).len(), 0);
        let ch1 = index.index_of(&from).unwrap();
        assert_eq!(index.degree(ch1), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_file_is_read_again_and_a_new_one_joins() {
        let (dir, mut index) = project(&[("a.md", "[[b]]\n"), ("b.md", "\n")]);
        let b = index.index_of(&dir.join("b.md")).unwrap();
        assert_eq!(index.inbound(b).len(), 1);
        index.replace(&dir.join("a.md"), "nothing\n");
        assert_eq!(index.inbound(b).len(), 0);
        index.replace(&dir.join("c.md"), "[[b]] [[b]]\n");
        assert_eq!(index.files.len(), 3);
        let b = index.index_of(&dir.join("b.md")).unwrap();
        assert_eq!(index.inbound(b).len(), 2);
        assert_eq!(index.generation, 3);
        index.forget(&dir.join("c.md"));
        assert_eq!(index.files.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_html_page_carries_every_node_and_edge_and_no_script_break() {
        let (dir, index) = project(&[("a.md", "[[b]] [[c]]\n"), ("b.md", "[[a]]\n"), ("c.md", "\n")]);
        let page = index.html("書");
        assert!(page.contains("\"name\":\"a\""));
        assert!(page.contains("[0,1],[0,2],[1,0]"), "{}", &page[page.find("EDGES=").unwrap()..][..40]);
        assert!(page.contains("\"書\""));
        assert!(!json_string("c</script>").contains("</"), "a name cannot close the script");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
