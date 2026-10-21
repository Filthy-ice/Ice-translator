//! 快捷键（FR-2.5、REQ-007/REQ-008）。
//!
//! 映射是纯函数，可无头断言。这里**不消费**按键事件：源文框的 `return_key` 设成 Shift+Enter，
//! 所以裸 Enter 本来就不会插入换行，不需要再从输入队列里摘掉。

use egui::{InputState, Key, Modifiers};

/// 源文输入框的稳定 Id，用来判断"焦点是否在源文里"（`Memory::has_focus`）。
pub const SOURCE_EDITOR_ID: &str = "source_editor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Translate,
    ToggleMode,
    CopyTranslation,
    ToggleTopmost,
    /// Cmd+W / 窗口关闭按钮：常驻后台而不是退出（FR-5.2）
    HideWindow,
    Quit,
    /// Shift+Enter：不提交，换行由 `TextEdit` 自己插入
    Newline,
}

/// 单个按键的分类
pub fn classify(key: Key, modifiers: Modifiers, focus_in_source: bool) -> Option<Action> {
    // 三个全局组合键，不看焦点
    if modifiers.matches_logically(Modifiers::COMMAND | Modifiers::SHIFT) {
        return match key {
            Key::M => Some(Action::ToggleMode),
            Key::C => Some(Action::CopyTranslation),
            Key::P => Some(Action::ToggleTopmost),
            _ => None,
        };
    }

    if modifiers.matches_logically(Modifiers::COMMAND) {
        return match key {
            Key::Enter => Some(Action::Translate),
            Key::W => Some(Action::HideWindow),
            Key::Q => Some(Action::Quit),
            _ => None,
        };
    }

    // 剩下的组合（Alt / Ctrl / 输入法）一律不抢
    if modifiers.alt || modifiers.command || modifiers.ctrl {
        return None;
    }

    match key {
        // Shift 优先：焦点在源文框时 Shift+Enter 也是换行，不是翻译
        Key::Enter if modifiers.shift => Some(Action::Newline),
        // 只有焦点在源文框时裸 Enter 才是"翻译"，否则在设置页填字段按 Enter 会误触发
        Key::Enter if focus_in_source => Some(Action::Translate),
        _ => None,
    }
}

/// 扫描本帧按键事件，按发生顺序返回动作。
///
/// 必须在绘制界面之前调用；按住不放（`repeat`）不重复触发，请求侧本来就有
/// "同一时刻只允许一个在飞"的约束，重复触发只会让加载态闪。
pub fn action_of(input: &InputState, focus_in_source: bool) -> Vec<Action> {
    let mut actions = Vec::new();
    for event in &input.events {
        if let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = event
        {
            if let Some(action) = classify(*key, *modifiers, focus_in_source) {
                actions.push(action);
            }
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const CMD: Modifiers = Modifiers::COMMAND;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CMD_SHIFT: Modifiers = Modifiers {
        command: true,
        shift: true,
        ..Modifiers::NONE
    };

    #[test]
    fn plain_enter_translates_only_when_focus_is_in_source() {
        assert_eq!(classify(Key::Enter, NONE, true), Some(Action::Translate));
        assert_eq!(classify(Key::Enter, NONE, false), None);
    }

    #[test]
    fn cmd_enter_translates_regardless_of_focus() {
        assert_eq!(classify(Key::Enter, CMD, false), Some(Action::Translate));
    }

    #[test]
    fn shift_enter_is_a_newline_even_with_focus_in_source() {
        assert_eq!(classify(Key::Enter, SHIFT, true), Some(Action::Newline));
        assert_eq!(classify(Key::Enter, SHIFT, false), Some(Action::Newline));
    }

    #[test]
    fn global_shortcuts_ignore_focus() {
        assert_eq!(classify(Key::M, CMD_SHIFT, false), Some(Action::ToggleMode));
        assert_eq!(
            classify(Key::C, CMD_SHIFT, false),
            Some(Action::CopyTranslation)
        );
        assert_eq!(
            classify(Key::P, CMD_SHIFT, false),
            Some(Action::ToggleTopmost)
        );
    }

    #[test]
    fn window_shortcuts_map_to_hide_and_quit() {
        assert_eq!(classify(Key::W, CMD, false), Some(Action::HideWindow));
        assert_eq!(classify(Key::Q, CMD, false), Some(Action::Quit));
    }

    #[test]
    fn unbound_keys_pass_through() {
        assert_eq!(classify(Key::A, NONE, true), None);
        // Esc 在全量模式下无绑定（简洁模式的"清空输入"属 P5）
        assert_eq!(classify(Key::Escape, NONE, true), None);
        assert_eq!(classify(Key::M, Modifiers::ALT, true), None);
    }

    /// 无头造一个 `InputState`：塞进 Context 再克隆出来，省得手搓 egui 的帧循环。
    fn input_with(events: Vec<egui::Event>) -> InputState {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.events = events);
        ctx.input(|i| i.clone())
    }

    fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn action_of_collects_keys_in_order_and_ignores_everything_else() {
        let input = input_with(vec![
            key_event(Key::Enter, SHIFT),
            key_event(Key::A, NONE),
            egui::Event::Text("x".into()),
            key_event(Key::M, CMD_SHIFT),
        ]);
        assert_eq!(
            action_of(&input, true),
            vec![Action::Newline, Action::ToggleMode]
        );
    }

    #[test]
    fn held_keys_do_not_repeat_the_action() {
        let input = input_with(vec![
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: NONE,
            },
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: NONE,
            },
        ]);
        assert!(action_of(&input, true).is_empty());
    }

    #[test]
    fn action_of_respects_focus_for_plain_enter() {
        let input = input_with(vec![key_event(Key::Enter, NONE)]);
        assert_eq!(action_of(&input, true), vec![Action::Translate]);
        assert!(action_of(&input, false).is_empty());
    }
}
