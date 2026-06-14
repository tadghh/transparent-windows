//! The KWin JavaScript payloads winalpha injects: the per-frame hover probe and
//! the resident opacity-enforcement script. Kept apart from the Rust logic so
//! the embedded JS is easy to find and edit.

use crate::platform::RuleSpec;

/// Plugin name of the resident opacity-enforcement script (the app id).
pub(super) fn plugin() -> String {
    crate::identity::APP_ID.to_owned()
}

/// Plugin name of the per-frame hover probe.
pub(super) fn hover_plugin() -> String {
    format!("{}-hover", crate::identity::APP_ID)
}

/// Window caption the probe matches to find (and move) our picker panel. Must
/// equal the `title:` set in `ui/hover-info.slint` — pinned by a test below.
pub(super) fn picker_caption() -> String {
    format!("{}-picker", crate::identity::APP_ID)
}

/// Probe script run each frame during a pick: it (1) moves our picker panel to
/// the cursor so it follows the mouse, and (2) reports the top-most window under
/// the cursor (skipping our panel and the desktop) by calling back into our hover
/// D-Bus interface (see [`super::dbus`]). The picker caption and D-Bus identity
/// are filled in by [`build_hover_script`] so they have a single definition.
const HOVER_SCRIPT_TEMPLATE: &str = r#"var p = workspace.cursorPos;
var stack = workspace.stackingOrder || ((typeof workspace.windowList === "function") ? workspace.windowList() : workspace.clientList);
var outClass = "";
var outCaption = "";
var self = null;
for (var i = stack.length - 1; i >= 0; i--) {
    var w = stack[i];
    if (!w) continue;
    if (("" + w.caption).indexOf("__PICKER_CAPTION__") >= 0) { self = w; continue; }
    if (!w.resourceClass) continue;
    if (w.minimized) continue;
    var c = "" + w.resourceClass;
    if (c === "plasmashell") continue;
    if (outClass === "") {
        var g = w.frameGeometry;
        if (g && p.x >= g.x && p.x <= g.x + g.width && p.y >= g.y && p.y <= g.y + g.height) {
            outClass = c;
            outCaption = "" + w.caption;
        }
    }
}
if (self) {
    var sg = self.frameGeometry;
    self.frameGeometry = { x: p.x + 16, y: p.y + 16, width: sg.width, height: sg.height };
}
if (outClass !== "") {
    callDBus("__HOVER_SERVICE__", "__HOVER_OBJECT__", "__HOVER_IFACE__", "__HOVER_METHOD__", outClass, outCaption);
}
"#;

/// Fill the probe template with the picker caption and the hover D-Bus identity,
/// so those strings live in exactly one place (here and [`super::dbus`]) instead
/// of being re-typed as JS literals.
pub(super) fn build_hover_script() -> String {
    use super::dbus;
    HOVER_SCRIPT_TEMPLATE
        .replace("__PICKER_CAPTION__", &picker_caption())
        .replace("__HOVER_SERVICE__", &dbus::hover_service())
        .replace("__HOVER_OBJECT__", dbus::HOVER_OBJECT)
        .replace("__HOVER_IFACE__", &dbus::hover_iface())
        .replace("__HOVER_METHOD__", dbus::HOVER_METHOD)
}

/// Generate the resident KWin script: reset every window to opaque, then apply
/// the per-class opacity rules, and keep applying them to newly opened windows.
pub(super) fn build_script(rules: &[RuleSpec]) -> String {
    let mut array = String::from("[");
    for (i, rule) in rules.iter().enumerate() {
        if i > 0 {
            array.push(',');
        }
        let opacity = (rule.alpha as f64 / 255.0).clamp(0.0, 1.0);
        let class = rule.window_class.replace('\\', "\\\\").replace('"', "\\\"");
        array.push_str(&format!("{{\"cls\":\"{class}\",\"op\":{opacity:.4}}}"));
    }
    array.push(']');

    format!(
        r#"var rules = {array};
function applyTo(w) {{
    if (!w || !w.resourceClass) return;
    var cls = "" + w.resourceClass;
    for (var i = 0; i < rules.length; i++) {{
        if (cls === rules[i].cls) {{ w.opacity = rules[i].op; return; }}
    }}
}}
var list = (typeof workspace.windowList === "function") ? workspace.windowList() : workspace.clientList;
for (var i = 0; i < list.length; i++) {{ list[i].opacity = 1.0; }}
for (var i = 0; i < list.length; i++) {{ applyTo(list[i]); }}
var added = workspace.windowAdded || workspace.clientAdded;
if (added) added.connect(applyTo);
"#
    )
}
