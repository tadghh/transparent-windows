use crate::{
    TransparencyRule,
    platform::{WindowHandle, WindowInfo, WindowManager, convert_to_full, convert_to_human, wm},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowConfig {
    #[serde(default)]
    process_name: String,
    #[serde(default)]
    window_class: String,
    #[serde(rename = "transparency", default)]
    alpha: u8,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    force: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_class: Option<String>,
}

impl WindowConfig {
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

    pub fn get_name(&self) -> String {
        self.process_name.clone()
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

    pub fn reset_config(&self) {
        let wm = wm();
        for handle in self.get_window_hwnds(wm) {
            _ = wm.set_window_alpha(handle, 255);
        }
    }

    pub fn refresh_config(&self) {
        let wm = wm();
        let alpha = self.get_alpha();
        for handle in self.get_window_hwnds(wm) {
            _ = wm.set_window_alpha(handle, alpha);
        }
    }

    pub fn unforce_windows_config(&self) {
        let wm = wm();
        for handle in self.get_window_hwnds(wm) {
            _ = wm.set_window_alpha(handle, 255);
        }
    }

    /*
      Returns all the current handles matching this rule's class and process.
    */
    pub fn get_window_hwnds(&self, wm: &dyn WindowManager) -> Vec<WindowHandle> {
        wm.enumerate_windows(&self.process_name, &self.window_class)
    }

    pub fn get_cache_key(&self) -> String {
        self.get_window_class().to_owned()
    }
}

impl Default for WindowConfig {
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

impl From<&WindowConfig> for TransparencyRule {
    fn from(config: &WindowConfig) -> Self {
        TransparencyRule {
            process_name: config.process_name.to_owned().into(),
            window_class: config.window_class.to_owned().into(),
            transparency: convert_to_human(config.alpha).into(),
            enabled: config.enabled,
            force: config.force,
            old_class: config.old_class.to_owned().unwrap_or_default().into(),
        }
    }
}

impl From<TransparencyRule> for WindowConfig {
    fn from(config: TransparencyRule) -> Self {
        WindowConfig {
            process_name: config.process_name.to_owned().into(),
            window_class: config.window_class.to_owned().into(),
            alpha: convert_to_full(config.transparency),
            enabled: config.enabled,
            force: config.force,
            old_class: if config.old_class.is_empty() {
                None
            } else {
                Some(config.old_class.into())
            },
        }
    }
}
