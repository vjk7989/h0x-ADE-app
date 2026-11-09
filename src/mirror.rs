//! Machine-owned layout documents and this screen's private view.
use crate::{
    layout::{Background, Content, Header, HeaderContent, Kind, Node},
    model::{Agent, Model, Recent, Space, Tab},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub spaces: Vec<SharedSpace>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedSpace {
    id: String,
    name: String,
    folder: String,
    tabs: Vec<SharedTab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SharedTab {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default = "empty_root")]
    root: Node,
}

fn empty_root() -> Node {
    Node::container("", crate::layout::Axis::Row, vec![])
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct View {
    active: Option<String>,
    spaces: BTreeMap<String, Option<String>>,
    tabs: BTreeMap<String, TabView>,
    recent: Vec<Recent>,
    attention: BTreeMap<String, Seen>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct TabView {
    focused: Option<String>,
    before: Vec<String>,
    zoomed: Option<String>,
    settled: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Seen {
    word: Option<String>,
    seen: bool,
}

pub fn qualify(id: &str, machine: Option<&str>) -> String {
    machine.map_or_else(|| id.to_owned(), |m| format!("{m}/{id}"))
}

pub fn local<'a>(id: &'a str, machine: Option<&str>) -> &'a str {
    machine.and_then(|m| id.strip_prefix(m).and_then(|s| s.strip_prefix('/'))).unwrap_or(id)
}

fn map_node(node: &mut Node, map: &impl Fn(&str) -> String) {
    node.id = map(&node.id);
    match &mut node.kind {
        Kind::Block { content: Content::Terminal { terminal } } => *terminal = map(terminal),
        Kind::Stack { active: Some(active), .. } => *active = map(active),
        _ => {}
    }
    for float in &mut node.floating {
        float.id = map(&float.id);
        if let Content::Terminal { terminal } = &mut float.content {
            *terminal = map(terminal);
        }
    }
    if let Some(children) = node.children_mut() {
        for child in children {
            map_node(child, map);
        }
    }
}

/// The only mods another machine's layout may place: they never run,
/// exec, call core or send, so a remote document cannot start a program here.
const REMOTE_MODS: &[&str] = &["help"];

fn remote_mod(name: &str) -> bool {
    REMOTE_MODS.contains(&name)
}

/// A Context block shows this machine's own tab, so another machine's layout
/// has no business placing one (inside a tab it would show itself without end).
fn remote_content(content: &Content) -> bool {
    match content {
        Content::Context { .. } => false,
        Content::Mod { name, .. } => remote_mod(name),
        _ => true,
    }
}

/// A header mod from another machine falls back to the built-in header.
fn remote_header(header: &mut Option<Header>) {
    if let Some(Header { content: HeaderContent::Mod { name, .. }, .. }) = header
        && !remote_mod(name)
    {
        *header = None;
    }
}

fn strip_node(node: &mut Node) {
    node.floating.retain(|f| remote_content(&f.content));
    // An image is read from this machine's disk: another machine names none.
    if matches!(node.background, Some(Background::Image { .. })) {
        node.background = None;
    }
    if let Kind::Stack { header, .. } = &mut node.kind {
        remote_header(header);
    }
    if let Some(children) = node.children_mut() {
        children.retain(|c| !matches!(&c.kind, Kind::Block { content } if !remote_content(content)));
        for child in children {
            strip_node(child);
        }
    }
}

impl Document {
    /// Another machine's layout: mod blocks and header mods it placed are
    /// dropped (their props could name a program to run here), and so are
    /// Context blocks and background images (a path on this machine).
    pub fn drop_remote_mods(&mut self) {
        for tab in self.spaces.iter_mut().flat_map(|s| &mut s.tabs) {
            if let Kind::Block { content } = &tab.root.kind
                && !remote_content(content)
            {
                tab.root = Node::container(tab.root.id.clone(), crate::layout::Axis::Row, vec![]);
            }
            strip_node(&mut tab.root);
        }
    }

    pub fn capture(model: &Model, machine: Option<&str>) -> Self {
        let map = |s: &str| local(s, machine).to_owned();
        Self {
            spaces: model
                .spaces
                .iter()
                .filter(|s| s.machine.as_deref() == machine)
                .map(|s| SharedSpace {
                    id: map(&s.id),
                    name: s.name.clone(),
                    folder: s.folder.clone(),
                    tabs: s
                        .tabs
                        .iter()
                        .map(|t| {
                            let mut root = t.root.clone();
                            map_node(&mut root, &map);
                            SharedTab { id: map(&t.id), name: t.name.clone(), root }
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn replace(&self, model: &mut Model, machine: Option<&str>) {
        let view = View::capture(model);
        let at = model.spaces.iter().position(|s| s.machine.as_deref() == machine).unwrap_or(model.spaces.len());
        model.spaces.retain(|s| s.machine.as_deref() != machine);
        let map = |s: &str| qualify(s, machine);
        let spaces = self.spaces.iter().map(|s| Space {
            id: map(&s.id),
            name: s.name.clone(),
            folder: s.folder.clone(),
            machine: machine.map(str::to_owned),
            active: None,
            tabs: s
                .tabs
                .iter()
                .map(|t| {
                    let mut root = t.root.clone();
                    map_node(&mut root, &map);
                    Tab { id: map(&t.id), name: t.name.clone(), root, focused: None, before: vec![], zoomed: None, settled: None }
                })
                .collect(),
        });
        model.spaces.splice(at..at, spaces);
        model.order_spaces();
        view.apply(model);
    }
}

impl View {
    pub fn capture(model: &Model) -> Self {
        Self {
            active: model.active.clone(),
            recent: model.recent.clone(),
            spaces: model.spaces.iter().map(|s| (s.id.clone(), s.active.clone())).collect(),
            tabs: model.tabs().map(|t| (t.id.clone(), TabView { focused: t.focused.clone(), before: t.before.clone(), zoomed: t.zoomed.clone(), settled: t.settled })).collect(),
            attention: model.agents.iter().map(|(a, s)| (a.clone(), Seen { word: s.word.clone(), seen: s.seen })).collect(),
        }
    }

    /// Restore saved choices when a machine's first document arrives, without
    /// resetting choices already made on other machines during startup.
    pub fn apply_machine(&self, model: &mut Model, machine: Option<&str>) {
        let mut merged = Self::capture(model);
        for space in model.spaces.iter().filter(|s| s.machine.as_deref() == machine) {
            if let Some(active) = self.spaces.get(&space.id) {
                merged.spaces.insert(space.id.clone(), active.clone());
            }
            for tab in &space.tabs {
                if let Some(view) = self.tabs.get(&tab.id) {
                    merged.tabs.insert(tab.id.clone(), view.clone());
                }
            }
            if self.active.as_deref() == Some(&space.id) {
                merged.active = self.active.clone();
            }
        }
        for (address, seen) in &self.attention {
            if address.split_once('/').map(|(m, _)| m) == machine {
                merged.attention.insert(address.clone(), seen.clone());
            }
        }
        if machine.is_none() {
            merged.recent = self.recent.clone();
        }
        merged.apply(model);
        for (address, seen) in &self.attention {
            if address.split_once('/').map(|(m, _)| m) == machine
                && let Some(agent) = model.agents.get_mut(address)
                && agent.word == seen.word
            {
                agent.seen = seen.seen;
            }
        }
    }

    pub fn preserve_unloaded(&mut self, saved: &Self, loaded: impl Fn(Option<&str>) -> bool) {
        let unloaded = |id: &str| !loaded(id.split_once('/').map(|(m, _)| m));
        self.spaces.extend(saved.spaces.iter().filter(|(id, _)| unloaded(id)).map(|(id, v)| (id.clone(), v.clone())));
        self.tabs.extend(saved.tabs.iter().filter(|(id, _)| unloaded(id)).map(|(id, v)| (id.clone(), v.clone())));
        self.attention.extend(saved.attention.iter().filter(|(id, _)| unloaded(id)).map(|(id, v)| (id.clone(), v.clone())));
    }

    pub fn apply(&self, model: &mut Model) {
        for s in &mut model.spaces {
            s.active = self.spaces.get(&s.id).cloned().flatten().filter(|id| s.tab(id).is_some()).or_else(|| s.tabs.first().map(|t| t.id.clone()));
            for t in &mut s.tabs {
                if let Some(v) = self.tabs.get(&t.id) {
                    t.focused = v.focused.clone().filter(|id| t.root.find(id).is_some());
                    t.zoomed = v.zoomed.clone().filter(|id| t.root.find(id).is_some());
                    t.before = v.before.iter().filter(|id| t.root.find(id).is_some()).cloned().collect();
                    t.settled = v.settled;
                }
                if t.focused.is_none() {
                    t.focused = t.root.panes().first().map(|n| n.id.clone());
                }
            }
        }
        model.active = self.active.clone().filter(|id| model.space(id).is_some()).or_else(|| model.spaces.first().map(|s| s.id.clone()));
        model.recent = self.recent.clone();
        for (a, s) in &self.attention {
            model.agents.entry(a.clone()).or_insert_with(|| Agent { word: s.word.clone(), seen: s.seen, ..Default::default() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis;
    fn model() -> Model {
        let mut m = Model::default();
        m.add_space("s", "work", "/work", Tab::new("t", "task", "p", "t1"));
        m
    }
    #[test]
    fn focus_zoom_and_settle_are_private() {
        let mut m = model();
        let before = Document::capture(&m, None);
        m.tab_mut("t").unwrap().zoomed = Some("p".into());
        m.settle("t", 12);
        assert_eq!(before, Document::capture(&m, None));
        let view = View::capture(&m);
        let mut other = Model::default();
        before.replace(&mut other, None);
        assert_eq!(other.tab("t").unwrap().zoomed, None);
        view.apply(&mut other);
        assert_eq!(other.tab("t").unwrap().zoomed.as_deref(), Some("p"));
        assert_eq!(other.tab("t").unwrap().settled, Some(12));
    }
    #[test]
    fn remote_roundtrip_qualifies_only_identity_not_mod_props() {
        let mut m = model();
        m.split_with("p", Axis::Row, Node::block("mod", Content::Mod { name: "test".into(), props: serde_json::json!({"terminal":"t1"}) }));
        let doc = Document::capture(&m, None);
        doc.replace(&mut m, Some("mac"));
        assert_eq!(m.address_of("mac/p").as_deref(), Some("mac/t1"));
        assert_eq!(doc, Document::capture(&m, Some("mac")));
        assert!(m.space("s").is_some());
        assert!(m.space("mac/s").is_some());
    }
    #[test]
    fn replacement_preserves_valid_view_and_falls_back_when_deleted() {
        let mut m = model();
        m.split("p", Axis::Row, "p2", "t2");
        let doc = Document::capture(&m, None);
        m.focus_pane("p");
        doc.replace(&mut m, None);
        assert_eq!(m.focused_pane().as_deref(), Some("p"));
        let mut other = model();
        other.remove_pane("p");
        Document::capture(&other, None).replace(&mut m, None);
        assert!(m.active_space().is_none());
    }
    #[test]
    fn late_remote_load_restores_only_that_machines_view() {
        let mut m = model();
        Document::capture(&m, None).replace(&mut m, Some("mac"));
        m.tab_mut("mac/t").unwrap().zoomed = Some("mac/p".into());
        m.active = Some("mac/s".into());
        let saved = View::capture(&m);
        let doc = Document::capture(&m, Some("mac"));
        m.spaces.retain(|s| s.machine.is_none());
        m.active = Some("s".into());
        m.tab_mut("t").unwrap().settled = Some(42);
        doc.replace(&mut m, Some("mac"));
        saved.apply_machine(&mut m, Some("mac"));
        assert_eq!(m.tab("t").unwrap().settled, Some(42));
        assert_eq!(m.tab("mac/t").unwrap().zoomed.as_deref(), Some("mac/p"));
        assert_eq!(m.active.as_deref(), Some("mac/s"));
    }
    #[test]
    fn seen_attention_survives_a_screen_restart_but_not_a_new_word() {
        let mut m = model();
        m.set_word("t1", Some("done".into()), None, false);
        m.agents.get_mut("t1").unwrap().seen = false;
        let saved = View::capture(&m);
        m.agents.clear();
        saved.apply_machine(&mut m, None);
        assert!(!m.agents["t1"].seen);
        m.set_word("t1", Some("working".into()), None, false);
        saved.apply_machine(&mut m, None);
        assert!(m.agents["t1"].seen);
    }
    #[test]
    fn shared_fields_can_be_omitted() {
        let doc: Document = serde_json::from_value(serde_json::json!({"spaces":[{"id":"s","tabs":[{"id":"t"}]}]})).unwrap();
        assert_eq!(doc.spaces[0].name, "");
        assert_eq!(doc.spaces[0].folder, "");
        assert_eq!(doc.spaces[0].tabs[0].name, "");
        assert!(doc.spaces[0].tabs[0].root.children().is_empty());
        assert_eq!(serde_json::from_value::<Document>(serde_json::json!({})).unwrap(), Document::default());
    }
    #[test]
    fn machine_blocks_are_deterministic_and_moves_stay_inside_them() {
        let doc = Document::capture(&model(), None);
        let mut m = Model::default();
        doc.replace(&mut m, Some("z"));
        doc.replace(&mut m, Some("a"));
        doc.replace(&mut m, None);
        assert_eq!(m.spaces.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s", "a/s", "z/s"]);
        m.add_space("s2", "", "", Tab::new("t2", "", "p2", "t2"));
        m.order_spaces();
        assert!(m.move_space("s", usize::MAX));
        assert_eq!(m.spaces.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2", "s", "a/s", "z/s"]);
        assert!(m.move_space("z/s", 1));
        assert_eq!(m.spaces.last().unwrap().id, "z/s");
        doc.replace(&mut m, Some("a"));
        assert_eq!(m.spaces.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2", "s", "a/s", "z/s"]);
    }
    fn layout_with(root: serde_json::Value) -> Document {
        serde_json::from_value(serde_json::json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":root}]}]})).unwrap()
    }
    fn root(doc: &Document) -> &Node {
        &doc.spaces[0].tabs[0].root
    }
    #[test]
    fn remote_layouts_lose_mods_that_could_run_programs() {
        let block = |id: &str, content: serde_json::Value| serde_json::json!({"id":id,"type":"block","content":content});
        let confirm = block("c", serde_json::json!({"mod":"confirm","props":{"command":{"name":"space.new","args":{"machine":"","argv":["sh","-c","id"]}}}}));
        let mut doc = layout_with(serde_json::json!({"id":"r","type":"row",
            "children":[block("p", serde_json::json!({"terminal":"t1"})), confirm.clone(), block("f", serde_json::json!({"mod":"frame-settings"})), block("h", serde_json::json!({"mod":"help"}))],
            "floating":[{"id":"fl","content":{"mod":"worktree","props":{"argv":["evil"]}}}, {"id":"ft","content":{"terminal":"t2"}}]}));
        let local = doc.clone();
        doc.drop_remote_mods();
        let ids = |n: &Node| n.children().iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(root(&doc)), ["p", "h"]);
        assert_eq!(root(&doc).floating.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["ft"]);
        // A local document is imported as it is.
        let mut m = Model::default();
        local.replace(&mut m, None);
        assert_eq!(Document::capture(&m, None), local);
        // A mod as a tab's only pane leaves an empty tab.
        let mut doc = layout_with(confirm);
        doc.drop_remote_mods();
        assert!(matches!(root(&doc).kind, Kind::Row { .. }));
        assert!(root(&doc).children().is_empty());
        assert_eq!(root(&doc).id, "c");
        let mut doc = layout_with(block("f", serde_json::json!({"mod":"frame-consent"})));
        doc.drop_remote_mods();
        assert!(matches!(root(&doc).kind, Kind::Row { .. }));
        assert!(root(&doc).children().is_empty());
    }
    #[test]
    fn remote_header_mods_fall_back_to_the_builtin_header() {
        let mut doc = layout_with(serde_json::json!({"id":"st","type":"stack","children":[{"id":"p","type":"block","content":{"terminal":"t1"}}],
            "header":{"content":{"mod":"frame-palette","props":{"argv":["evil"]}}}}));
        doc.drop_remote_mods();
        assert!(matches!(&root(&doc).kind, Kind::Stack { header: None, .. }));
        assert_eq!(root(&doc).children().len(), 1);
        let mut doc = layout_with(serde_json::json!({"id":"st","type":"stack","children":[],"header":{"content":"tabs"}}));
        let before = doc.clone();
        doc.drop_remote_mods();
        assert_eq!(doc, before);
    }
    #[test]
    fn remote_layouts_lose_background_images_and_context_blocks() {
        let image = |p: &str| serde_json::json!({"image": p});
        let paths = [r"\\evil\share\x.png", "//evil/share/x.png", "/etc/passwd", r"C:\x.png", "../../x.png", "bg/sky.jpg"];
        for path in paths {
            let mut doc = layout_with(serde_json::json!({"id":"r","type":"row","background":image(path),
                "children":[{"id":"p","type":"block","content":{"terminal":"t1"},"background":image(path)},
                            {"id":"c","type":"block","content":{"tab":"active"}}],
                "floating":[{"id":"fc","content":{"tab":"active"}}]}));
            doc.drop_remote_mods();
            assert_eq!(root(&doc).background, None, "{path}");
            assert_eq!(root(&doc).children().len(), 1, "{path}");
            assert_eq!(root(&doc).children()[0].background, None, "{path}");
            assert!(root(&doc).floating.is_empty(), "{path}");
        }
        // Colors stay; a lone Context block (either spelling) leaves an empty tab.
        let mut doc = layout_with(serde_json::json!({"id":"r","type":"row","background":"surface","children":[]}));
        doc.drop_remote_mods();
        assert_eq!(root(&doc).background, Some(Background::Color("surface".into())));
        let mut doc = layout_with(serde_json::json!({"id":"c","type":"block","content":{"context":"active"}}));
        doc.drop_remote_mods();
        assert!(matches!(root(&doc).kind, Kind::Row { .. }));
        // A local document keeps its relative image and Context block.
        let local = layout_with(serde_json::json!({"id":"r","type":"row","background":image("bg/sky.jpg"),"children":[{"id":"c","type":"block","content":{"tab":"active"}}]}));
        let mut m = Model::default();
        local.replace(&mut m, None);
        assert_eq!(Document::capture(&m, None), local);
    }
    #[test]
    fn remote_layouts_lose_images_and_context_blocks_at_any_depth() {
        let image = serde_json::json!({"image": "bg/sky.jpg", "fit": "contain"});
        let gradient = serde_json::json!({"gradient": ["#000000", "surface"], "angle": 90});
        // Containers nested 40 deep (stack, column, row in turn), each with a
        // Context child (with an image pane frame), a Context float and an agent.
        let mut node = serde_json::json!({"id":"leaf","type":"block","content":{"terminal":"t1"},"background":image});
        for depth in 0..40 {
            let kind = ["stack", "column", "row"][depth % 3];
            let paint = if depth % 2 == 0 { image.clone() } else { gradient.clone() };
            node = serde_json::json!({"id": format!("n{depth}"), "type": kind,
                "background": paint,
                "children":[{"id": format!("c{depth}"),"type":"block","content":{"context":"active","pane":{"background":image}}},
                            node,
                            {"id": format!("a{depth}"),"type":"block","content":{"agent":"focused"}}],
                "floating":[{"id": format!("f{depth}"),"content":{"tab":"active"}},{"id": format!("g{depth}"),"content":{"terminal":"t2"}}]});
        }
        let mut doc = layout_with(node);
        doc.drop_remote_mods();
        let mut n = root(&doc);
        for depth in (0..40).rev() {
            assert_eq!(n.id, format!("n{depth}"));
            if depth % 2 == 0 {
                assert_eq!(n.background, None, "image kept at depth {depth}");
            } else {
                assert!(matches!(n.background, Some(Background::Gradient { .. })), "gradient lost at depth {depth}");
            }
            let floats: Vec<&str> = n.floating.iter().map(|f| f.id.as_str()).collect();
            assert_eq!(floats, [format!("g{depth}")]);
            let inner = if depth == 0 { "leaf".to_owned() } else { format!("n{}", depth - 1) };
            let kids: Vec<&str> = n.children().iter().map(|c| c.id.as_str()).collect();
            assert_eq!(kids, [inner, format!("a{depth}")]);
            n = &n.children()[0];
        }
        assert_eq!(n.id, "leaf");
        assert_eq!(n.background, None);
        // Nothing to strip: an empty document and a space without tabs stay as they are.
        let mut empty: Document = serde_json::from_value(serde_json::json!({"spaces":[]})).unwrap();
        let before = empty.clone();
        empty.drop_remote_mods();
        assert_eq!(empty, before);
        let mut bare: Document = serde_json::from_value(serde_json::json!({"spaces":[{"id":"s","tabs":[]}]})).unwrap();
        let before = bare.clone();
        bare.drop_remote_mods();
        assert_eq!(bare, before);
    }
    #[test]
    fn fresh_ids_are_not_a_per_screen_counter() {
        assert_ne!(Model::default().fresh_id("p"), Model::default().fresh_id("p"));
    }
}
