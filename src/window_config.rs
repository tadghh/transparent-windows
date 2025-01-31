use std::path::Path;

use serde::{Deserialize, Serialize};

use windows::core::PCWSTR;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::ProcessStatus::GetProcessImageFileNameA;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::Win32::{
    Foundation::{BOOL, HWND, LPARAM, MAX_PATH},
    UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetParent,
    },
};

use crate::win_utils::{convert_to_full, convert_to_human};
use crate::TransparencyRule;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowConfig {
    #[serde(default)]
    process_name: String,
    #[serde(default)]
    window_class: String,
    #[serde(default)]
    transparency: u8,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    wide_catch: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_class: Option<String>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            process_name: String::new(),
            window_class: String::new(),
            transparency: 255,
            enabled: false,
            wide_catch: false,
            old_class: None,
        }
    }
}

impl From<&WindowConfig> for TransparencyRule {
    fn from(config: &WindowConfig) -> Self {
        TransparencyRule {
            process_name: config.process_name.to_owned().into(),
            window_class: config.window_class.to_owned().into(),
            transparency: convert_to_human(config.transparency).into(),
            enabled: config.enabled,
            wide_catch: config.wide_catch,
            old_class: config.old_class.to_owned().unwrap_or_default().into(),
        }
    }
}

impl From<TransparencyRule> for WindowConfig {
    fn from(config: TransparencyRule) -> Self {
        WindowConfig {
            process_name: config.process_name.to_owned().into(),
            window_class: config.window_class.to_owned().into(),
            transparency: convert_to_full(config.transparency.try_into().unwrap()),
            enabled: config.enabled,
            wide_catch: config.wide_catch,
            old_class: if config.old_class.is_empty() {
                None
            } else {
                Some(config.old_class.into())
            },
        }
    }
}

impl WindowConfig {
    pub fn new(process_name: String, window_class: String, transparency: u8) -> Self {
        Self {
            process_name,
            window_class,
            transparency,
            enabled: true,
            wide_catch: false,
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

    pub fn get_transparency(&self) -> u8 {
        self.transparency
    }

    pub fn set_transparency(&mut self, new_transparency: u8) {
        self.transparency = new_transparency
    }

    pub fn get_window_class(&self) -> &String {
        &self.window_class
    }

    pub fn set_window_class(&mut self, new_class_name: String) {
        self.window_class = new_class_name
    }

    pub fn set_enabled(&mut self, new_state: bool) {
        self.enabled = new_state
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_wide(&mut self, new_state: bool) {
        self.wide_catch = new_state
    }

    pub fn is_wide(&self) -> bool {
        self.wide_catch
    }

    /*
      Returns all the current handles for the classname
    */
    pub fn get_window_hwnds(&self) -> Vec<isize> {
        let wide_class: Vec<u16> = self
            .get_window_class()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let class_ptr = PCWSTR::from_raw(wide_class.as_ptr());
        let mut handles = Vec::new();

        unsafe {
            if let Ok(mut hwnd) = FindWindowW(class_ptr, None) {
                while !hwnd.is_invalid() {
                    let mut process_id = 0;
                    GetWindowThreadProcessId(hwnd, Some(&mut process_id));

                    if let Ok(process_handle) =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
                    {
                        let mut buffer = [0u8; 260];
                        let len = GetProcessImageFileNameA(process_handle, &mut buffer);
                        let _ = CloseHandle(process_handle);

                        if len > 0 {
                            let path_str =
                                String::from_utf8_lossy(&buffer[..len as usize]).to_string();
                            if let Some(name) = Path::new(&path_str)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| s.split('.').next().unwrap_or(s))
                            {
                                if name == self.process_name {
                                    handles.push(std::mem::transmute(hwnd));
                                }
                            }
                        }
                    }
                    hwnd = match FindWindowExW(None, Some(hwnd), class_ptr, None) {
                        Ok(next_hwnd) if !next_hwnd.is_invalid() => next_hwnd,
                        _ => break,
                    };
                }
            }
        }

        handles
    }

    pub fn get_cache_key(&self) -> String {
        self.get_window_class().to_owned()
    }
}

fn get_window_class_name(hwnd: HWND) -> Option<String> {
    let mut class_name = [0u16; MAX_PATH as usize];

    unsafe {
        let length = GetClassNameW(hwnd, &mut class_name);

        if length == 0 {
            return None;
        }

        String::from_utf16_lossy(&class_name[..length as usize])
            .trim_end_matches('\0')
            .to_string()
            .into()
    }
}

pub fn find_parent_from_child_class(
    child_class: &str,
) -> windows::core::Result<Option<(HWND, String)>> {
    let child_hwnd = match find_window_by_class(child_class)? {
        Some(hwnd) => hwnd,
        None => {
            return Ok(None);
        }
    };

    Ok(get_window_class_name(child_hwnd).map(|class_name| (child_hwnd, class_name)))
}

fn find_window_by_class(target_class: &str) -> windows::core::Result<Option<HWND>> {
    struct SearchState<'a> {
        target_class: &'a str,
        found_hwnd: Option<HWND>,
    }

    unsafe extern "system" fn enum_child_windows_proc(child_hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut SearchState);
        if let Some(class_name) = get_window_class_name(child_hwnd) {
            if class_name == state.target_class {
                state.found_hwnd = Some(child_hwnd);

                return false.into();
            }
        }
        true.into()
    }

    unsafe extern "system" fn enum_windows_proc(parent_hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut SearchState);
        let _ = EnumChildWindows(Some(parent_hwnd), Some(enum_child_windows_proc), lparam);

        if state.found_hwnd.is_some() {
            state.found_hwnd = state.found_hwnd;

            false.into()
        } else {
            true.into()
        }
    }

    fn find_topmost_parent(hwnd: HWND) -> Option<HWND> {
        unsafe {
            let current_hwnd = GetParent(hwnd).ok()?;

            Some(current_hwnd)
        }
    }

    let mut state = SearchState {
        target_class,
        found_hwnd: None,
    };

    unsafe {
        _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut state as *mut _ as isize),
        );
    }

    if let Some(found_hwnd) = state.found_hwnd {
        Ok(find_topmost_parent(found_hwnd))
    } else {
        Ok(None)
    }
}
