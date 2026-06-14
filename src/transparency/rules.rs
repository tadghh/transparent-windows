use crate::{
    TransparencyRule,
    platform::{
        Opacity, PollingOpacity, WindowHandle, WindowInfo, WindowManager, alpha_to_percent,
        percent_to_alpha,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
// Container-level default: any field missing from the stored JSON is filled from
// `WindowRule::default()` (notably `alpha = 255`, i.e. opaque). Per-field
// `#[serde(default)]` would instead use the field *type's* default — `alpha` 0,
// a fully transparent window — so a hand-edited/partial rule must default to
// opaque here, not invisible.
#[serde(default)]
pub struct WindowRule {
    process_name: String,
    window_class: String,
    #[serde(rename = "transparency")]
    alpha: u8,
    enabled: bool,
    force: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_class: Option<String>,
}

impl WindowRule {
    pub fn new(info: &WindowInfo, alpha: u8) -> Self {
        Self {
            process_name: info.process_name.to_owned(),
            window_class: info.class_name.to_owned(),
            alpha,
            enabled: true,
            force: false,
            old_class: None,
        }
    }

    pub fn get_key(&self) -> String {
        self.process_name.to_owned() + "|" + &self.window_class
    }

    pub fn get_name(&self) -> &str {
        &self.process_name
    }

    pub fn set_name(&mut self, new_process_name: String) {
        self.process_name = new_process_name
    }

    pub fn set_old_classname(&mut self, old_classname: Option<String>) {
        self.old_class = old_classname
    }
    pub fn get_old_classname(&self) -> &Option<String> {
        &self.old_class
    }

    pub fn get_alpha(&self) -> u8 {
        self.alpha
    }

    pub fn set_alpha(&mut self, new_alpha: u8) {
        self.alpha = new_alpha
    }

    pub fn get_window_class(&self) -> &String {
        &self.window_class
    }

    pub fn set_window_class(&mut self, new_class_name: &str) {
        self.window_class = new_class_name.to_owned()
    }

    pub fn set_enabled(&mut self, new_state: bool) {
        self.enabled = new_state
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_forced(&mut self, new_state: bool) {
        self.force = new_state
    }

    pub fn is_forced(&self) -> bool {
        self.force
    }

    pub fn refresh(&self, wm: &dyn WindowManager) {
        // Only polling backends apply per-window alpha; compositor backends
        // enforce opacity through their own rule set, so there's nothing to do.
        let Opacity::Polling(wm) = wm.opacity() else {
            return;
        };
        let alpha = self.get_alpha();
        for handle in self.get_window_handles(wm) {
            _ = wm.set_window_alpha(handle, alpha);
        }
    }

    /// Restore every matching window to fully opaque (used when a rule is
    /// disabled or un-forced).
    pub fn unforce(&self, wm: &dyn WindowManager) {
        let Opacity::Polling(wm) = wm.opacity() else {
            return;
        };
        for handle in self.get_window_handles(wm) {
            _ = wm.set_window_alpha(handle, 255);
        }
    }

    pub fn get_window_handles(&self, wm: &dyn PollingOpacity) -> Vec<WindowHandle> {
        wm.enumerate_windows(&self.process_name, &self.window_class)
    }

    pub fn get_cache_key(&self) -> &str {
        self.window_class.as_str()
    }
}

impl Default for WindowRule {
    fn default() -> Self {
        Self {
            process_name: String::new(),
            window_class: String::new(),
            alpha: 255,
            enabled: false,
            force: false,
            old_class: None,
        }
    }
}

// Bridge between the persisted model (`WindowRule`) and the Slint UI type
// (`TransparencyRule`): alpha is stored 0-255 but shown to the user as a percentage.
impl From<&WindowRule> for TransparencyRule {
    fn from(rule: &WindowRule) -> Self {
        TransparencyRule {
            process_name: rule.process_name.to_owned().into(),
            window_class: rule.window_class.to_owned().into(),
            transparency: alpha_to_percent(rule.alpha).into(),
            enabled: rule.enabled,
            force: rule.force,
            old_class: rule.old_class.to_owned().unwrap_or_default().into(),
        }
    }
}

#[cfg(test)]
#[path = "../tests/rules.rs"]
mod tests;

impl From<TransparencyRule> for WindowRule {
    fn from(rule: TransparencyRule) -> Self {
        WindowRule {
            process_name: rule.process_name.into(),
            window_class: rule.window_class.into(),
            alpha: percent_to_alpha(rule.transparency),
            enabled: rule.enabled,
            force: rule.force,
            old_class: if rule.old_class.is_empty() {
                None
            } else {
                Some(rule.old_class.into())
            },
        }
    }
}
